//! Reduced C5: canonical admission must reject atomically, through real owner storage.
#![cfg(feature = "fs-store")]
use sem_lang::{Engine, FsRepository, RuntimeState, SemanticPacket, VocabularyModule, restoration};
use sem_lang_context::ContextStore;
use sem_lang_memory::definitions;
use sem_lang_repository::{HeadPublication, HeadStore, RepositoryRoot, RevisionRef, head_intent};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const TERM: &str = "42000000000000000000000000000001";
const UNKNOWN: &str = "42000000000000000000000000000099";
const HEAD: &str = "test/admission";

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sem-admission-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn image(path: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn visit(base: &Path, p: &Path, out: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(p).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                visit(base, &p, out)
            } else {
                out.insert(p.strip_prefix(base).unwrap().into(), fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(path, path, &mut out);
    out
}
fn fixture(path: &Path) -> Result<(FsRepository, restoration::RestorationInput)> {
    let mut repo = FsRepository::create(path)?;
    let module: VocabularyModule = serde_json::from_value(
        json!({"schema_version":"0.10.0","vocabulary_id":"admission","version":"0.10.0","dependencies":[],
      "terms":[{"id":TERM,"label":"Fixture relation","description":"Explicit primitive","deprecation":null,"replacement":null,"semantic_definition":{"Primitive":"Relation"}}],
      "types":[{"id":TERM,"label":"Fixture type","super_types":[],"description":"fixture","deprecation":null}],
      "roles":[{"id":TERM,"label":"Fixture role","cardinality":"ZeroOrMany","description":"fixture"}],"lexical_bindings":[]}),
    )?;
    let revision = definitions::adopt(
        &mut repo,
        "fixture",
        None,
        &[module],
        "decision:fixture",
        &RevisionRef::default(),
    )?;
    let input = restoration::checkpoint(
        &mut repo,
        &[restoration::RevisionReference {
            namespace: "fixture".into(),
            revision,
        }],
        &[],
        &ContextStore::default(),
        &RuntimeState::new(),
    )?;
    repo.publish_head(&head_intent(
        HEAD,
        "initial",
        &RevisionRef::default(),
        &HeadPublication {
            root: RepositoryRoot::Snapshot(input.runtime_snapshot.clone()),
            request_digest: RuntimeState::new().semantic_state_digest()?,
        },
    )?)?;
    Ok((repo, input))
}
fn packet() -> Value {
    json!({"id":"03000000000000000000000000000001","conversation_id":null,"sender":"not-a-term","recipient":null,"cause":null,"correlation":null,"act":null,"content":{"Value":{"TermRef":TERM}},"context_delta":null,"vocabulary_manifest":null,"resolution":"Resolved","pragmatics":[],"provenance":null})
}
fn request(repo: &FsRepository) -> Result<restoration::CheckpointRequest> {
    Ok(restoration::CheckpointRequest {
        packet: serde_json::from_value(
            json!({"operation_id":"apply","expected_commit_revision":RevisionRef::default(),"expected_semantic_digest":RuntimeState::new().semantic_state_digest()?}),
        )?,
        expected_commit_revision: repo.head_revision(HEAD)?,
    })
}

#[test]
fn restored_entity_index_uses_adopted_definitions_not_checkpoint_dictionary_copies() -> Result {
    use sem_lang::entity_index::{IndexField, IndexRule};
    use sem_lang_core::{Digest, EntityId};
    let scratch = Scratch::new();
    let path = scratch.0.join("repo");
    let (mut repo, input) = fixture(&path)?;
    let mut p = packet();
    p["context_delta"] = json!({"added_claims":[{
        "id":"03000000000000000000000000000002","actor":{"Entity":UNKNOWN},"predicate":TERM,
        "arguments":[{"role":TERM,"value":{"Text":"Same name"}}],
        "constraints":[{"TypeConstraint":{"value":{"Entity":UNKNOWN},"ty":TERM}}],
        "validity":{"valid_from":null,"valid_until":null}
    }],"removed_claims":[],"annotations":{}});
    let request = request(&repo)?;
    let accepted = restoration::apply(
        &mut repo,
        &input,
        &serde_json::from_value(p)?,
        &request,
        HEAD,
    )?;
    drop(repo);
    let restored = Engine::restore(FsRepository::open_existing(&path)?, accepted.restoration)?;
    let state = &restored.runtime().state;
    assert!(state.terms.is_empty());
    assert!(state.types.is_empty());
    let definition = &restored.registry().modules().next().unwrap().terms[0];
    let rules = [IndexRule {
        predicate: definition.id,
        definition_digest: Digest::from_serializable(1, definition)?,
        value_role: serde_json::from_value(json!(TERM))?,
        field: IndexField::Label,
    }];
    let before = image(&path);
    let index = restored.entity_index(&rules, 10)?;
    let entity: EntityId = serde_json::from_value(json!(UNKNOWN))?;
    assert_eq!(index.get(entity)?.labels[0].value, "Same name");
    assert_eq!(index.get(entity)?.types.len(), 1);
    drop(restored);
    assert_eq!(image(&path), before);
    Ok(())
}

#[test]
fn all_packet_reference_positions_reject_without_any_durable_mutation() -> Result {
    let mut invalid = Vec::new();
    for content in [
        json!({"Value":{"TermRef":UNKNOWN}}),
        json!({"Value":{"TypeRef":UNKNOWN}}),
        json!({"Value":{"RoleRef":UNKNOWN}}),
        json!({"Value":{"Map":{"nested":{"Sequence":[{"TermRef":TERM},{"TermRef":UNKNOWN}]}}}}),
        json!({"Value":{"TypedHole":{"id":"hole","expected":UNKNOWN,"surface":null}}}),
        json!({"TypedHole":{"id":"hole","expected":UNKNOWN,"surface":null}}),
        json!({"Binding":{"variable":{"id":"x","expected":UNKNOWN},"value":"Nil"}}),
        json!({"Reference":{"target":{"TermRef":UNKNOWN},"scope":"opaque"}}),
        json!({"Apply":{"operator":UNKNOWN,"arguments":[]}}),
        json!({"Apply":{"operator":TERM,"arguments":[{"role":UNKNOWN,"value":"Nil"}]}}),
    ] {
        let mut p = packet();
        p["content"] = content;
        invalid.push(p);
    }
    let mut p = packet();
    p["act"] = json!(UNKNOWN);
    invalid.push(p);
    let mut p = packet();
    p["pragmatics"] = json!([{"TypeRef":UNKNOWN}]);
    invalid.push(p);
    let claim = json!({"id":"03000000000000000000000000000002","actor":null,"predicate":TERM,"arguments":[],"constraints":[],"validity":{"valid_from":null,"valid_until":null}});
    for slot in ["added_claims", "removed_claims"] {
        for field in ["predicate", "actor", "arguments", "constraints"] {
            let mut c = claim.clone();
            c[field] = match field {
                "predicate" => json!(UNKNOWN),
                "actor" => json!({"RoleRef":UNKNOWN}),
                "arguments" => json!([{"role":null,"value":{"TypeRef":UNKNOWN}}]),
                _ => {
                    json!([{"ScopeConstraint":{"variable":{"id":"x","expected":null},"type_id":UNKNOWN}}])
                }
            };
            let mut p = packet();
            p["context_delta"] = json!({"added_claims":[],"removed_claims":[],"annotations":{}});
            p["context_delta"][slot] = json!([c]);
            invalid.push(p);
        }
    }
    for (index, p) in invalid.into_iter().enumerate() {
        let scratch = Scratch::new();
        let path = scratch.0.join("repo");
        let (mut repo, input) = fixture(&path)?;
        let request = request(&repo)?;
        let before = image(&path);
        let result = restoration::apply(
            &mut repo,
            &input,
            &serde_json::from_value(p)?,
            &request,
            HEAD,
        );
        assert!(
            result.is_err(),
            "case {index} accepted an undefined semantic reference"
        );
        let error = serde_json::to_value(result.unwrap_err())?;
        assert!(
            error.get("SemanticReference").is_some(),
            "case {index}: {error}"
        );
        assert_eq!(image(&path), before, "case {index}: durable mutation");
        drop(repo);
        let engine = Engine::restore(FsRepository::open_existing(&path)?, input)?;
        assert!(engine.runtime().state.packets.is_empty());
        assert!(engine.runtime().state.claims.is_empty());
        drop(engine);
        assert_eq!(image(&path), before);
    }
    Ok(())
}

#[test]
fn explicit_definition_replay_restart_stale_cas_and_opaque_ids() -> Result {
    let scratch = Scratch::new();
    let path = scratch.0.join("repo");
    let (mut repo, input) = fixture(&path)?;
    let mut p = packet();
    p["pragmatics"] = json!([{"Entity":UNKNOWN},{"Node":UNKNOWN},{"Expr":UNKNOWN},{"Text":UNKNOWN},{"TypeRef":TERM},{"RoleRef":TERM}]);
    p["act"] = json!(TERM);
    p["context_delta"] = json!({"added_claims":[{"id":"03000000000000000000000000000002","actor":{"Entity":UNKNOWN},"predicate":TERM,
        "arguments":[{"role":TERM,"value":{"TypeRef":TERM}}],
        "constraints":[{"TypeConstraint":{"value":{"TermRef":TERM},"ty":TERM}},{"RelationConstraint":{"value":{"RoleRef":TERM},"role":TERM,"target":{"Node":UNKNOWN}}},{"ScopeConstraint":{"variable":{"id":"x","expected":TERM},"type_id":TERM}}],
        "validity":{"valid_from":null,"valid_until":null}}],"removed_claims":[],"annotations":{}});
    let p: SemanticPacket = serde_json::from_value(p)?;
    let request = request(&repo)?;
    let accepted = restoration::apply(&mut repo, &input, &p, &request, HEAD)?;
    let after = image(&path);
    let replay = restoration::apply(&mut repo, &input, &p, &request, HEAD)?;
    assert_eq!(
        serde_json::to_value(&accepted)?,
        serde_json::to_value(&replay)?
    );
    assert_eq!(image(&path), after);
    let mut stale = request.clone();
    stale.packet.operation_id = "stale".into();
    assert!(matches!(
        restoration::apply(&mut repo, &input, &p, &stale, HEAD),
        Err(restoration::RestorationError::Repository(
            sem_lang_repository::RepositoryError::CommitConflict(_)
        ))
    ));
    assert_eq!(image(&path), after);
    drop(repo);
    let engine = Engine::restore(FsRepository::open_existing(&path)?, accepted.restoration)?;
    assert_eq!(engine.runtime().state.packets.get(&p.id), Some(&p));
    let missing = scratch.0.join("missing");
    assert!(FsRepository::open_existing(&missing).is_err());
    assert!(!missing.exists());
    Ok(())
}

#[test]
fn checkpoint_state_cannot_bypass_packet_admission() -> Result {
    let scratch = Scratch::new();
    let path = scratch.0.join("repo");
    let (mut repo, input) = fixture(&path)?;
    let initial = serde_json::to_value(RuntimeState::new())?;
    let mut cases = Vec::new();
    let node = "03000000000000000000000000000002";
    let mut s = initial.clone();
    s["graph"]["nodes"] =
        json!({node:{"id":node,"expr":{"Value":{"TermRef":UNKNOWN}},"created_by":null}});
    cases.push(s);
    let mut s = initial.clone();
    s["graph"]["edges"] = json!([{"source":node,"role":UNKNOWN,"target":{"Entity":UNKNOWN}}]);
    cases.push(s);
    let mut s = initial.clone();
    s["graph"]["edges"] = json!([{"source":node,"role":TERM,"target":{"TypeRef":UNKNOWN}}]);
    cases.push(s);
    let mut s = initial.clone();
    s["terms"] = json!({TERM:{"id":TERM,"name":"opaque","type_id":UNKNOWN,"digest":RuntimeState::new().semantic_state_digest()?,"deprecation":false}});
    cases.push(s);
    let mut s = initial.clone();
    s["types"] = json!({TERM:{"id":TERM,"name":"opaque","super_types":[UNKNOWN],"pure":true}});
    cases.push(s);
    let mut s = initial;
    s["roles"] = json!({TERM:{"id":TERM,"name":"opaque","parent":UNKNOWN,"description":null}});
    cases.push(s);
    for (index, state) in cases.into_iter().enumerate() {
        let before = image(&path);
        let result = restoration::checkpoint(
            &mut repo,
            &input.definitions,
            &[],
            &ContextStore::default(),
            &serde_json::from_value(state)?,
        );
        assert!(
            matches!(
                result,
                Err(restoration::RestorationError::SemanticReference(_))
            ),
            "state case {index}: {result:?}"
        );
        assert_eq!(before, image(&path));
    }
    Ok(())
}
