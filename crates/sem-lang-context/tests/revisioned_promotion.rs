#[path = "../../../test-support/context.rs"]
mod context_fixture;
use context_fixture::ContextFixture;
mod current_fixture;
fn selected(engine: &ContextEngine) -> Vec<String> {
    current_fixture::values(
        engine,
        &ContextSubject {
            id: "person:alice".into(),
            class: "person".into(),
        },
    )
}
use sem_lang_context::{
    ContextEngine, ContextError, ContextStore, ContextSubject, PromotionOperation, PromotionRequest,
};

fn decide(
    engine: &mut ContextEngine,
    candidate: sem_lang_context::ContextDefaultCandidate,
    operation: PromotionOperation,
    decision: &str,
) -> Result<sem_lang_context::PromotionReceipt, ContextError> {
    let request = PromotionRequest {
        subject_ref: ContextSubject {
            id: "person:alice".into(),
            class: "person".into(),
        },
        candidate_ref: candidate.reference()?,
        expected_memory_revision: engine.memory_revision()?,
        expected_commit_revision: engine.commit_revision()?,
        decision_ref: decision.into(),
        operation,
    };
    engine.promote(candidate, &request)
}

#[test]
fn selections_clear_rejections_replacements_and_corrections_are_distinct() {
    let mut engine = ContextEngine::new(ContextStore::default());
    engine.record_timezone_confirmation("Asia/Tokyo");
    engine.record_timezone_confirmation("Asia/Tokyo");
    let candidate = engine.induce_defaults().remove(0);
    decide(
        &mut engine,
        candidate.clone(),
        PromotionOperation::Retain,
        "retain",
    )
    .unwrap();
    assert_eq!(engine.induce_defaults().len(), 1);
    decide(
        &mut engine,
        candidate.clone(),
        PromotionOperation::SelectShortTerm,
        "temporary",
    )
    .unwrap();
    assert_eq!(selected(&engine)[0], "Asia/Tokyo");
    assert!(engine.store.persistent_defaults.is_empty());
    assert!(engine.induce_defaults().is_empty());
    engine.clear_short_term();
    assert!(selected(&engine).is_empty());
    decide(
        &mut engine,
        candidate.clone(),
        PromotionOperation::PromoteLongTerm,
        "permanent",
    )
    .unwrap();
    let mut correction = candidate.clone();
    correction.value = "Europe/London".into();
    assert!(matches!(
        decide(
            &mut engine,
            correction.clone(),
            PromotionOperation::PromoteLongTerm,
            "silent-correction"
        ),
        Err(ContextError::CorrectionRequired)
    ));
    let assertion_ref = engine.store.persistent_defaults[0].id.clone();
    decide(
        &mut engine,
        correction.clone(),
        PromotionOperation::CorrectLongTerm { assertion_ref },
        "correction",
    )
    .unwrap();
    assert_eq!(selected(&engine)[0], "Europe/London");
    assert_eq!(engine.store.persistent_defaults.len(), 1);
    let mut other = candidate.clone();
    other.value = "UTC".into();
    decide(
        &mut engine,
        other.clone(),
        PromotionOperation::Reject,
        "reject",
    )
    .unwrap();
    assert!(engine.induce_defaults().is_empty());
    let mut replacement = other.clone();
    replacement.value = "Europe/Paris".into();
    decide(
        &mut engine,
        other,
        PromotionOperation::Replace {
            candidate: replacement.clone(),
        },
        "replace",
    )
    .unwrap();
    assert_eq!(engine.induce_defaults(), vec![replacement]);
    assert_eq!(selected(&engine)[0], "Europe/London");
    engine.clear_short_term();
    assert_eq!(selected(&engine)[0], "Europe/London");
}

#[test]
fn explicit_subject_candidate_revision_decision_and_retry_survive_restart() {
    let mut engine = ContextEngine::new(ContextStore::default());
    engine.record_timezone_confirmation("Asia/Tokyo");
    engine.record_timezone_confirmation("Asia/Tokyo");
    let candidate = engine.induce_defaults().remove(0);
    let request = PromotionRequest {
        subject_ref: ContextSubject {
            id: "person:alice".into(),
            class: "person".into(),
        },
        candidate_ref: candidate.reference().unwrap(),
        expected_memory_revision: engine.memory_revision().unwrap(),
        expected_commit_revision: engine.commit_revision().unwrap(),
        decision_ref: "decision:one".into(),
        operation: PromotionOperation::PromoteLongTerm,
    };
    let mut wrong_candidate = request.clone();
    wrong_candidate.candidate_ref = "different".into();
    assert!(engine.promote(candidate.clone(), &wrong_candidate).is_err());
    let receipt = engine.promote(candidate.clone(), &request).unwrap();
    assert_eq!(
        engine.store.persistent_defaults[0].subject,
        request.subject_ref
    );
    assert_eq!(
        engine.store.persistent_defaults[0].promotion_ref,
        request.decision_ref
    );
    assert_eq!(receipt.input_revision, request.expected_memory_revision);
    assert_ne!(receipt.output_revision, receipt.input_revision);
    let mut stale = request.clone();
    stale.decision_ref = "decision:other".into();
    assert!(matches!(
        engine.promote(candidate.clone(), &stale),
        Err(ContextError::RevisionConflict)
    ));
    let restored: ContextStore =
        serde_json::from_slice(&serde_json::to_vec(&engine.store).unwrap()).unwrap();
    let mut restarted = ContextEngine::new(restored);
    assert_eq!(
        serde_json::to_value(restarted.promote(candidate, &request).unwrap()).unwrap(),
        serde_json::to_value(receipt).unwrap()
    );
}

#[test]
fn clearing_a_temporary_selection_does_not_revalidate_an_old_request() {
    let mut source = ContextEngine::new(ContextStore::default());
    source.record_timezone_confirmation("UTC");
    source.record_timezone_confirmation("UTC");
    let candidate = source.induce_defaults().remove(0);
    let mut engine = ContextEngine::new(ContextStore::default());
    let request = PromotionRequest {
        subject_ref: ContextSubject {
            id: "person:alice".into(),
            class: "person".into(),
        },
        candidate_ref: candidate.reference().unwrap(),
        expected_memory_revision: engine.memory_revision().unwrap(),
        expected_commit_revision: engine.commit_revision().unwrap(),
        decision_ref: "temporary".into(),
        operation: PromotionOperation::SelectShortTerm,
    };
    engine.promote(candidate.clone(), &request).unwrap();
    engine.clear_short_term();
    assert!(selected(&engine).is_empty());
    let mut stale = request.clone();
    stale.decision_ref = "stale".into();
    assert!(matches!(
        engine.promote(candidate.clone(), &stale),
        Err(ContextError::CommitConflict(
            sem_lang_context::CommitConflict::Stale { .. }
        ))
    ));
    let revision = engine.memory_revision().unwrap();
    engine.promote(candidate, &request).unwrap();
    assert_eq!(
        engine.memory_revision().unwrap(),
        revision,
        "retry must not recreate a cleared selection"
    );
}
