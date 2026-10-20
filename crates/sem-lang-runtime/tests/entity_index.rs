use sem_lang_core::*;
use sem_lang_runtime::entity_index::{EntityIndex, IndexError, IndexField, IndexRule};
use sem_lang_runtime::{PacketReducer, PacketRequest};
use sem_lang_vocab::{VocabularyModule, VocabularyRegistry};
use std::collections::BTreeMap;

fn entity(n: u64) -> EntityId {
    EntityId::new_random(n, 1)
}
fn ty(n: u64) -> TypeId {
    TypeId::new_random(n, 2)
}
fn term(n: u64) -> TermId {
    TermId::new_random(n, 3)
}
fn role() -> RoleId {
    RoleId::new_random(1, 4)
}
fn claim(n: u64, who: u64, predicate: u64, value: Value) -> Claim {
    Claim {
        id: NodeId::new_random(n, 5),
        actor: Some(Value::Entity(entity(who))),
        predicate: term(predicate),
        arguments: vec![Argument {
            role: Some(role()),
            value,
        }],
        constraints: vec![],
        validity: Validity {
            valid_from: None,
            valid_until: None,
        },
    }
}
fn apply(state: &RuntimeState, n: u64, added: Vec<Claim>, removed: Vec<Claim>) -> RuntimeState {
    let packet = SemanticPacket {
        id: PacketId::new_random(n, 6),
        conversation_id: None,
        sender: None,
        recipient: None,
        cause: None,
        correlation: None,
        act: None,
        content: Expr::Value(Value::Nil),
        context_delta: Some(ContextDelta {
            added_claims: added,
            removed_claims: removed,
            annotations: BTreeMap::new(),
        }),
        vocabulary_manifest: None,
        resolution: Resolution::Resolved,
        pragmatics: vec![],
        provenance: Some(Provenance {
            source: Some(format!("source:{n}")),
            confidence: None,
            observed_at: n as i64,
        }),
    };
    PacketReducer::reduce(
        state,
        &packet,
        &PacketRequest {
            operation_id: format!("index:{n}"),
            expected_commit_revision: PacketReducer::commit_revision(state).unwrap(),
            expected_semantic_digest: state.semantic_state_digest().unwrap(),
        },
    )
    .unwrap()
    .into_state()
}
fn fixture() -> (RuntimeState, VocabularyRegistry, Vec<IndexRule>) {
    let state = RuntimeState::new();
    let mut module: VocabularyModule = serde_json::from_value(serde_json::json!({
        "schema_version":"0.10.0", "vocabulary_id":"entity-test", "version":"0.10.0", "dependencies":[],
        "terms":[], "types":[
            {"id":ty(1),"label":"Person","super_types":[],"description":"person type","deprecation":null},
            {"id":ty(2),"label":"Website","super_types":[],"description":"website type","deprecation":null}],
        "roles":[{"id":role(),"label":"Value","cardinality":"ZeroOrMany","description":"explicit value role"}], "lexical_bindings":[]
    })).unwrap();
    let mut rules = vec![];
    for (n, field) in [
        (1, IndexField::Label),
        (2, IndexField::Identifier),
        (3, IndexField::Relation),
    ] {
        let definition: sem_lang_vocab::TermDefinition = serde_json::from_value(serde_json::json!({
            "id":term(n), "label":"same presentation label", "description":"explicit relation",
            "deprecation":null, "replacement":null, "semantic_definition":{"Primitive":"Relation"}
        })).unwrap();
        let digest = Digest::from_serializable(1, &definition).unwrap();
        module.terms.push(definition);
        rules.push(IndexRule {
            predicate: term(n),
            definition_digest: digest,
            value_role: role(),
            field,
        });
    }
    let mut claims = vec![];
    for (n, t) in [(1, 1), (2, 1), (3, 2)] {
        let mut typed = claim(
            100 + n,
            n,
            1,
            Value::Text(if n == 3 { "Example" } else { "Taro" }.into()),
        );
        typed.constraints.push(Constraint::TypeConstraint {
            value: Value::Entity(entity(n)),
            ty: ty(t),
        });
        claims.push(typed);
    }
    claims.extend([
        claim(4, 1, 1, Value::Text("太郎".into())),
        claim(5, 1, 1, Value::Text("T".into())),
        claim(6, 3, 2, Value::Text("https://example.test".into())),
        claim(7, 3, 2, Value::Text("192.0.2.1".into())),
        claim(8, 1, 3, Value::Entity(entity(2))),
    ]);
    let mut registry = VocabularyRegistry::new();
    registry.register(module).unwrap();
    (apply(&state, 1, claims, vec![]), registry, rules)
}

