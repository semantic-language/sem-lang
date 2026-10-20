//! Current read failures survive; no alternate temporal compiler can hide them.
#[path = "../../../test-support/context.rs"]
mod context_fixture;
use context_fixture::ContextFixture;
use sem_lang_context::*;

#[test]
fn current_query_failures_preserve_scope_revision_and_no_fallback() {
    let subject = ContextSubject {
        id: "person:test".into(),
        class: "person".into(),
    };
    let mut engine = ContextEngine::new(ContextStore::default()).with_subject(subject.clone());
    engine.bind_explicit_timezone("UTC", ContextScope::Session);
    let refs = [ContextReference::Session];
    engine.store.session_timezone.as_mut().unwrap().validity = ContextValidity {
        valid_from: Some(200),
        valid_until: Some(100),
        superseded_by: None,
    };
    let before = engine.memory_revision().unwrap();
    assert!(matches!(
        engine.read_current(&refs, &subject, ContextScope::CurrentRequest, Some(100)),
        Err(CurrentReadError::InvalidContext)
    ));
    assert_eq!(engine.memory_revision().unwrap(), before);
    engine.store.session_timezone.as_mut().unwrap().validity = ContextValidity {
        valid_from: Some(100),
        valid_until: Some(200),
        superseded_by: None,
    };
    assert!(matches!(
        engine.read_current(&refs, &subject, ContextScope::CurrentRequest, None),
        Err(CurrentReadError::MissingInstant)
    ));
    let valid = engine
        .read_current(&refs, &subject, ContextScope::CurrentRequest, Some(100))
        .unwrap();
    assert_eq!(valid.select(ContextKind::TimeZone, &[]).values, vec!["UTC"]);
    let expired = engine
        .read_current(&refs, &subject, ContextScope::CurrentRequest, Some(200))
        .unwrap();
    assert_eq!(expired.entries[0].status, ContextStatus::Expired);
    assert!(expired.select(ContextKind::TimeZone, &[]).values.is_empty());
    let unscoped = ContextEngine::new(engine.store.clone());
    assert!(matches!(
        unscoped.read_current(&refs, &subject, ContextScope::CurrentRequest, Some(100)),
        Err(CurrentReadError::InvalidSubject)
    ));
}
