//! semantic combinations, not candidate-membership mocks.
use super::*;
use sem_lang::{CompileContext, SurfaceInput};
use sem_lang_context::{
    ContextDefaultCandidate, ContextKind, ContextScope, ContextValidity, PromotionOperation,
};
use sem_lang_frontend::current::{ConfirmationError, SelectionRequest, SemanticSelection};

fn fixture() -> (Engine, SurfaceInput, CompileContext) {
    let mut definitions = module();
    definitions.terms[0].semantic_definition = Some(SemanticDefinition::Composition(
        SemanticExpr::Conjunction(vec![
            SemanticExpr::Apply {
                predicate: term(2),
                arguments: vec![(role(1), Box::new(SemanticExpr::Variable("target".into())))],
            },
            SemanticExpr::Contextual {
                association: sem_lang_core::ContextAssociation {
                    kind: ContextKind::ReferenceDate,
                    semantic_ref: term(3),
                    role: role(2),
                    scope: ContextScope::CurrentRequest,
                    validity: ContextValidity::default(),
                },
                variable: "datetime".into(),
            },
        ]),
    ));
    let selected = scope();
    let mut state = RuntimeState::new();
    state
        .claims
        .insert(node(1), claim(1, selected.actor(), "clinic", None));
    let owner = engine(&state, definitions);
    let context = CompileContext {
        subject: selected.subject,
        evidence: selected.evidence,
        at: Some(100),
        locale: "en".into(),
        semantic_revision: owner
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap(),
        context_refs: vec![],
        context_scope: ContextScope::CurrentRequest,
        observation_evidence: vec![],
    };
    (
        owner,
        SurfaceInput::new("observation:reservation", "reserve this"),
        context,
    )
}

fn request(engine: &Engine, input: &SurfaceInput, context: &CompileContext) -> SelectionRequest {
    let ProcessingOutcome::NeedsResolution(result) =
        engine.compile_surface(input, context).unwrap()
    else {
        panic!("missing date")
    };
    assert_eq!(result.bindings.len(), 1);
    assert_eq!(result.bindings[0].value, Value::Text("clinic".into()));
    assert_eq!(result.issues.len(), 1);
    SelectionRequest {
        subject: context.subject.clone(),
        expected_semantic_revision: context.semantic_revision.clone(),
        resolution_ref: result.identity.clone(),
        decision_ref: "decision:date".into(),
        selections: vec![SemanticSelection {
            residual_ref: result.issue_reference(&result.issues[0]).unwrap(),
            candidate: ContextDefaultCandidate {
                kind: ContextKind::ReferenceDate,
                value: "2026-09-08T09:00:00+09:00".into(),
                inferred_scope: ContextScope::Session,
                inferred_validity: ContextValidity::default(),
                supporting_evidence: vec![],
                counter_evidence: vec![],
                confidence: 0.0,
            },
            operation: PromotionOperation::SelectShortTerm,
        }],
    }
}

#[test]
fn selection_preview_preserves_target_uses_formal_short_term_and_never_mutates_source() {
    let (engine, input, context) = fixture();
    let request = request(&engine, &input, &context);
    let before = serde_json::to_value(&engine.context().store).unwrap();
    let preview = engine
        .current_compiler()
        .unwrap()
        .validate_selection(&input, &context, &request)
        .unwrap();
    let ProcessingOutcome::Resolved(result) = preview.outcome() else {
        panic!("date selection must resolve")
    };
    assert_eq!(result.bindings.len(), 2);
    assert!(
        result
            .bindings
            .iter()
            .any(|v| v.variable.id == "target" && v.value == Value::Text("clinic".into()))
    );
    assert!(result.bindings.iter().any(|v| v.variable.id == "datetime"
        && v.value == Value::Text(request.selections[0].candidate.value.clone())));
    assert_ne!(result.semantic_revision, context.semantic_revision);
    assert_eq!(
        before,
        serde_json::to_value(&engine.context().store).unwrap()
    );
    assert!(preview.memory().persistent_defaults.is_empty());
    assert_eq!(preview.receipts().len(), 1);
    assert_eq!(preview.receipts()[0].request.decision_ref, "decision:date");
    let mut cleared = sem_lang_context::ContextEngine::new(preview.memory().clone());
    cleared.clear_short_term();
    assert!(cleared.store.persistent_defaults.is_empty());
}

