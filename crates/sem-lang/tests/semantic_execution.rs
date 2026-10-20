//! Standard operation -> exact role mapping -> explicit surface admission.
#![cfg(feature = "semantic-execution")]
use sem_lang::current::{ProcessingError, ProcessingOutcome};
use sem_lang::semantic_execution::*;
use sem_lang::surface_admission::SurfaceAdmission;
use sem_lang::{
    CompileContext, Engine, InMemoryRepository, RuntimeState, SurfaceInput, restoration,
};
use sem_lang_context::{ContextScope, ContextStore, ContextSubject};
use sem_lang_core::{NodeId, PacketId, Value};
use sem_lang_memory::{bindings::DefinitionRef, definitions};
use sem_lang_repository::{HeadStore, RevisionRef};
use std::collections::BTreeMap;

#[test]
fn standard_inference_has_exact_roles_and_no_implicit_semantic_write() {
    let mut repo = InMemoryRepository::default();
    let module = sem_lang::standard::inference();
    let term = module.terms[0].id;
    let role = module.roles[0].id;
    let ty = module.types[0].id;
    let revision = definitions::adopt(
        &mut repo,
        "standard",
        None,
        &[module],
        "adopt:standard",
        &RevisionRef::default(),
    )
    .unwrap();
    let restoration = restoration::checkpoint(
        &mut repo,
        &[restoration::RevisionReference {
            namespace: "standard".into(),
            revision: revision.clone(),
        }],
        &[],
        &ContextStore::default(),
        &RuntimeState::new(),
    )
    .unwrap();
    let engine = Engine::restore_view(&repo, restoration).unwrap();
    let context = CompileContext {
        subject: ContextSubject {
            id: "owner".into(),
            class: "person".into(),
        },
        evidence: vec![],
        at: None,
        locale: "en".into(),
        semantic_revision: engine
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap(),
        context_refs: vec![],
        context_scope: ContextScope::CurrentRequest,
        observation_evidence: vec![],
    };
    let operation = DefinitionRef {
        namespace: "standard".into(),
        revision: revision.clone(),
        term,
    };
    let exact_role = ExactRoleRef {
        namespace: "standard".into(),
        revision: revision.clone(),
        id: role,
    };
    let requirement = TypeRequirement {
        reference: ExactTypeRef {
            namespace: "standard".into(),
            revision,
            id: ty,
        },
        kind: ValueKind::Text,
    };
    let contract = SemanticOperationContract {
        owner: "application".into(),
        id: "input".into(),
        inputs: BTreeMap::from([("input".into(), requirement.clone())]),
        input_roles: BTreeMap::from([("input".into(), exact_role.clone())]),
        output: requirement.clone(),
    };
    let arguments = vec![SemanticArgument {
        role: exact_role,
        value: TypedValue {
            type_ref: requirement.reference,
            value: Value::Text("The opposite of no is".into()),
        },
    }];
    let input = SurfaceInput::new("input:1", "infer");
    let before = repo.head_revision("definitions-standard").unwrap();
    let (resolution, typed) = engine
        .execution_input(&repo, &operation, &contract, &arguments, &input, &context)
        .unwrap();
    assert!(
        matches!(resolution, ProcessingOutcome::Resolved(_)),
        "{resolution:?}"
    );
    assert_eq!(typed.unwrap().arguments["input"], arguments[0].value);
    let mut wrong = arguments.clone();
    wrong[0].role.id = sem_lang_core::RoleId::new_random(44, 55);
    assert!(matches!(
        engine.execution_input(&repo, &operation, &contract, &wrong, &input, &context),
        Err(InputError::Contract(ContractError::Role))
    ));
    let mut stale = operation.clone();
    stale.revision = "unknown".into();
    assert!(matches!(
        engine.execution_input(&repo, &stale, &contract, &arguments, &input, &context),
        Err(InputError::OperationNotDefined)
    ));
    let (unresolved, request) = engine
        .execution_input(
            &repo,
            &operation,
            &contract,
            &arguments,
            &SurfaceInput::new("unknown", "unknown expression"),
            &context,
        )
        .unwrap();
    assert!(matches!(unresolved, ProcessingOutcome::NeedsResolution(_)));
    assert!(request.is_none());
    let admission = SurfaceAdmission {
        input,
        context,
        packet_id: PacketId::new_random(1, 2),
        claim_id: NodeId::new_random(3, 4),
        decision_ref: "explicit:1".into(),
        observed_at: 0,
        evidence_refs: vec![],
    };
    // A resolved operation alone is not an assertion. The packet constructor
    // cannot convert a primitive or arbitrary model text into truth.
    assert_eq!(
        engine.surface_packet(&admission).unwrap_err(),
        ProcessingError::UnsupportedRepresentation
    );
    let mut invalid = admission.clone();
    invalid.input.text = "\0".into();
    assert_eq!(
        engine.surface_packet(&invalid).unwrap_err(),
        ProcessingError::InvalidInput
    );
    assert_eq!(repo.head_revision("definitions-standard").unwrap(), before);
    assert!(engine.runtime().state.claims.is_empty());
    assert!(engine.context().store.short_term.is_empty());
}