#[test]
fn person_and_website_share_exact_bounded_index_without_name_authority() {
    let (state, registry, rules) = fixture();
    let before = serde_json::to_vec(&state).unwrap();
    let index = EntityIndex::build(&state, &registry, &rules, 10).unwrap();
    assert_eq!(index.evaluated_at(), 10);
    assert_eq!(
        index.rules_digest(),
        Digest::from_serializable(1, &rules).unwrap()
    );
    assert_eq!(index.search("Taro", 32).unwrap().len(), 2);
    assert_eq!(index.search("太郎", 32).unwrap()[0].entity, entity(1));
    let person = index.get(entity(1)).unwrap();
    assert_eq!(person.labels.len(), 3);
    assert_eq!(person.relations[0].target, entity(2));
    assert_eq!(person.types[0].type_id, ty(1));
    let website = index.get(entity(3)).unwrap();
    assert_eq!(website.types[0].type_id, ty(2));
    assert_eq!(website.identifiers.len(), 2);
    assert_eq!(index.search("192.0.2.1", 1).unwrap()[0].entity, entity(3));
    assert_eq!(
        index.search("Taro", 1).unwrap_err(),
        IndexError::LimitExceeded
    );
    assert_eq!(
        index.get(entity(99)).unwrap_err(),
        IndexError::EntityUnavailable
    );
    assert!(person.labels.iter().all(|v| !v.evidence.is_empty()));
    assert_eq!(serde_json::to_vec(&state).unwrap(), before);
    let mut renamed = state.clone();
    renamed.terms.insert(
        term(1),
        Term {
            id: term(1),
            name: "forged runtime copy".into(),
            type_id: ty(2),
            digest: Digest::from_bytes(1, b"forged"),
            deprecation: true,
        },
    );
    assert_eq!(
        EntityIndex::build(&renamed, &registry, &rules, 10)
            .unwrap()
            .entries(),
        index.entries()
    );
    let mut wrong = rules.clone();
    wrong[0].definition_digest = Digest::from_bytes(1, b"wrong");
    assert_eq!(
        EntityIndex::build(&state, &registry, &wrong, 10).unwrap_err(),
        IndexError::DefinitionUnavailable
    );
}

#[test]
fn canonical_updates_expiry_revision_and_restart_preserve_entity_not_old_facts() {
    let (state, registry, rules) = fixture();
    let first = EntityIndex::build(&state, &registry, &rules, 10).unwrap();
    let old = state.claims[&NodeId::new_random(4, 5)].clone();
    let changed = claim(9, 1, 1, Value::Text("New alias".into()));
    let mut unrelated = claim(10, 1, 99, Value::Text("timezone".into()));
    unrelated.validity.valid_until = Some(20);
    let state = apply(&state, 2, vec![changed, unrelated], vec![old]);
    let second = EntityIndex::build(&state, &registry, &rules, 30).unwrap();
    assert!(second.search("太郎", 32).unwrap().is_empty());
    assert_eq!(second.search("New alias", 32).unwrap()[0].entity, entity(1));
    assert_eq!(
        second.get(entity(1)).unwrap().types,
        first.get(entity(1)).unwrap().types
    );
    assert_ne!(second.revision(), first.revision());
    let new_fact = &second
        .get(entity(1))
        .unwrap()
        .labels
        .iter()
        .find(|v| v.value == "New alias")
        .unwrap()
        .evidence[0];
    assert_eq!(new_fact.packet, PacketId::new_random(2, 6));
    assert_eq!(
        new_fact.provenance.as_ref().unwrap().source.as_deref(),
        Some("source:2")
    );
    let restored: RuntimeState =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(
        EntityIndex::build(&restored, &registry, &rules, 30)
            .unwrap()
            .entries(),
        second.entries()
    );
    let mut missing = state.clone();
    missing
        .claims
        .retain(|_, c| c.actor != Some(Value::Entity(entity(1))));
    let index = EntityIndex::build(&missing, &registry, &rules, 30).unwrap();
    assert_eq!(
        index.get(entity(1)).unwrap_err(),
        IndexError::EntityUnavailable
    );
    assert_eq!(index.search("Taro", 32).unwrap()[0].entity, entity(2));
}