#[test]
fn wrong_scope_stale_unknown_duplicate_and_semantically_wrong_selections_are_rejected() {
    let (engine, input, context) = fixture();
    let original = request(&engine, &input, &context);
    let before = serde_json::to_value(&engine.context().store).unwrap();
    let mut invalid = original.clone();
    invalid.subject.id = "another".into();
    assert!(matches!(
        engine
            .current_compiler()
            .unwrap()
            .validate_selection(&input, &context, &invalid),
        Err(ConfirmationError::SubjectMismatch)
    ));
    invalid = original.clone();
    invalid.expected_semantic_revision = "0".repeat(64);
    assert!(matches!(
        engine
            .current_compiler()
            .unwrap()
            .validate_selection(&input, &context, &invalid),
        Err(ConfirmationError::StaleRevision)
    ));
    invalid = original.clone();
    invalid.resolution_ref = "0".repeat(64);
    assert!(
        engine
            .current_compiler()
            .unwrap()
            .validate_selection(&input, &context, &invalid)
            .is_err()
    );
    invalid = original.clone();
    invalid.selections[0].residual_ref = "0".repeat(64);
    assert!(
        engine
            .current_compiler()
            .unwrap()
            .validate_selection(&input, &context, &invalid)
            .is_err()
    );
    invalid = original.clone();
    invalid.selections.push(invalid.selections[0].clone());
    assert!(matches!(
        engine
            .current_compiler()
            .unwrap()
            .validate_selection(&input, &context, &invalid),
        Err(ConfirmationError::DuplicateSelection)
    ));
    for kind in [
        ContextKind::Location,
        ContextKind::TimeZone,
        ContextKind::Agent,
    ] {
        invalid = original.clone();
        invalid.selections[0].candidate.kind = kind;
        assert!(matches!(
            engine
                .current_compiler()
                .unwrap()
                .validate_selection(&input, &context, &invalid),
            Err(ConfirmationError::InapplicableSelection)
        ));
    }
    invalid = original.clone();
    invalid.selections[0].candidate.inferred_scope = ContextScope::Environment;
    assert!(
        engine
            .current_compiler()
            .unwrap()
            .validate_selection(&input, &context, &invalid)
            .is_err()
    );
    invalid = original.clone();
    invalid.selections[0]
        .candidate
        .inferred_validity
        .valid_until = Some(99);
    assert!(
        engine
            .current_compiler()
            .unwrap()
            .validate_selection(&input, &context, &invalid)
            .is_err()
    );
    invalid = original.clone();
    invalid.selections[0].candidate.confidence = f32::NAN;
    assert!(
        engine
            .current_compiler()
            .unwrap()
            .validate_selection(&input, &context, &invalid)
            .is_err()
    );
    assert_eq!(
        before,
        serde_json::to_value(&engine.context().store).unwrap()
    );
}

#[test]
fn explicit_promotion_and_rejection_are_not_temporary_selection_or_score_policy() {
    let (engine, input, context) = fixture();
    let mut request = request(&engine, &input, &context);
    request.selections[0].operation = PromotionOperation::PromoteLongTerm;
    let preview = engine
        .current_compiler()
        .unwrap()
        .validate_selection(&input, &context, &request)
        .unwrap();
    assert!(matches!(preview.outcome(), ProcessingOutcome::Resolved(_)));
    assert_eq!(preview.memory().persistent_defaults.len(), 1);
    assert_eq!(
        preview.memory().persistent_defaults[0].promotion_ref,
        request.decision_ref
    );
    request.selections[0].operation = PromotionOperation::Reject;
    let preview = engine
        .current_compiler()
        .unwrap()
        .validate_selection(&input, &context, &request)
        .unwrap();
    assert!(matches!(
        preview.outcome(),
        ProcessingOutcome::NeedsResolution(_)
    ));
    assert!(preview.memory().persistent_defaults.is_empty());
    assert_eq!(
        preview
            .memory()
            .decided_candidates
            .values()
            .next()
            .unwrap()
            .disposition,
        sem_lang_context::CandidateDisposition::Rejected
    );
}

