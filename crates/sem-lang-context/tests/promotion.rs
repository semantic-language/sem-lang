use context_fixture::ContextFixture;
use sem_lang_context::{ContextEngine, ContextStore};
use sem_lang_repository::InMemoryObjectStore;
use support::context_fixture;

#[test]
fn observations_remain_candidates_until_an_explicit_decision_is_recorded() {
    let mut engine = ContextEngine::new(ContextStore::default());
    engine.record_timezone_confirmation("Asia/Tokyo");
    engine.record_timezone_confirmation("Asia/Tokyo");
    let before = serde_json::to_value(&engine.store).unwrap();
    let candidates = engine.induce_defaults();
    assert_eq!(candidates.len(), 1);
    assert_eq!(serde_json::to_value(&engine.store).unwrap(), before);
    for reference in ["", "  "] {
        assert!(support::promote(&mut engine, candidates[0].clone(), reference).is_err());
        assert_eq!(serde_json::to_value(&engine.store).unwrap(), before);
    }
    support::promote(&mut engine, candidates[0].clone(), "decision:confirmed:1").unwrap();
    let accepted = &engine.store.persistent_defaults[0];
    assert_eq!(accepted.value, "Asia/Tokyo");
    assert_eq!(accepted.promotion_ref, "decision:confirmed:1");
    let mut repository = InMemoryObjectStore::new();
    let id = engine.store.save_to_repository(&mut repository).unwrap();
    let restored = ContextStore::load_from_repository(&repository, &id).unwrap();
    assert_eq!(
        restored.revisions[0].defaults[0].promotion_ref,
        "decision:confirmed:1"
    );
    let mut legacy = serde_json::to_value(&restored).unwrap();
    legacy["persistent_defaults"][0]
        .as_object_mut()
        .unwrap()
        .remove("promotion_ref");
    assert!(serde_json::from_value::<ContextStore>(legacy).is_err());
    engine
        .retain_observation(observation_fixture::request(
            "evidence:1",
            "unresolved evidence",
        ))
        .unwrap();
    engine.clear_short_term();
    assert!(engine.short_term().is_empty());
    assert_eq!(
        engine.store.persistent_defaults[0].promotion_ref,
        "decision:confirmed:1"
    );
    assert_eq!(engine.store.revisions.len(), 1);
}

#[test]
fn invalid_parameters_are_rejected_and_evidence_is_not_authorization() {
    let mut engine = ContextEngine::new(ContextStore::default());
    engine.record_timezone_confirmation("UTC");
    engine.record_timezone_confirmation("UTC");
    let candidate = engine.induce_defaults().remove(0);
    for confidence in [f32::NAN, f32::INFINITY, -1.0, 1.1] {
        let mut invalid = candidate.clone();
        invalid.confidence = confidence;
        assert!(support::promote(&mut engine, invalid, "decision:1").is_err());
    }
    let mut conflicting = candidate;
    conflicting.counter_evidence = conflicting.supporting_evidence.clone();
    // A conflict must remain inspectable. It is not an authorization mechanism:
    // the consuming authority explicitly chooses whether to adopt this candidate.
    assert!(support::promote(&mut engine, conflicting.clone(), "").is_err());
    assert!(engine.store.persistent_defaults.is_empty());
    assert!(engine.store.revisions.is_empty());
    assert!(support::timezone(&mut engine, "UTC", "").is_err());
    conflicting.confidence = 0.0;
    let reference = conflicting.reference().unwrap();
    support::promote(&mut engine, conflicting.clone(), "decision:explicit-review").unwrap();
    assert_eq!(
        engine.store.decided_candidates[&reference]
            .candidate
            .counter_evidence,
        conflicting.counter_evidence
    );
}
mod observation_fixture;
mod support;
