//! Exact owner observations replace guessed local time and semantic-expression strings.
#[path = "../../../test-support/context.rs"]
mod context_fixture;
use context_fixture::ContextFixture;
use sem_lang_context::{
    ContextEngine, ContextError, ContextStore, ObservationError, SurfaceRetention,
};
mod observation_fixture;
use observation_fixture::request;

#[test]
fn retained_input_is_scoped_idempotent_bounded_and_never_an_invented_interpretation() {
    let mut engine = ContextEngine::new(ContextStore::default());
    let input = request("observation:1", "9 cats, not a declared time");
    let subject = input.context.subject.clone();
    let first = engine.retain_observation(input.clone()).unwrap();
    assert!(first.changed);
    let revision = engine.memory_revision().unwrap();
    let stored = engine.observation(&first.reference, &subject).unwrap();
    assert_eq!(
        stored.surface.as_deref(),
        Some("9 cats, not a declared time")
    );
    assert_eq!(stored.input_ref, "observation:1");
    assert_eq!(stored.context.subject, subject);
    assert_eq!(stored.surface_input().unwrap().reference, first.reference);
    let bytes = serde_json::to_string(stored).unwrap();
    for absent in [
        "local_time",
        "semantic_expression",
        "context_holes",
        "history",
    ] {
        assert!(!bytes.contains(absent));
    }
    assert!(!engine.retain_observation(input.clone()).unwrap().changed);
    assert_eq!(engine.memory_revision().unwrap(), revision);
    let mut changed = input.clone();
    changed.input.text = "a different input".into();
    assert!(matches!(
        engine.retain_observation(changed),
        Err(ContextError::Observation(ObservationError::Conflict))
    ));
    assert_eq!(engine.memory_revision().unwrap(), revision);
    let mut other_subject = subject.clone();
    other_subject.id = "different:person".into();
    assert!(matches!(
        engine.observation(&first.reference, &other_subject),
        Err(ObservationError::SubjectMismatch)
    ));
    assert!(matches!(
        engine.observation(&"0".repeat(64), &subject),
        Err(ObservationError::Missing)
    ));
    let mut oversized = input.clone();
    oversized.input.text = "x".repeat(65_537);
    assert!(matches!(
        engine.retain_observation(oversized),
        Err(ContextError::Observation(
            ObservationError::CapacityExceeded { .. }
        ))
    ));
    assert_eq!(engine.memory_revision().unwrap(), revision);
    let mut forged = serde_json::to_value(&input).unwrap();
    forged["semantic_expression"] = serde_json::json!("guessed");
    assert!(serde_json::from_value::<sem_lang_context::ObservationRequest>(forged).is_err());
    let mut corrupted = engine.store.clone();
    corrupted.short_term[0].surface = Some("substituted".into());
    assert!(corrupted.validate_observations().is_err());
    engine.clear_short_term();
    assert!(engine.store.short_term.is_empty());
    assert!(matches!(
        engine.observation(&first.reference, &subject),
        Err(ObservationError::Missing)
    ));
}

#[test]
fn discard_is_explicit_unavailable_not_empty_or_reconstructed_and_cannot_be_sanitized_on_save() {
    let mut engine = ContextEngine::new(ContextStore::default());
    let mut input = request("observation:discard", "private source");
    input.retention = SurfaceRetention::DiscardAfterCompile;
    let subject = input.context.subject.clone();
    let receipt = engine.retain_observation(input).unwrap();
    let stored = engine.observation(&receipt.reference, &subject).unwrap();
    assert!(stored.surface.is_none());
    assert!(matches!(
        stored.surface_input(),
        Err(ObservationError::SurfaceUnavailable)
    ));
    assert!(
        !serde_json::to_string(&engine.store)
            .unwrap()
            .contains("private source")
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("context.json");
    engine.store.save(&path).unwrap();
    let restored = ContextStore::load(&path).unwrap();
    assert_eq!(restored.short_term, engine.store.short_term);
    let before = std::fs::read(&path).unwrap();
    engine.store.short_term[0].surface = Some("forged source".into());
    assert!(engine.store.save(&path).is_err());
    assert_eq!(std::fs::read(path).unwrap(), before);
}
