#[path = "../../../../test-support/context.rs"]
pub mod context_fixture;
use sem_lang_context::*;

pub fn promote(
    engine: &mut ContextEngine,
    candidate: ContextDefaultCandidate,
    decision: &str,
) -> Result<PromotionReceipt, ContextError> {
    let subject = ContextSubject {
        id: "fixture:person".into(),
        class: "person".into(),
    };
    // This fixture explicitly selects a correction; production callers must
    // obtain this operation and assertion reference from their own decision.
    let operation = engine
        .store
        .persistent_defaults
        .iter()
        .find(|item| {
            item.subject == subject
                && item.kind == candidate.kind
                && item.scope == candidate.inferred_scope
                && item.value != candidate.value
        })
        .map(|item| PromotionOperation::CorrectLongTerm {
            assertion_ref: item.id.clone(),
        })
        .unwrap_or(PromotionOperation::PromoteLongTerm);
    let request = PromotionRequest {
        subject_ref: subject,
        candidate_ref: candidate.reference()?,
        expected_memory_revision: engine.memory_revision()?,
        expected_commit_revision: engine.commit_revision()?,
        decision_ref: decision.into(),
        operation,
    };
    engine.promote(candidate, &request)
}

pub fn timezone(
    engine: &mut ContextEngine,
    value: &str,
    decision: &str,
) -> Result<PromotionReceipt, ContextError> {
    promote(
        engine,
        ContextDefaultCandidate {
            kind: ContextKind::TimeZone,
            value: value.into(),
            inferred_scope: ContextScope::PersistentAgentDefault,
            inferred_validity: ContextValidity::default(),
            confidence: 1.0,
            supporting_evidence: vec![EvidenceRef {
                reference: "fixture:selection".into(),
                detail: "explicit test input".into(),
            }],
            counter_evidence: vec![],
        },
        decision,
    )
}