#[test]
fn unprovenanced_or_expired_assertions_and_oversized_queries_fail_explicitly() {
    let (mut state, registry, rules) = fixture();
    state.claims.insert(
        NodeId::new_random(90, 5),
        claim(90, 1, 1, Value::Text("invented".into())),
    );
    assert_eq!(
        EntityIndex::build(&state, &registry, &rules, 10).unwrap_err(),
        IndexError::ProvenanceUnavailable
    );
    state.claims.remove(&NodeId::new_random(90, 5));
    let mut expiring = claim(91, 1, 1, Value::Text("temporary".into()));
    expiring.validity.valid_until = Some(20);
    let state = apply(&state, 2, vec![expiring], vec![]);
    assert_eq!(
        EntityIndex::build(&state, &registry, &rules, 10)
            .unwrap()
            .search("temporary", 32)
            .unwrap()
            .len(),
        1
    );
    let index = EntityIndex::build(&state, &registry, &rules, 20).unwrap();
    assert!(index.search("temporary", 32).unwrap().is_empty());
    assert!(index.get(entity(1)).is_ok());
    assert_eq!(
        index.search(&"x".repeat(1025), 32).unwrap_err(),
        IndexError::LimitExceeded
    );
}

#[test]
fn name_and_ip_changes_keep_entity_refs_and_facts_cannot_bypass_byte_bounds() {
    let (state, registry, rules) = fixture();
    let old_name = state.claims[&NodeId::new_random(101, 5)].clone();
    let mut name = old_name.clone();
    name.id = NodeId::new_random(92, 5);
    name.arguments[0].value = Value::Text("Renamed".into());
    let old_ip = state.claims[&NodeId::new_random(7, 5)].clone();
    let ip = claim(93, 3, 2, Value::Text("192.0.2.2".into()));
    let updated = apply(&state, 2, vec![name, ip], vec![old_name, old_ip]);
    let index = EntityIndex::build(&updated, &registry, &rules, 10).unwrap();
    assert_eq!(index.search("Renamed", 32).unwrap()[0].entity, entity(1));
    assert_eq!(index.search("Taro", 32).unwrap()[0].entity, entity(2));
    assert_eq!(index.get(entity(1)).unwrap().types[0].type_id, ty(1));
    assert!(index.search("192.0.2.1", 32).unwrap().is_empty());
    assert_eq!(index.search("192.0.2.2", 32).unwrap()[0].entity, entity(3));
    let oversized = apply(
        &state,
        2,
        vec![claim(94, 1, 1, Value::Text("x".repeat(1025)))],
        vec![],
    );
    assert_eq!(
        EntityIndex::build(&oversized, &registry, &rules, 10).unwrap_err(),
        IndexError::LimitExceeded
    );
    let mut malformed = state.clone();
    malformed
        .packets
        .values_mut()
        .next()
        .unwrap()
        .provenance
        .as_mut()
        .unwrap()
        .source = Some("x".repeat(4 * 1024 * 1024));
    assert_eq!(
        EntityIndex::build(&malformed, &registry, &rules, 10).unwrap_err(),
        IndexError::LimitExceeded
    );
}