#[test]
fn surface_observation_requires_resolution_before_owner_packet_admission() {
    use sem_lang_core::{Argument, Claim, TermId, Validity};
    use sem_lang_linguistics::{SemanticDefinition, SemanticExpr};
    let mut repo = InMemoryRepository::default();
    let mut module = sem_lang::standard::inference();
    let predicate = module.terms[0].id;
    let role = module.roles[0].id;
    let mut statement = module.terms[0].clone();
    statement.id = TermId::new_random(30, 1);
    statement.label = "Recorded candidate".into();
    statement.semantic_definition = Some(SemanticDefinition::Composition(SemanticExpr::Apply {
        predicate,
        arguments: vec![(role, Box::new(SemanticExpr::Variable("content".into())))],
    }));
    let mut lexical = module.lexical_bindings[0].clone();
    lexical.surface_form = "recorded candidate".into();
    lexical.normalized_form = lexical.surface_form.clone();
    lexical.target_term = statement.id;
    module.terms.push(statement);
    module.lexical_bindings.push(lexical.clone());
    let subject = ContextSubject {
        id: "owner".into(),
        class: "person".into(),
    };
    let actor = Value::Map(BTreeMap::from([
        ("id".into(), Value::Text(subject.id.clone())),
        ("class".into(), Value::Text(subject.class.clone())),
    ]));
    let claim = Claim {
        id: NodeId::new_random(31, 1),
        actor: Some(actor),
        predicate,
        arguments: vec![Argument {
            role: Some(role),
            value: Value::Text("prior explicitly recorded content".into()),
        }],
        constraints: vec![],
        validity: Validity {
            valid_from: None,
            valid_until: None,
        },
    };
    let mut state = RuntimeState::new();
    state.claims.insert(claim.id, claim.clone());
    let revision = definitions::adopt(
        &mut repo,
        "standard",
        None,
        &[module.clone()],
        "definitions:1",
        &RevisionRef::default(),
    )
    .unwrap();
    let restoration = restoration::checkpoint(
        &mut repo,
        &[restoration::RevisionReference {
            namespace: "standard".into(),
            revision,
        }],
        &[],
        &ContextStore::default(),
        &state,
    )
    .unwrap();
    let engine = Engine::restore_view(&repo, restoration.clone()).unwrap();
    let context = CompileContext {
        subject,
        evidence: vec![claim.id],
        at: None,
        locale: "en".into(),
        semantic_revision: engine
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap(),
        context_refs: vec![],
        context_scope: ContextScope::CurrentRequest,
        observation_evidence: vec![],
    };
    let admission = SurfaceAdmission {
        input: SurfaceInput::new("observation:exact", "recorded candidate"),
        context,
        packet_id: PacketId::new_random(32, 1),
        claim_id: NodeId::new_random(32, 2),
        decision_ref: "explicit:admit".into(),
        observed_at: 100,
        evidence_refs: vec![ExactRef {
            owner: "physical-owner".into(),
            id: "result".into(),
            revision: "exact-receipt".into(),
        }],
    };
    let before = engine.runtime().state.semantic_state_digest().unwrap();
    let (outcome, packet) = engine.surface_packet(&admission).unwrap();
    assert!(
        matches!(outcome, ProcessingOutcome::Resolved(_)),
        "{outcome:?}"
    );
    let packet = packet.unwrap();
    assert_eq!(
        packet.context_delta.as_ref().unwrap().added_claims[0].arguments,
        claim.arguments
    );
    assert_eq!(packet.correlation.as_deref(), Some("observation:exact"));
    assert!(packet.provenance.as_ref().unwrap().confidence.is_none());
    assert_eq!(
        engine.runtime().state.semantic_state_digest().unwrap(),
        before
    );
    assert!(engine.context().store.short_term.is_empty());
    let request = sem_lang::PacketRequest {
        operation_id: "explicit:admit".into(),
        expected_commit_revision: sem_lang::PacketReducer::commit_revision(&state).unwrap(),
        expected_semantic_digest: before,
    };
    let transition = engine
        .runtime()
        .reduce(&state, &packet, &request)
        .unwrap()
        .into_state();
    assert_eq!(transition.claims.len(), 2);
    let replay = engine
        .runtime()
        .reduce(&transition, &packet, &request)
        .unwrap()
        .into_state();
    assert_eq!(replay.claims.len(), 2);
    let mut unresolved = admission.clone();
    unresolved.context.evidence.clear();
    let (outcome, packet) = engine.surface_packet(&unresolved).unwrap();
    assert!(matches!(outcome, ProcessingOutcome::NeedsResolution(_)));
    assert!(packet.is_none());
    let mut stale = admission.clone();
    stale.context.semantic_revision = "0".repeat(64);
    assert_eq!(
        engine.surface_packet(&stale).unwrap_err(),
        ProcessingError::StaleRevision
    );
    // Equal-priority declared interpretations remain ambiguous, even with
    // valid evidence. They never select a convenient candidate for admission.
    lexical.target_term = predicate;
    module.lexical_bindings.push(lexical);
    let mut ambiguous_repo = InMemoryRepository::default();
    let revision = definitions::adopt(
        &mut ambiguous_repo,
        "standard",
        None,
        &[module],
        "definitions:2",
        &RevisionRef::default(),
    )
    .unwrap();
    let input = restoration::checkpoint(
        &mut ambiguous_repo,
        &[restoration::RevisionReference {
            namespace: "standard".into(),
            revision,
        }],
        &[],
        &ContextStore::default(),
        &state,
    )
    .unwrap();
    let ambiguous_engine = Engine::restore_view(&ambiguous_repo, input).unwrap();
    let mut ambiguous = admission;
    ambiguous.context.semantic_revision = ambiguous_engine
        .current_compiler()
        .unwrap()
        .semantic_revision()
        .unwrap();
    let (outcome, packet) = ambiguous_engine.surface_packet(&ambiguous).unwrap();
    assert!(matches!(outcome, ProcessingOutcome::NeedsResolution(_)));
    assert!(packet.is_none());
}