#[test]
fn owner_plans_retain_confirm_restore_exactly_and_rejections_write_no_object() {
    use sem_lang::{feedback, restoration};
    use sem_lang_context::{ObservationRequest, SurfaceRetention};
    use sem_lang_repository::Repository;
    let (owner, input, mut context) = fixture();
    let mut repository = InMemoryRepository::default();
    let definition = adoption_fixture::adopt(
        &mut repository,
        "test",
        None,
        &owner.registry().modules().cloned().collect::<Vec<_>>(),
        "decision:fixture",
    )
    .unwrap();
    let initial = restoration::checkpoint(
        &mut repository,
        &[restoration::RevisionReference {
            namespace: "test".into(),
            revision: definition,
        }],
        &[],
        &owner.context().store,
        &owner.runtime().state,
    )
    .unwrap();
    let count = repository.verify().unwrap().objects_checked;
    let retain = feedback::retain(
        &repository,
        &initial,
        context_fixture::formal(
            owner.context(),
            context.subject.clone(),
            context.context_scope,
            sem_lang_context::ContextOperation::Retain {
                observation: ObservationRequest {
                    input: input.clone(),
                    context: context.clone(),
                    retention: SurfaceRetention::KeepForSession,
                },
            },
        ),
    )
    .unwrap();
    assert_eq!(
        count,
        repository.verify().unwrap().objects_checked,
        "planning cannot write"
    );
    context = retain.context.clone();
    let observation_ref = retain.observation_ref.clone();
    let retained = retain.checkpoint.persist(&mut repository).unwrap();
    let read = feedback::RetainedRead {
        observation_ref,
        subject: context.subject.clone(),
        semantic_revision: context.semantic_revision.clone(),
        context_refs: context.context_refs.clone(),
    };
    let ProcessingOutcome::NeedsResolution(result) =
        feedback::resume(&repository, &retained, &read).unwrap()
    else {
        panic!("date unresolved")
    };
    let mut selection = request(
        &owner,
        &input,
        &CompileContext {
            semantic_revision: owner
                .current_compiler()
                .unwrap()
                .semantic_revision()
                .unwrap(),
            ..context.clone()
        },
    );
    selection.expected_semantic_revision = read.semantic_revision.clone();
    selection.resolution_ref = result.identity.clone();
    selection.selections[0].residual_ref = result.issue_reference(&result.issues[0]).unwrap();
    let count = repository.verify().unwrap().objects_checked;
    let mut bad = selection.clone();
    bad.subject.id = "other".into();
    assert!(feedback::select(&repository, &retained, &read, &bad).is_err());
    assert_eq!(count, repository.verify().unwrap().objects_checked);
    let option = feedback::prepare_option(&repository, &retained, &read, &selection).unwrap();
    assert_eq!(count, repository.verify().unwrap().objects_checked);
    let option = option.persist(&mut repository).unwrap();
    let count = repository.verify().unwrap().objects_checked;
    for changed in ["decision", "consequence", "subject"] {
        let mut wrong = option.clone();
        match changed {
            "decision" => wrong.decision_ref = "other-decision".into(),
            "consequence" => wrong.consequence_ref = "0".repeat(64),
            _ => wrong.subject.id = "another".into(),
        }
        assert!(feedback::select_option(&repository, &retained, &wrong).is_err());
        assert_eq!(count, repository.verify().unwrap().objects_checked);
    }
    let selected = feedback::select_option(&repository, &retained, &option).unwrap();
    let next_read = feedback::RetainedRead {
        semantic_revision: selected.context.semantic_revision.clone(),
        context_refs: selected.context.context_refs.clone(),
        ..read.clone()
    };
    let expected = selected.outcome.clone();
    let receipts = selected.receipts.clone();
    let confirmed = selected.checkpoint.persist(&mut repository).unwrap();
    assert_eq!(
        feedback::resume(&repository, &confirmed, &next_read).unwrap(),
        expected
    );
    assert!(matches!(expected, ProcessingOutcome::Resolved(_)));
    // Repeating the same immutable source plan produces the exact same target,
    // operation receipt, and object IDs. Publication CAS is the caller's fence.
    let retry = feedback::select(&repository, &retained, &read, &selection).unwrap();
    assert_eq!(
        serde_json::to_value(retry.receipts).unwrap(),
        serde_json::to_value(receipts).unwrap()
    );
    let count = repository.verify().unwrap().objects_checked;
    assert_eq!(
        retry.checkpoint.persist(&mut repository).unwrap(),
        confirmed
    );
    assert_eq!(count, repository.verify().unwrap().objects_checked);
    assert!(
        feedback::resume(&repository, &confirmed, &read).is_err(),
        "never follow current head for stale input"
    );
    let mut modified_state = owner.runtime().state.clone();
    modified_state
        .annotations
        .insert("audit".into(), "not a fact".into());
    let memory = ContextStore::load_from_repository(&repository, &confirmed.memory).unwrap();
    let audit_only = restoration::checkpoint(
        &mut repository,
        &confirmed.definitions,
        &confirmed.bindings,
        &memory,
        &modified_state,
    )
    .unwrap();
    assert_eq!(
        feedback::resume(&repository, &audit_only, &next_read).unwrap(),
        expected
    );
    let mut discarded_input = input;
    discarded_input.reference = "observation:discarded".into();
    context.semantic_revision = owner
        .current_compiler()
        .unwrap()
        .semantic_revision()
        .unwrap();
    let discard = feedback::retain(
        &repository,
        &initial,
        context_fixture::formal(
            owner.context(),
            context.subject.clone(),
            context.context_scope,
            sem_lang_context::ContextOperation::Retain {
                observation: ObservationRequest {
                    input: discarded_input,
                    context,
                    retention: SurfaceRetention::DiscardAfterCompile,
                },
            },
        ),
    )
    .unwrap();
    let discarded_read = feedback::RetainedRead {
        observation_ref: discard.observation_ref,
        subject: discard.context.subject,
        semantic_revision: discard.context.semantic_revision,
        context_refs: discard.context.context_refs,
    };
    let discarded = discard.checkpoint.persist(&mut repository).unwrap();
    assert!(matches!(
        feedback::resume(&repository, &discarded, &discarded_read),
        Err(feedback::FeedbackError::Confirmation(
            ConfirmationError::Observation(sem_lang_context::ObservationError::SurfaceUnavailable)
        ))
    ));
    let stored = ContextStore::load_from_repository(&repository, &discarded.memory).unwrap();
    assert!(stored.short_term[0].surface.is_none());
}
