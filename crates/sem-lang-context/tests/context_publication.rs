use sem_lang_context::*;
use zixcel_revision::{Conflict, RevisionRef};

fn subject() -> ContextSubject {
    ContextSubject {
        id: "person:1".into(),
        class: "person".into(),
    }
}
fn request(engine: &ContextEngine, id: &str, operation: ContextOperation) -> ContextRequest {
    ContextRequest {
        operation_id: id.into(),
        expected_commit_revision: engine.commit_revision().unwrap(),
        expected_memory_revision: engine.memory_revision().unwrap(),
        subject_ref: subject(),
        scope: ContextScope::Session,
        operation,
    }
}
fn retain() -> ContextOperation {
    ContextOperation::Retain {
        observation: ObservationRequest {
            input: SurfaceInput::new("input:1", "private source"),
            context: CompileContext {
                subject: subject(),
                evidence: vec![],
                at: None,
                locale: "en".into(),
                semantic_revision: "a".repeat(64),
                context_refs: vec![],
                context_scope: ContextScope::Session,
                observation_evidence: vec![],
            },
            retention: SurfaceRetention::KeepForSession,
        },
    }
}
#[test]
fn publication_replay_after_discard_and_invalid_requests_have_no_file_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("context.json");
    ContextStore::create(&path).unwrap();
    let engine = ContextEngine::new(ContextStore::load(&path).unwrap());
    let retained = request(&engine, "retain:1", retain());
    let first = ContextStore::publish(&path, &retained).unwrap();
    assert!(first.changed);
    assert_eq!(first.commit.committed_revision.sequence, 1);
    let engine = ContextEngine::new(ContextStore::load(&path).unwrap());
    let reference = first.observation_ref.clone().unwrap();
    let discard = request(
        &engine,
        "discard:1",
        ContextOperation::Discard {
            reference: reference.clone(),
        },
    );
    let removed = ContextStore::publish(&path, &discard).unwrap();
    assert!(removed.changed);
    assert_eq!(removed.commit.committed_revision.sequence, 2);
    let before = std::fs::read(&path).unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    let engine = ContextEngine::new(ContextStore::load(&path).unwrap());
    let mut invalid = vec![];
    let mut stale = request(&engine, "stale:1", retain());
    stale.expected_commit_revision = RevisionRef::default();
    invalid.push(stale);
    let mut semantic = request(&engine, "semantic:1", retain());
    semantic.expected_memory_revision = "0".repeat(64);
    invalid.push(semantic);
    let mut wrong_subject = request(&engine, "subject:1", retain());
    wrong_subject.subject_ref.id = "wrong".into();
    invalid.push(wrong_subject);
    let mut wrong_scope = request(&engine, "scope:1", retain());
    wrong_scope.scope = ContextScope::Task;
    invalid.push(wrong_scope);
    invalid.push(request(
        &engine,
        "discard:missing",
        ContextOperation::Discard { reference },
    ));
    invalid.push(request(
        &engine,
        "clear:invalid",
        ContextOperation::Clear {
            targets: vec![ContextReference::Default("not-a-short-term-target".into())],
        },
    ));
    let mut reused = retained.clone();
    reused.subject_ref.id = "another".into();
    invalid.push(reused);
    let mut bad_ref = request(&engine, "retain:bad-reference", retain());
    if let ContextOperation::Retain { observation } = &mut bad_ref.operation {
        observation.context.context_refs = vec![ContextReference::Default("missing".into())];
    }
    invalid.push(bad_ref);
    for _ in 0..100 {
        assert_eq!(ContextStore::publish(&path, &retained).unwrap(), first);
        assert_eq!(ContextStore::publish(&path, &discard).unwrap(), removed);
        for bad in &invalid {
            assert!(ContextStore::publish(&path, bad).is_err(), "{bad:?}");
        }
    }
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified
    );
    assert!(ContextStore::load(&path).unwrap().short_term.is_empty());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}
#[test]
fn semantic_no_change_and_aba_are_fenced_only_by_commit_revision() {
    let mut engine = ContextEngine::new(ContextStore::default());
    let bind = request(
        &engine,
        "bind:1",
        ContextOperation::BindTimezone {
            value: "UTC".into(),
        },
    );
    let first = engine.apply(&bind).unwrap();
    let same = request(
        &engine,
        "bind:2",
        ContextOperation::BindTimezone {
            value: "UTC".into(),
        },
    );
    let second = engine.apply(&same).unwrap();
    assert!(!second.changed);
    assert_eq!(first.output_revision, second.output_revision);
    assert_ne!(
        first.commit.committed_revision,
        second.commit.committed_revision
    );
    let mut stale = request(&engine, "bind:stale", same.operation.clone());
    stale.expected_commit_revision = first.commit.committed_revision.clone();
    assert!(matches!(
        engine.apply(&stale),
        Err(ContextError::CommitConflict(Conflict::Stale { .. }))
    ));
    let clear = request(
        &engine,
        "clear:1",
        ContextOperation::Clear {
            targets: vec![ContextReference::Session],
        },
    );
    engine.apply(&clear).unwrap();
    assert!(engine.store.session_timezone.is_none());
    let before = serde_json::to_vec(&engine.store).unwrap();
    assert_eq!(engine.apply(&bind).unwrap(), first);
    assert_eq!(serde_json::to_vec(&engine.store).unwrap(), before);
}
