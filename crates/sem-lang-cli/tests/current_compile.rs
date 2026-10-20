//! One real-process scenario: exact snapshot, structured outcomes, context,
//! restart, stale inspection and no packet/history mutation.
#[path = "../../../test-support/adoption.rs"]
mod adoption_fixture;
use sem_lang::{CompileContext, Engine, SurfaceInput};
use sem_lang_context::{ContextStore, ContextSubject};
use sem_lang_core::{RoleId, RuntimeState, TermId};
use sem_lang_store_fs::FsRepository;
use serde_json::{Value, json};
use std::process::Command;

#[test]
fn current_compile_cli_uses_the_same_revision_bound_frontend_without_writing() {
    let scratch = tempfile::tempdir().unwrap();
    let repo = scratch.path().join("repository");
    let mut repository = FsRepository::create(&repo).unwrap();
    let concept = TermId::new_random(101, 1);
    let role = RoleId::new_random(102, 1);
    let subject = ContextSubject {
        id: "person:test".into(),
        class: "person".into(),
    };
    let association = sem_lang_core::ContextAssociation {
        kind: sem_lang_context::ContextKind::TimeZone,
        semantic_ref: concept,
        role,
        scope: sem_lang_context::ContextScope::CurrentRequest,
        validity: sem_lang_context::ContextValidity::default(),
    };
    let mut memory = ContextStore {
        active_timezone: Some(sem_lang_context::ContextBinding {
            subject: subject.clone(),
            kind: association.kind,
            value: "Asia/Tokyo".into(),
            authority: sem_lang_context::ContextAuthority::ActiveContext,
            source: sem_lang_context::ContextSource::ExplicitUserStatement,
            scope: sem_lang_context::ContextScope::CurrentRequest,
            validity: sem_lang_context::ContextValidity::default(),
            sensitivity: 1,
        }),
        ..ContextStore::default()
    };
    memory
        .persistent_defaults
        .push(sem_lang_context::PersistentContextualDefault {
            id: "default:cli".into(),
            subject: subject.clone(),
            kind: association.kind,
            value: "Asia/Tokyo".into(),
            scope: sem_lang_context::ContextScope::PersistentAgentDefault,
            validity: sem_lang_context::ContextValidity::default(),
            evidence: vec![],
            revision: "revision:cli".into(),
            promotion_ref: "decision:cli".into(),
        });
    let module = serde_json::from_value(json!({
        "schema_version":"0.10.0", "vocabulary_id":"current.cli.test",
        "version":"0.10.0", "dependencies":[],
        "terms":[{
            "id":concept, "label":"test-concept", "description":"",
            "deprecation":null, "replacement":null,
            "semantic_definition":{"Composition":{"Contextual":{"association":association,"variable":"opaque-slot"}}}
        }],
        "types":[], "roles":[{"id":role, "label":"target", "cardinality":"ExactlyOne", "description":""}],
        "lexical_bindings":[{
            "language_tag":"en", "surface_form":"this", "normalized_form":"this",
            "contribution_kind":"Denotational", "target_term":concept,
            "grammatical_condition":null, "semantic_type_condition":null,
            "priority":1, "valid_from":90, "valid_until":110,
            "provenance":"test-only"
        }]
    }))
    .unwrap();
    let revision =
        adoption_fixture::adopt(&mut repository, "test", None, &[module], "decision:test").unwrap();
    let restoration = sem_lang::restoration::checkpoint(
        &mut repository,
        &[sem_lang::restoration::RevisionReference {
            namespace: "test".into(),
            revision,
        }],
        &[],
        &memory,
        &RuntimeState::new(),
    )
    .unwrap();
    let mut engine = Engine::restore(repository, restoration.clone()).unwrap();
    engine.context_mut().set_subject(subject.clone());
    let context = CompileContext {
        context_refs: vec![
            sem_lang_context::ContextReference::Default("default:cli".into()),
            sem_lang_context::ContextReference::Active,
        ],
        context_scope: sem_lang_core::ContextScope::CurrentRequest,
        observation_evidence: vec![],
        subject: ContextSubject {
            id: "person:test".into(),
            class: "person".into(),
        },
        evidence: vec![],
        at: Some(100),
        locale: "en".into(),
        semantic_revision: engine
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap(),
    };
    let input = SurfaceInput::new("observation:test", "this");
    let expected = serde_json::to_value(engine.compile_surface(&input, &context)).unwrap();
    assert_eq!(expected["Ok"]["kind"], "Resolved");
    assert_eq!(expected["Ok"]["result"]["concept"], json!(concept));
    assert_eq!(
        expected["Ok"]["result"]["bindings"][0]["variable"]["id"],
        "opaque-slot"
    );
    assert_eq!(
        expected["Ok"]["result"]["bindings"][0]["value"],
        json!({"Text":"Asia/Tokyo"})
    );
    assert_eq!(
        expected["Ok"]["result"]["context_refs"],
        json!([["Active","Applicable"],[{"Default":"default:cli"},"Applicable"]])
    );
    drop(engine);
    let restore_file = scratch.path().join("restoration.json");
    let context_file = scratch.path().join("context.json");
    std::fs::write(&restore_file, serde_json::to_vec(&restoration).unwrap()).unwrap();
    let run = |context: &CompileContext| {
        std::fs::write(&context_file, serde_json::to_vec(context).unwrap()).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_sem-lang-cli"))
            .args(["--format", "json", "compile", "--repo"])
            .arg(&repo)
            .arg("--restoration")
            .arg(&restore_file)
            .arg("--context")
            .arg(&context_file)
            .args(["--observation-ref", "observation:test", "--text", "this"])
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&output.stderr)));
        (output.status.success(), value)
    };
    for _ in 0..2 {
        assert_eq!(run(&context), (true, expected.clone()));
    }
    // Invalid external requests cannot silently choose one input or old context.
    for extra in [
        vec![],
        vec!["--text", "this", "--stdin"],
        vec!["--locale", "en"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_sem-lang-cli"))
            .args(["--format", "json", "compile", "--repo"])
            .arg(&repo)
            .arg("--restoration")
            .arg(&restore_file)
            .arg("--context")
            .arg(&context_file)
            .args(["--observation-ref", "observation:test"])
            .args(extra)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    let mut selected = context.clone();
    selected.locale = "ja-JP".into();
    let (success, wrong_locale) = run(&selected);
    assert!(success);
    assert_eq!(wrong_locale["Ok"]["kind"], "NeedsResolution");
    assert_eq!(wrong_locale["Ok"]["result"]["concept"], Value::Null);
    assert_eq!(
        wrong_locale["Ok"]["result"]["issues"][0]["reason"],
        "UnknownExpression"
    );
    selected = context.clone();
    selected.at = Some(110);
    assert_eq!(run(&selected).1["Ok"]["kind"], "NeedsResolution");
    selected = context.clone();
    selected.semantic_revision = "0".repeat(64);
    assert_eq!(run(&selected), (false, json!({"Err":"StaleRevision"})));
    selected.at = None;
    assert_eq!(run(&selected), (false, json!({"Err":"StaleRevision"})));
    selected = context.clone();
    selected.at = None;
    assert_eq!(run(&selected), (false, json!({"Err":"InvalidInput"})));
    let mut restored = Engine::restore(
        FsRepository::open_existing(&repo).unwrap(),
        restoration.clone(),
    )
    .unwrap();
    restored.context_mut().set_subject(subject);
    assert_eq!(
        restored
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap(),
        context.semantic_revision
    );
    assert!(restored.runtime().state.packets.is_empty());
    assert_eq!(
        sem_lang::PacketReducer::commit_revision(&restored.runtime().state)
            .unwrap()
            .sequence,
        0
    );
    assert_eq!(
        serde_json::to_value(restored.compile_surface(&input, &context)).unwrap(),
        expected
    );
    drop(restored);
    // The deleted temporal error reader cannot hide invalid current memory.
    let mut broken = memory.clone();
    broken.active_timezone.as_mut().unwrap().validity = sem_lang_context::ContextValidity {
        valid_from: Some(200),
        valid_until: Some(100),
        superseded_by: None,
    };
    let mut repository = FsRepository::open_existing(&repo).unwrap();
    let invalid = sem_lang::restoration::checkpoint(
        &mut repository,
        &restoration.definitions,
        &restoration.bindings,
        &broken,
        &RuntimeState::new(),
    )
    .unwrap();
    let invalid_engine = Engine::restore(repository, invalid.clone()).unwrap();
    let invalid_revision = invalid_engine
        .current_compiler()
        .unwrap()
        .semantic_revision()
        .unwrap();
    drop(invalid_engine);
    std::fs::write(&restore_file, serde_json::to_vec(&invalid).unwrap()).unwrap();
    selected = context.clone();
    selected.semantic_revision = invalid_revision;
    for _ in 0..2 {
        assert_eq!(run(&selected), (false, json!({"Err":"ContractViolation"})));
    }
    let invalid_again =
        Engine::restore(FsRepository::open_existing(&repo).unwrap(), invalid).unwrap();
    assert_eq!(
        invalid_again
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap(),
        selected.semantic_revision
    );
    assert!(invalid_again.context().store.short_term.is_empty());
    assert_eq!(
        sem_lang::PacketReducer::commit_revision(&invalid_again.runtime().state)
            .unwrap()
            .sequence,
        0
    );
}
