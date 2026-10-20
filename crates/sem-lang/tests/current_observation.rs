//! current-state and owner confirmation acceptance.
#[path = "../../../test-support/adoption.rs"]
mod adoption_fixture;
#[path = "../../../test-support/context.rs"]
mod context_fixture;
use context_fixture::ContextFixture;
#[path = "current_observation/applicability.rs"]
mod applicability;
#[path = "current_observation/cardinality.rs"]
mod cardinality;
#[path = "current_observation/conditions.rs"]
mod conditions;
#[path = "current_observation/confirmation.rs"]
mod confirmation;
#[path = "current_observation/grammar.rs"]
mod grammar;
use sem_lang::current::{
    EvaluationBound, Observation, ObservationScope, ProcessingError, ProcessingOutcome, Reason,
};
use sem_lang::{Engine, InMemoryRepository, RuntimeState};
use sem_lang_context::{ContextStore, ContextSubject};
use sem_lang_core::{Argument, Claim, NodeId, RoleId, TermId, Validity, Value};
use sem_lang_linguistics::{PrimitiveSemantic, SemanticDefinition, SemanticExpr};
use sem_lang_vocab::*;

fn term(seed: u64) -> TermId {
    TermId::new_random(seed, 1)
}
fn role(seed: u64) -> RoleId {
    RoleId::new_random(seed, 2)
}
fn node(seed: u64) -> NodeId {
    NodeId::new_random(seed, 3)
}
fn scope() -> ObservationScope {
    ObservationScope {
        subject: ContextSubject {
            id: "owner".into(),
            class: "person".into(),
        },
        evidence: vec![node(1)],
        at: Some(100),
    }
}
fn observation(text: &str) -> Observation {
    Observation {
        reference: "observation:1".into(),
        text: text.into(),
        locale: "en".into(),
    }
}
fn module() -> VocabularyModule {
    VocabularyModule {
        schema_version: "0.10.0".into(),
        vocabulary_id: "test.application".into(),
        version: "0.10.0".into(),
        dependencies: vec![],
        terms: (1..=3)
            .map(|seed| TermDefinition {
                id: term(seed),
                label: format!("term{seed}"),
                description: String::new(),
                deprecation: None,
                replacement: None,
                semantic_definition: Some(if seed == 1 {
                    SemanticDefinition::Composition(SemanticExpr::Apply {
                        predicate: term(2),
                        arguments: ["target", "datetime"]
                            .into_iter()
                            .enumerate()
                            .map(|(index, name)| {
                                (
                                    role(index as u64 + 1),
                                    Box::new(SemanticExpr::Variable(name.into())),
                                )
                            })
                            .collect(),
                    })
                } else {
                    SemanticDefinition::Primitive(PrimitiveSemantic::Relation)
                }),
            })
            .collect(),
        types: vec![],
        roles: ["target", "datetime"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| RoleDefinition {
                id: role(index as u64 + 1),
                label: name.into(),
                cardinality: Cardinality::ExactlyOne,
                description: String::new(),
            })
            .collect(),
        lexical_bindings: vec![LexicalBinding {
            language_tag: "en".into(),
            surface_form: "reserve this".into(),
            normalized_form: "reserve this".into(),
            contribution_kind: ContributionKind::Predicative,
            target_term: term(1),
            grammatical_condition: None,
            semantic_type_condition: None,
            priority: 1,
            valid_from: None,
            valid_until: None,
            provenance: "test-only".into(),
        }],
    }
}
fn claim(seed: u64, actor: Value, target: &str, datetime: Option<&str>) -> Claim {
    let mut arguments = vec![Argument {
        role: Some(role(1)),
        value: Value::Text(target.into()),
    }];
    if let Some(value) = datetime {
        arguments.push(Argument {
            role: Some(role(2)),
            value: Value::Text(value.into()),
        });
    }
    Claim {
        id: node(seed),
        actor: Some(actor),
        predicate: term(2),
        arguments,
        constraints: vec![],
        validity: Validity {
            valid_from: None,
            valid_until: None,
        },
    }
}
fn engine(state: &RuntimeState, module: VocabularyModule) -> Engine {
    engine_with_context(state, module, &ContextStore::default())
}

// Reduced C5: malformed semantic references reject at admission, before reads.
fn rejected_checkpoint(
    state: &RuntimeState,
    module: VocabularyModule,
) -> sem_lang::restoration::SemanticReferenceError {
    let mut repository = InMemoryRepository::default();
    let revision =
        adoption_fixture::adopt(&mut repository, "test", None, &[module], "decision:fixture")
            .unwrap();
    match sem_lang::restoration::checkpoint(
        &mut repository,
        &[sem_lang::restoration::RevisionReference {
            namespace: "test".into(),
            revision,
        }],
        &[],
        &ContextStore::default(),
        state,
    ) {
        Err(sem_lang::restoration::RestorationError::SemanticReference(error)) => error,
        other => panic!("expected admission rejection: {other:?}"),
    }
}
fn engine_with_context(
    state: &RuntimeState,
    module: VocabularyModule,
    context: &ContextStore,
) -> Engine {
    let mut repository = InMemoryRepository::default();
    let revision =
        adoption_fixture::adopt(&mut repository, "test", None, &[module], "decision:fixture")
            .unwrap();
    let input = sem_lang::restoration::checkpoint(
        &mut repository,
        &[sem_lang::restoration::RevisionReference {
            namespace: "test".into(),
            revision,
        }],
        &[],
        context,
        state,
    )
    .unwrap();
    let engine = Engine::restore(repository, input).unwrap();
    // The stream implementation must preserve the existing semantic projection,
    // not add packet receipts/audit metadata or change canonical byte order.
    let bytes = serde_json::to_vec(&(
        engine.compiled_language().digest,
        &state.graph,
        &state.claims,
        &state.terms,
        &state.types,
        &state.roles,
        engine.context().memory_revision().unwrap(),
    ))
    .unwrap();
    assert_eq!(
        engine
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap(),
        sem_lang_core::Digest::semantic_state_digest(&bytes).to_hex()
    );
    engine
}

#[test]
fn typed_retained_observation_resumes_only_against_explicit_current_truth() {
    use sem_lang::{CompileContext, SurfaceInput};
    use sem_lang_context::{
        ContextEngine, ContextReference, ContextScope, ObservationError, ObservationRequest,
        SurfaceRetention,
    };
    let selected = scope();
    let mut state = RuntimeState::new();
    state
        .claims
        .insert(node(1), claim(1, selected.actor(), "clinic", None));
    let initial = engine(&state, module());
    let context = CompileContext {
        subject: selected.subject.clone(),
        evidence: selected.evidence.clone(),
        at: selected.at,
        locale: "en".into(),
        semantic_revision: initial
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap(),
        context_refs: vec![],
        context_scope: ContextScope::CurrentRequest,
        observation_evidence: vec![],
    };
    let mut memory = ContextEngine::new(ContextStore::default());
    let receipt = memory
        .retain_observation(ObservationRequest {
            input: SurfaceInput::new("input:reserve", "reserve this"),
            context,
            retention: SurfaceRetention::KeepAsEvidence,
        })
        .unwrap();
    // Every persistence path must reject invalid input before creating objects.
    use sem_lang_repository::ObjectStore;
    let mut repository = InMemoryRepository::default();
    let revision = adoption_fixture::adopt(
        &mut repository,
        "test",
        None,
        &[module()],
        "decision:fixture",
    )
    .unwrap();
    let mut invalid = memory.store.clone();
    invalid.short_term[0].surface = Some("tampered".into());
    let invalid_id =
        sem_lang_core::Digest::object_digest(&serde_json::to_vec(&invalid).unwrap()).to_hex();
    assert!(
        sem_lang::restoration::checkpoint(
            &mut repository,
            &[sem_lang::restoration::RevisionReference {
                namespace: "test".into(),
                revision
            }],
            &[],
            &invalid,
            &state,
        )
        .is_err()
    );
    assert!(!repository.contains_object(&invalid_id));
    let current = engine_with_context(&state, module(), &memory.store);
    let stored = current
        .context()
        .observation(&receipt.reference, &selected.subject)
        .unwrap();
    let input = stored.surface_input().unwrap();
    assert_eq!(input.reference, receipt.reference);
    assert_eq!(input.text, "reserve this");
    // Retention changed the exact memory revision. Never follow a new head silently.
    assert_eq!(
        current.compile_surface(&input, &stored.context),
        Err(ProcessingError::StaleRevision)
    );
    let mut resumed = stored.context.clone();
    resumed.semantic_revision = current
        .current_compiler()
        .unwrap()
        .semantic_revision()
        .unwrap();
    resumed.context_refs = vec![ContextReference::Observation(receipt.reference.clone())];
    let ProcessingOutcome::NeedsResolution(partial) =
        current.compile_surface(&input, &resumed).unwrap()
    else {
        panic!("retained words cannot invent the missing datetime");
    };
    assert_eq!(partial.observation_ref, receipt.reference);
    assert_eq!(partial.bindings.len(), 1);
    assert_eq!(partial.bindings[0].variable.id, "target");
    assert_eq!(partial.bindings[0].value, Value::Text("clinic".into()));
    assert_eq!(partial.issues.len(), 1);
    assert_eq!(partial.issues[0].variable.as_deref(), Some("datetime"));
    // An explicit owner fact changes truth, not a continuation overlay or transcript.
    state.claims.insert(
        node(1),
        claim(1, selected.actor(), "clinic", Some("morning")),
    );
    let next = engine_with_context(&state, module(), &memory.store);
    assert_eq!(
        next.compile_surface(&input, &resumed),
        Err(ProcessingError::StaleRevision)
    );
    resumed.semantic_revision = next
        .current_compiler()
        .unwrap()
        .semantic_revision()
        .unwrap();
    let ProcessingOutcome::Resolved(resolved) = next.compile_surface(&input, &resumed).unwrap()
    else {
        panic!("the current explicit fact completes the same input");
    };
    assert!(resolved.issues.is_empty());
    assert_eq!(resolved.bindings.len(), 2);
    assert!(
        resolved
            .bindings
            .iter()
            .any(|b| b.variable.id == "target" && b.value == Value::Text("clinic".into()))
    );
    assert!(
        resolved
            .bindings
            .iter()
            .any(|b| b.variable.id == "datetime" && b.value == Value::Text("morning".into()))
    );
    memory.clear_short_term();
    let cleared = engine_with_context(&state, module(), &memory.store);
    assert!(matches!(
        cleared
            .context()
            .observation(&receipt.reference, &selected.subject),
        Err(ObservationError::Missing)
    ));
    assert_eq!(cleared.runtime().state.claims, state.claims);
}

#[test]
fn declared_role_identity_is_independent_of_labels_and_rejects_legacy_wire() {
    let selected = scope();
    let mut state = RuntimeState::new();
    state.claims.insert(
        node(1),
        claim(1, selected.actor(), "clinic", Some("morning")),
    );
    let definitions = module();
    let current = engine(&state, definitions.clone());
    let ProcessingOutcome::Resolved(before) = current
        .process_observation(&observation("reserve this"), &selected)
        .unwrap()
    else {
        panic!("resolved fixture")
    };
    let mut renamed = definitions.clone();
    for role in &mut renamed.roles {
        role.label = "same translated display".into();
    }
    let ProcessingOutcome::Resolved(after) = engine(&state, renamed)
        .process_observation(&observation("reserve this"), &selected)
        .unwrap()
    else {
        panic!("renaming labels must not change role interpretation")
    };
    assert_eq!(before.bindings, after.bindings);
    assert_eq!(before.relations, after.relations);
    assert_eq!(before.expression, after.expression);
    let raw = serde_json::to_value(&definitions).unwrap();
    let mut obsolete = raw.clone();
    obsolete["terms"][0]["compatibility_alias"] = serde_json::json!([]);
    assert!(serde_json::from_value::<VocabularyModule>(obsolete).is_err());
    let mut label = raw.clone();
    label["terms"][0]["semantic_definition"]["Composition"]["Apply"]["arguments"][0][0] =
        serde_json::json!("target");
    assert!(serde_json::from_value::<VocabularyModule>(label).is_err());
    let mut missing = raw;
    missing["terms"][0]["semantic_definition"]["Composition"]["Apply"]["arguments"][0][0] =
        serde_json::json!(role(999).to_hex());
    let mut registry = VocabularyRegistry::new();
    registry
        .register(serde_json::from_value::<VocabularyModule>(missing).unwrap())
        .unwrap();
    assert!(registry.validate_definitions().is_err());
}

#[test]
fn contextual_selection_narrows_correlated_relations_without_fabricating_missing_facts() {
    use sem_lang::{CompileContext, SurfaceInput};
    use sem_lang_context::*;
    use sem_lang_core::ContextAssociation;
    let mut definitions = module();
    let SemanticDefinition::Composition(relation) =
        definitions.terms[0].semantic_definition.take().unwrap()
    else {
        panic!("fixture relation")
    };
    definitions.terms[0].semantic_definition = Some(SemanticDefinition::Composition(
        SemanticExpr::Conjunction(vec![
            relation,
            SemanticExpr::Contextual {
                association: ContextAssociation {
                    kind: ContextKind::Location,
                    semantic_ref: term(3),
                    role: role(1),
                    scope: ContextScope::CurrentRequest,
                    validity: ContextValidity::default(),
                },
                variable: "target".into(),
            },
        ]),
    ));
    let mut selected = scope();
    selected.evidence.push(node(2));
    let mut state = RuntimeState::new();
    state.claims.insert(
        node(1),
        claim(1, selected.actor(), "clinic", Some("morning")),
    );
    state.claims.insert(
        node(2),
        claim(2, selected.actor(), "other", Some("evening")),
    );
    let mut store = ContextStore::default();
    store.persistent_defaults.push(PersistentContextualDefault {
        id: "location".into(),
        subject: selected.subject.clone(),
        kind: ContextKind::Location,
        value: "other".into(),
        scope: ContextScope::PersistentAgentDefault,
        validity: ContextValidity::default(),
        evidence: vec![],
        revision: "definition:1".into(),
        promotion_ref: "decision:1".into(),
    });
    let run = |state: &RuntimeState, definitions: VocabularyModule, store: &ContextStore| {
        let engine = engine_with_context(state, definitions, store);
        let input = CompileContext {
            subject: selected.subject.clone(),
            evidence: selected.evidence.clone(),
            at: selected.at,
            locale: "en".into(),
            semantic_revision: engine
                .current_compiler()
                .unwrap()
                .semantic_revision()
                .unwrap(),
            context_refs: vec![ContextReference::Default("location".into())],
            context_scope: ContextScope::CurrentRequest,
            observation_evidence: vec![],
        };
        let outcome = engine
            .compile_surface(&SurfaceInput::new("context:join", "reserve this"), &input)
            .unwrap();
        assert_eq!(
            outcome,
            engine
                .compile_surface(&SurfaceInput::new("context:join", "reserve this"), &input)
                .unwrap()
        );
        assert_eq!(
            engine
                .current_compiler()
                .unwrap()
                .semantic_revision()
                .unwrap(),
            input.semantic_revision
        );
        outcome
    };
    let ProcessingOutcome::Resolved(result) = run(&state, definitions.clone(), &store) else {
        panic!("context must select a tuple, not leave old ambiguity")
    };
    assert_eq!(result.relations[0].rows.len(), 1);
    assert_eq!(
        result
            .bindings
            .iter()
            .find(|b| b.variable.id == "target")
            .unwrap()
            .value,
        Value::Text("other".into())
    );
    assert_eq!(
        result
            .bindings
            .iter()
            .find(|b| b.variable.id == "datetime")
            .unwrap()
            .value,
        Value::Text("evening".into())
    );
    definitions.roles[0].cardinality = Cardinality::OneOrMany;
    let ProcessingOutcome::Resolved(many) = run(&state, definitions.clone(), &store) else {
        panic!("a many summary must not be compared as a scalar")
    };
    assert_eq!(
        many.bindings
            .iter()
            .find(|b| b.variable.id == "target")
            .unwrap()
            .value,
        Value::Sequence(vec![Value::Text("other".into())])
    );
    store.persistent_defaults[0].value = "absent".into();
    let ProcessingOutcome::NeedsResolution(conflict) = run(&state, definitions.clone(), &store)
    else {
        panic!("a contradictory requirement cannot replace facts")
    };
    assert!(conflict.bindings.is_empty());
    assert!(
        conflict
            .issues
            .iter()
            .any(|i| i.reason == Reason::ConflictingEvidence)
    );
    store.persistent_defaults[0].value = "other".into();
    for item in state.claims.values_mut() {
        item.arguments.remove(0);
    }
    let ProcessingOutcome::NeedsResolution(missing) = run(&state, definitions, &store) else {
        panic!("a context default cannot fabricate a missing relational argument")
    };
    assert!(!missing.bindings.iter().any(|b| b.variable.id == "target"));
    assert!(
        missing
            .issues
            .iter()
            .any(|i| i.variable.as_deref() == Some("target") && i.reason == Reason::MissingContext)
    );
}

#[test]
fn contextual_associations_read_real_memory_with_claims_precedence_and_restart() {
    use sem_lang::{CompileContext, SurfaceInput};
    use sem_lang_context::*;
    use sem_lang_core::ContextAssociation;
    let scope = scope();
    let mut module = module();
    let association = ContextAssociation {
        kind: ContextKind::TimeZone,
        semantic_ref: term(2),
        role: role(1),
        scope: ContextScope::CurrentRequest,
        validity: ContextValidity::default(),
    };
    module.terms[0].semantic_definition =
        Some(SemanticDefinition::Composition(SemanticExpr::Contextual {
            association: association.clone(),
            variable: "unrelated-label".into(),
        }));
    let mut context =
        ContextEngine::new(ContextStore::default()).with_subject(scope.subject.clone());
    context
        .store
        .persistent_defaults
        .push(PersistentContextualDefault {
            id: "default".into(),
            subject: scope.subject.clone(),
            kind: ContextKind::TimeZone,
            value: "UTC".into(),
            scope: ContextScope::PersistentAgentDefault,
            validity: ContextValidity::default(),
            evidence: vec![],
            revision: "revision:default".into(),
            promotion_ref: "decision:default".into(),
        });
    let mut expired = context.store.persistent_defaults[0].clone();
    expired.id = "expired".into();
    expired.value = "expired-data".into();
    expired.validity.valid_until = Some(100);
    context.store.persistent_defaults.push(expired);
    context.record_timezone_confirmation("Asia/Tokyo");
    context.record_timezone_confirmation("Asia/Tokyo");
    let candidate = context.induce_defaults().remove(0);
    let candidate_ref = candidate.reference().unwrap();
    context
        .promote(
            candidate.clone(),
            &PromotionRequest {
                subject_ref: scope.subject.clone(),
                candidate_ref: candidate_ref.clone(),
                expected_memory_revision: context.memory_revision().unwrap(),
                expected_commit_revision: context.commit_revision().unwrap(),
                decision_ref: "decision:select".into(),
                operation: PromotionOperation::SelectShortTerm,
            },
        )
        .unwrap();
    let mut state = RuntimeState::new();
    state
        .claims
        .insert(node(1), claim(1, scope.actor(), "long-term", None));
    state
        .claims
        .insert(node(2), claim(2, scope.actor(), "explicit", None));
    let restored = engine_with_context(&state, module.clone(), &context.store);
    let input = SurfaceInput::new("observation:context", "reserve this");
    let mut selection = CompileContext {
        subject: scope.subject.clone(),
        evidence: vec![node(1), node(2)],
        at: Some(100),
        locale: "en".into(),
        semantic_revision: restored
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap(),
        context_refs: vec![
            ContextReference::Default("default".into()),
            ContextReference::Default("expired".into()),
            ContextReference::Decision(candidate_ref.clone()),
        ],
        context_scope: ContextScope::CurrentRequest,
        observation_evidence: vec![node(2)],
    };
    let value = |outcome: ProcessingOutcome| {
        let ProcessingOutcome::Resolved(result) = outcome else {
            panic!("expected resolved")
        };
        assert_eq!(result.bindings.len(), 1);
        assert_eq!(result.bindings[0].variable.id, "unrelated-label");
        assert_eq!(
            result
                .context_refs
                .iter()
                .filter(|(_, s)| *s == ContextStatus::Expired)
                .count(),
            1
        );
        result.bindings[0].value.clone()
    };
    assert_eq!(
        value(restored.compile_surface(&input, &selection).unwrap()),
        Value::Text("explicit".into())
    );
    selection.observation_evidence.clear();
    assert_eq!(
        value(restored.compile_surface(&input, &selection).unwrap()),
        Value::Text("Asia/Tokyo".into())
    );
    let repeated = engine_with_context(&state, module.clone(), &context.store);
    assert_eq!(
        repeated.compile_surface(&input, &selection),
        restored.compile_surface(&input, &selection)
    );
    selection.context_refs.pop();
    selection.evidence = vec![node(1)];
    assert_eq!(
        value(restored.compile_surface(&input, &selection).unwrap()),
        Value::Text("long-term".into())
    );
    selection.evidence.clear();
    assert_eq!(
        value(restored.compile_surface(&input, &selection).unwrap()),
        Value::Text("UTC".into())
    );
    let mut competing = candidate;
    competing.value = "Europe/Paris".into();
    let competing_ref = competing.reference().unwrap();
    context
        .promote(
            competing,
            &PromotionRequest {
                subject_ref: scope.subject.clone(),
                candidate_ref: competing_ref.clone(),
                expected_memory_revision: context.memory_revision().unwrap(),
                expected_commit_revision: context.commit_revision().unwrap(),
                decision_ref: "decision:conflict".into(),
                operation: PromotionOperation::SelectShortTerm,
            },
        )
        .unwrap();
    selection.context_refs.extend([
        ContextReference::Decision(candidate_ref),
        ContextReference::Decision(competing_ref),
    ]);
    let conflicting = engine_with_context(&state, module.clone(), &context.store);
    selection.semantic_revision = conflicting
        .current_compiler()
        .unwrap()
        .semantic_revision()
        .unwrap();
    // Formal selection replaces the old selection; it does not create a conflict.
    assert_eq!(
        value(conflicting.compile_surface(&input, &selection).unwrap()),
        Value::Text("Europe/Paris".into())
    );
    // Two explicitly selected observations conflict at the strongest rank.
    selection.evidence = vec![node(1), node(2)];
    selection.observation_evidence = selection.evidence.clone();
    let ProcessingOutcome::NeedsResolution(result) =
        conflicting.compile_surface(&input, &selection).unwrap()
    else {
        panic!("no weak-default fallthrough")
    };
    assert!(result.bindings.is_empty());
    assert_eq!(result.issues[0].reason, Reason::ConflictingEvidence);
    assert_eq!(
        result.issues[0].candidates,
        vec![
            Value::Text("explicit".into()),
            Value::Text("long-term".into())
        ]
    );
    // Exact references are validated even if no surface can be interpreted.
    selection
        .context_refs
        .push(ContextReference::Default("missing".into()));
    assert_eq!(
        conflicting.compile_surface(&SurfaceInput::new("observation:bad", "???"), &selection),
        Err(ProcessingError::InvalidReference)
    );
    let mut invalid = association;
    invalid.semantic_ref = term(999);
    module.terms[0].semantic_definition =
        Some(SemanticDefinition::Composition(SemanticExpr::Contextual {
            association: invalid,
            variable: "x".into(),
        }));
    assert!(
        adoption_fixture::adopt(
            &mut InMemoryRepository::default(),
            "bad",
            None,
            &[module],
            "decision:bad"
        )
        .is_err()
    );
}

#[test]
fn generic_frontend_consumes_exact_context_and_matches_scoped_processing() {
    use sem_lang::{CompileContext, SurfaceInput};
    let scope = scope();
    let mut state = RuntimeState::new();
    let mut evidence = claim(1, scope.actor(), "clinic", Some("morning"));
    evidence.validity = Validity {
        valid_from: Some(90),
        valid_until: Some(110),
    };
    state.claims.insert(node(1), evidence);
    let mut definitions = module();
    definitions.lexical_bindings[0].valid_from = Some(80);
    definitions.lexical_bindings[0].valid_until = Some(120);
    let mut restored = engine(&state, definitions);
    let input = SurfaceInput::new("observation:1", "reserve this");
    let context = CompileContext {
        context_refs: vec![],
        context_scope: sem_lang_core::ContextScope::CurrentRequest,
        observation_evidence: vec![],
        subject: scope.subject.clone(),
        evidence: scope.evidence.clone(),
        at: scope.at,
        locale: "en".into(),
        semantic_revision: restored
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap(),
    };
    let expected = restored
        .process_observation(&observation("reserve this"), &scope)
        .unwrap();
    assert!(matches!(expected, ProcessingOutcome::Resolved(_)));
    assert_eq!(
        restored.compile_surface(&input, &context).unwrap(),
        expected
    );
    assert_eq!(
        restored
            .current_compiler()
            .unwrap()
            .compile(&input, &context)
            .unwrap(),
        expected
    );
    let standalone = sem_lang_frontend::current::CurrentStateCompiler::new(
        restored.compiled_language(),
        &restored.runtime().state,
        restored.context(),
    );
    assert_eq!(standalone.compile(&input, &context).unwrap(), expected);

    let mut selected = context.clone();
    selected.locale = "ja-JP".into();
    let ProcessingOutcome::NeedsResolution(wrong_locale) =
        restored.compile_surface(&input, &selected).unwrap()
    else {
        panic!("locale must be used")
    };
    assert_eq!(wrong_locale.concept, None);
    assert_eq!(wrong_locale.issues[0].reason, Reason::UnknownExpression);
    selected = context.clone();
    selected.at = Some(115);
    let ProcessingOutcome::NeedsResolution(expired) =
        restored.compile_surface(&input, &selected).unwrap()
    else {
        panic!("expired evidence")
    };
    assert!(expired.bindings.is_empty());
    assert!(expired.evidence_refs.is_empty());
    assert_eq!(expired.issues.len(), 2);
    selected.at = Some(125);
    let ProcessingOutcome::NeedsResolution(expired_language) =
        restored.compile_surface(&input, &selected).unwrap()
    else {
        panic!("expired language")
    };
    assert_eq!(expired_language.concept, None);
    selected.at = None;
    assert_eq!(
        restored.compile_surface(&input, &selected),
        Err(ProcessingError::InvalidInput)
    );
    selected = context.clone();
    selected.subject.id = "another-person".into();
    assert_eq!(
        restored.compile_surface(&input, &selected),
        Err(ProcessingError::InvalidReference)
    );
    selected = context.clone();
    selected.evidence.clear();
    let ProcessingOutcome::NeedsResolution(no_evidence) =
        restored.compile_surface(&input, &selected).unwrap()
    else {
        panic!("no evidence")
    };
    assert!(no_evidence.bindings.is_empty());
    selected = context.clone();
    selected.semantic_revision = "0".repeat(64);
    assert_eq!(
        restored.compile_surface(&input, &selected),
        Err(ProcessingError::StaleRevision)
    );
    selected.semantic_revision.clear();
    assert_eq!(
        restored.compile_surface(&input, &selected),
        Err(ProcessingError::InvalidInput)
    );

    // The engine does not substitute a new revision after a caller's inspection.
    restored.context_mut().record_timezone_confirmation("UTC");
    assert_ne!(
        restored
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap(),
        context.semantic_revision
    );
    assert_eq!(
        restored.compile_surface(&input, &context),
        Err(ProcessingError::StaleRevision)
    );
    let raw = serde_json::to_value(&context).unwrap();
    let mut invalid = raw.clone();
    invalid["runtime_state"] = serde_json::json!({});
    assert!(serde_json::from_value::<CompileContext>(invalid).is_err());
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<CompileContext>(raw.clone()).unwrap())
            .unwrap(),
        raw
    );
    let unadopted = Engine::builder()
        .repository(InMemoryRepository::default())
        .with_vocabulary_module(module())
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(
        unadopted.compile_surface(&input, &context),
        Err(ProcessingError::InvalidReference)
    );
}

#[test]
fn restored_state_drives_partial_resolution_and_exact_repeat_without_history() {
    let scope = scope();
    let mut state = RuntimeState::new();
    state
        .claims
        .insert(node(1), claim(1, scope.actor(), "clinic", None));
    let observation = observation("reserve this");
    let first = engine(&state, module());
    let before = first.runtime().state.semantic_state_digest().unwrap();
    let ProcessingOutcome::NeedsResolution(partial) =
        first.process_observation(&observation, &scope).unwrap()
    else {
        panic!("partial expected")
    };
    assert_eq!(partial.concept, Some(term(1)));
    assert_eq!(partial.bindings.len(), 1);
    assert_eq!(partial.bindings[0].variable.id, "target");
    assert_eq!(partial.bindings[0].value, Value::Text("clinic".into()));
    assert_eq!(partial.issues.len(), 1);
    assert_eq!(partial.issues[0].reason, Reason::MissingContext);
    assert_eq!(partial.issues[0].variable.as_deref(), Some("datetime"));
    assert_eq!(
        first.runtime().state.semantic_state_digest().unwrap(),
        before
    );
    assert_eq!(
        first.process_observation(&observation, &scope).unwrap(),
        engine(&state, module())
            .process_observation(&observation, &scope)
            .unwrap()
    );

    state
        .claims
        .get_mut(&node(1))
        .unwrap()
        .arguments
        .push(Argument {
            role: Some(role(2)),
            value: Value::Text("2026-09-07T09:00:00+09:00".into()),
        });
    let current = engine(&state, module());
    let ProcessingOutcome::Resolved(resolved) =
        current.process_observation(&observation, &scope).unwrap()
    else {
        panic!("resolved expected")
    };
    assert!(resolved.issues.is_empty());
    assert_eq!(resolved.bindings.len(), 2);
    assert_eq!(
        resolved
            .bindings
            .iter()
            .find(|v| v.variable.id == "target")
            .unwrap()
            .value,
        partial.bindings[0].value
    );
    assert_ne!(resolved.identity, partial.identity);
    state
        .annotations
        .insert("audit".into(), "clinic is a different place".into());
    assert_eq!(
        ProcessingOutcome::Resolved(resolved),
        engine(&state, module())
            .process_observation(&observation, &scope)
            .unwrap()
    );
    state.claims.clear();
    let mut no_evidence = scope.clone();
    no_evidence.evidence.clear();
    let ProcessingOutcome::NeedsResolution(empty) = engine(&state, module())
        .process_observation(&observation, &no_evidence)
        .unwrap()
    else {
        panic!("no audit-derived facts")
    };
    assert!(empty.bindings.is_empty());
    assert_eq!(empty.issues.len(), 2);
}

#[test]
fn ambiguity_conflict_unknown_and_invalid_scope_are_distinct_and_never_mutate() {
    let mut scope = scope();
    let mut state = RuntimeState::new();
    state
        .claims
        .insert(node(1), claim(1, scope.actor(), "clinic", Some("tomorrow")));
    state
        .claims
        .insert(node(2), claim(2, scope.actor(), "other", Some("tomorrow")));
    scope.evidence.push(node(2));
    let current = engine(&state, module());
    let ProcessingOutcome::NeedsResolution(conflict) = current
        .process_observation(&observation("reserve this"), &scope)
        .unwrap()
    else {
        panic!("conflict expected")
    };
    assert_eq!(conflict.issues[0].reason, Reason::Ambiguous);
    assert_eq!(conflict.issues[0].candidates.len(), 2);
    assert!(
        conflict
            .bindings
            .iter()
            .all(|item| item.variable.id != "target")
    );

    let ProcessingOutcome::NeedsResolution(unknown) = current
        .process_observation(&observation("unlisted"), &scope)
        .unwrap()
    else {
        panic!("unknown expected")
    };
    assert_eq!(unknown.issues[0].reason, Reason::UnknownExpression);
    let ProcessingOutcome::NeedsResolution(unparsed) = current
        .process_observation(
            &observation("reserve this except something unknown"),
            &scope,
        )
        .unwrap()
    else {
        panic!("unparsed suffix must not disappear")
    };
    assert!(
        unparsed
            .issues
            .iter()
            .any(|issue| issue.reason == Reason::UnknownExpression)
    );
    let mut ambiguous = module();
    let mut lexical = ambiguous.lexical_bindings[0].clone();
    lexical.target_term = term(3);
    ambiguous.lexical_bindings.push(lexical);
    let ProcessingOutcome::NeedsResolution(result) = engine(&state, ambiguous)
        .process_observation(&observation("reserve this"), &scope)
        .unwrap()
    else {
        panic!("ambiguity expected")
    };
    assert_eq!(result.issues[0].reason, Reason::Ambiguous);
    assert_eq!(result.issues[0].candidates.len(), 2);
    scope.subject.class = "another class".into();
    assert_eq!(
        current.process_observation(&observation("reserve this"), &scope),
        Err(ProcessingError::InvalidReference)
    );
    scope = super_scope_without_evidence();
    scope.evidence.push(node(999));
    assert_eq!(
        current.process_observation(&observation("reserve this"), &scope),
        Err(ProcessingError::InvalidReference)
    );
}

#[test]
fn time_reference_boundaries_and_malformed_evidence_fail_without_semantic_guessing() {
    let mut scope = scope();
    let mut state = RuntimeState::new();
    let mut evidence = claim(1, scope.actor(), "clinic", Some("tomorrow"));
    evidence.validity = Validity {
        valid_from: Some(99),
        valid_until: Some(101),
    };
    state.claims.insert(node(1), evidence);
    let current = engine(&state, module());
    assert!(matches!(
        current
            .process_observation(&observation("reserve this"), &scope)
            .unwrap(),
        ProcessingOutcome::Resolved(_)
    ));
    scope.at = None;
    assert_eq!(
        current.process_observation(&observation("reserve this"), &scope),
        Err(ProcessingError::InvalidInput)
    );
    scope.at = Some(101);
    let ProcessingOutcome::NeedsResolution(expired) = current
        .process_observation(&observation("reserve this"), &scope)
        .unwrap()
    else {
        panic!("expiry")
    };
    assert!(expired.bindings.is_empty());
    scope.at = Some(100);
    state.claims.get_mut(&node(1)).unwrap().arguments[0].value = Value::TermRef(term(999));
    assert_eq!(
        rejected_checkpoint(&state, module()),
        sem_lang::restoration::SemanticReferenceError::UnknownTerm(term(999))
    );
    scope.evidence.push(node(1));
    assert_eq!(
        current.process_observation(&observation("reserve this"), &scope),
        Err(ProcessingError::InvalidInput)
    );
}

fn super_scope_without_evidence() -> ObservationScope {
    let mut value = scope();
    value.evidence.clear();
    value
}

#[test]
fn relation_joins_preserve_tuples_partial_rows_and_independent_resolved_parts() {
    let mut scope = scope();
    scope.evidence = vec![node(1), node(2), node(3)];
    let mut state = RuntimeState::new();
    state
        .claims
        .insert(node(1), claim(1, scope.actor(), "clinic", Some("morning")));
    state
        .claims
        .insert(node(2), claim(2, scope.actor(), "other", Some("evening")));
    let mut eligibility = claim(3, scope.actor(), "clinic", Some("approved"));
    eligibility.predicate = term(3);
    state.claims.insert(node(3), eligibility);
    let mut joined = module();
    let first = match joined.terms[0].semantic_definition.clone().unwrap() {
        SemanticDefinition::Composition(value) => value,
        _ => unreachable!(),
    };
    joined.terms[0].semantic_definition = Some(SemanticDefinition::Composition(
        SemanticExpr::Conjunction(vec![
            first,
            SemanticExpr::Apply {
                predicate: term(3),
                arguments: vec![
                    (role(1), Box::new(SemanticExpr::Variable("target".into()))),
                    (role(2), Box::new(SemanticExpr::Literal("approved".into()))),
                ],
            },
        ]),
    ));
    let current = engine(&state, joined.clone());
    let before = current.runtime().state.semantic_state_digest().unwrap();
    let ProcessingOutcome::Resolved(result) = current
        .process_observation(&observation("reserve this"), &scope)
        .unwrap()
    else {
        panic!("the shared target must select a complete matching tuple")
    };
    assert_eq!(
        result
            .bindings
            .iter()
            .map(|b| (b.variable.id.as_str(), &b.value))
            .collect::<Vec<_>>(),
        vec![
            ("datetime", &Value::Text("morning".into())),
            ("target", &Value::Text("clinic".into()))
        ]
    );
    assert_eq!(
        before,
        current.runtime().state.semantic_state_digest().unwrap()
    );
    scope.evidence.reverse();
    assert_eq!(
        ProcessingOutcome::Resolved(result),
        current
            .process_observation(&observation("reserve this"), &scope)
            .unwrap()
    );

    // An alternative partial assertion must not borrow the other tuple's date.
    state
        .claims
        .get_mut(&node(1))
        .unwrap()
        .arguments
        .truncate(1);
    let ProcessingOutcome::NeedsResolution(partial) = engine(&state, joined.clone())
        .process_observation(&observation("reserve this"), &scope)
        .unwrap()
    else {
        panic!("partial")
    };
    assert_eq!(partial.bindings.len(), 1);
    assert_eq!(partial.bindings[0].value, Value::Text("clinic".into()));
    assert_eq!(partial.issues.len(), 1);
    assert_eq!(partial.issues[0].variable.as_deref(), Some("datetime"));
    assert_eq!(partial.issues[0].reason, Reason::MissingContext);

    // A shared variable with disjoint evidence is a conflict, not a union.
    state.claims.get_mut(&node(3)).unwrap().arguments[0].value = Value::Text("third".into());
    let ProcessingOutcome::NeedsResolution(conflict) = engine(&state, joined.clone())
        .process_observation(&observation("reserve this"), &scope)
        .unwrap()
    else {
        panic!("conflict")
    };
    assert!(conflict.bindings.is_empty());
    assert!(
        conflict
            .issues
            .iter()
            .all(|v| v.reason == Reason::ConflictingEvidence)
    );
    // An unrelated component still resolves even if the joined one conflicts.
    let Some(SemanticDefinition::Composition(SemanticExpr::Conjunction(parts))) =
        joined.terms[0].semantic_definition.as_mut()
    else {
        unreachable!()
    };
    parts.push(SemanticExpr::Apply {
        predicate: term(2),
        arguments: vec![
            (role(1), Box::new(SemanticExpr::Literal("other".into()))),
            (
                role(2),
                Box::new(SemanticExpr::Variable("independent".into())),
            ),
        ],
    });
    let ProcessingOutcome::NeedsResolution(independent) = engine(&state, joined)
        .process_observation(&observation("reserve this"), &scope)
        .unwrap()
    else {
        panic!("partial components")
    };
    assert_eq!(independent.bindings.len(), 1);
    assert_eq!(independent.bindings[0].variable.id, "independent");
    assert_eq!(independent.bindings[0].value, Value::Text("evening".into()));

    // Without a declared relationship, partial alternative rows cannot be fused.
    scope.evidence = vec![node(1), node(2)];
    state.claims.get_mut(&node(2)).unwrap().arguments.remove(0);
    let ProcessingOutcome::NeedsResolution(unjoined) = engine(&state, module())
        .process_observation(&observation("reserve this"), &scope)
        .unwrap()
    else {
        panic!("no invented tuple")
    };
    assert!(unjoined.bindings.is_empty());
    assert_eq!(unjoined.issues.len(), 2);
    assert!(
        unjoined
            .issues
            .iter()
            .all(|v| v.reason == Reason::MissingContext)
    );
}

#[test]
fn lexical_scope_filters_expired_meanings_and_other_locales_before_ambiguity() {
    let scope = scope();
    let mut state = RuntimeState::new();
    state
        .claims
        .insert(node(1), claim(1, scope.actor(), "clinic", Some("morning")));
    let mut definitions = module();
    definitions.lexical_bindings[0].valid_from = Some(100);
    definitions.lexical_bindings[0].valid_until = Some(200);
    let mut expired = definitions.lexical_bindings[0].clone();
    expired.target_term = term(3);
    expired.valid_from = Some(0);
    expired.valid_until = Some(100);
    definitions.lexical_bindings.push(expired);
    let mut other_locale = definitions.lexical_bindings[0].clone();
    other_locale.language_tag = "ja".into();
    other_locale.grammatical_condition = Some(sem_lang_linguistics::SurfacePattern::Text(
        "not matched".into(),
    ));
    definitions.lexical_bindings.push(other_locale);
    let current = engine(&state, definitions.clone());
    assert!(matches!(
        current.process_observation(&observation("reserve this"), &scope),
        Ok(ProcessingOutcome::Resolved(_))
    ));
    let mut absent_time = scope.clone();
    absent_time.at = None;
    assert_eq!(
        current.process_observation(&observation("reserve this"), &absent_time),
        Err(ProcessingError::InvalidInput)
    );
    let mut future = scope.clone();
    future.at = Some(200);
    let ProcessingOutcome::NeedsResolution(unknown) = current
        .process_observation(&observation("reserve this"), &future)
        .unwrap()
    else {
        panic!("expired definitions")
    };
    assert_eq!(unknown.issues[0].reason, Reason::UnknownExpression);
    definitions.lexical_bindings[1].valid_until = Some(101);
    let ProcessingOutcome::NeedsResolution(ambiguous) = engine(&state, definitions)
        .process_observation(&observation("reserve this"), &scope)
        .unwrap()
    else {
        panic!("overlapping meanings")
    };
    assert_eq!(ambiguous.issues[0].reason, Reason::Ambiguous);
}

#[path = "current_observation/limits.rs"]
mod limits;
#[path = "current_observation/nested.rs"]
mod nested;

#[test]
fn typed_scope_type_and_reference_constraints_use_selected_current_state() {
    use sem_lang_core::{Constraint, TypeId, Variable};
    let wanted = TypeId::new_random(1, 4);
    let other = TypeId::new_random(2, 4);
    let mut definitions = module();
    definitions.types = [wanted, other]
        .map(|id| TypeDefinition {
            id,
            label: id.to_hex(),
            super_types: vec![],
            description: String::new(),
            deprecation: None,
        })
        .to_vec();
    let body = match definitions.terms[0].semantic_definition.take().unwrap() {
        SemanticDefinition::Composition(body) => body,
        _ => unreachable!(),
    };
    let constraint = Constraint::ScopeConstraint {
        variable: Variable {
            id: "target".into(),
            expected: Some(wanted),
        },
        type_id: wanted,
    };
    definitions.terms[0].semantic_definition = Some(SemanticDefinition::Composition(
        SemanticExpr::Conjunction(vec![
            body,
            SemanticExpr::Constraint(constraint.try_into().unwrap()),
        ]),
    ));
    let mut scope = scope();
    scope.evidence.push(node(2));
    let mut state = RuntimeState::new();
    for (index, ty, date) in [(1, wanted, "morning"), (2, other, "evening")] {
        let mut evidence = claim(index, scope.actor(), "unused", Some(date));
        evidence.arguments[0].value = Value::TypeRef(ty);
        state.claims.insert(node(index), evidence);
    }
    let ProcessingOutcome::Resolved(resolved) = engine(&state, definitions.clone())
        .process_observation(&observation("reserve this"), &scope)
        .unwrap()
    else {
        panic!("type selects one tuple")
    };
    assert_eq!(resolved.bindings[0].value, Value::Text("morning".into()));
    assert_eq!(resolved.bindings[1].value, Value::TypeRef(wanted));
    let mut many = definitions.clone();
    for role in &mut many.roles {
        role.cardinality = Cardinality::OneOrMany;
    }
    let ProcessingOutcome::Resolved(multiple) = engine(&state, many)
        .process_observation(&observation("reserve this"), &scope)
        .unwrap()
    else {
        panic!("typed members do not lose their types in a many summary")
    };
    assert_eq!(
        multiple.bindings[1].value,
        Value::Sequence(vec![Value::TypeRef(wanted)])
    );
    let Some(SemanticDefinition::Composition(SemanticExpr::Conjunction(parts))) =
        definitions.terms[0].semantic_definition.as_mut()
    else {
        unreachable!()
    };
    parts.push(SemanticExpr::Constraint(
        Constraint::TypeConstraint {
            value: Value::TypeRef(wanted),
            ty: wanted,
        }
        .try_into()
        .unwrap(),
    ));
    parts.push(SemanticExpr::Constraint(
        Constraint::RelationConstraint {
            value: Value::TermRef(term(2)),
            role: role(2),
            target: Value::Text("morning".into()),
        }
        .try_into()
        .unwrap(),
    ));
    let current = engine(&state, definitions.clone());
    assert!(matches!(
        current.process_observation(&observation("reserve this"), &scope),
        Ok(ProcessingOutcome::Resolved(_))
    ));
    // Existing but unselected assertions cannot satisfy a reference condition.
    scope.evidence = vec![node(2)];
    assert!(matches!(
        current.process_observation(&observation("reserve this"), &scope),
        Ok(ProcessingOutcome::NeedsResolution(_))
    ));
    let before = current.runtime().state.semantic_state_digest().unwrap();
    let Some(SemanticDefinition::Composition(SemanticExpr::Conjunction(parts))) =
        definitions.terms[0].semantic_definition.as_mut()
    else {
        unreachable!()
    };
    parts.push(SemanticExpr::Constraint(
        Constraint::TypeConstraint {
            value: Value::TypeRef(other),
            ty: wanted,
        }
        .try_into()
        .unwrap(),
    ));
    let ProcessingOutcome::NeedsResolution(conflict) = engine(&state, definitions)
        .process_observation(&observation("reserve this"), &scope)
        .unwrap()
    else {
        panic!("false constraint is not true")
    };
    assert!(
        conflict
            .issues
            .iter()
            .any(|issue| issue.reason == Reason::ConflictingEvidence)
    );
    let issue = conflict
        .issues
        .iter()
        .find(|issue| issue.reason == Reason::ConflictingEvidence)
        .unwrap();
    assert_eq!(
        issue.constraint.as_ref().unwrap().value(),
        &Constraint::TypeConstraint {
            value: Value::TypeRef(other),
            ty: wanted
        }
    );
    let wire = serde_json::to_value(&conflict).unwrap();
    assert!(
        wire["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["constraint"]["TypeConstraint"].is_object())
    );
    assert_eq!(
        before,
        current.runtime().state.semantic_state_digest().unwrap()
    );
}

#[test]
fn conditional_claims_require_grounded_evidence_and_retain_typed_conditions() {
    use sem_lang::current::ConstraintStatus;
    use sem_lang_core::Constraint;
    let mut state = RuntimeState::new();
    let mut scope = scope();
    let condition = Constraint::RelationConstraint {
        value: Value::TermRef(term(3)),
        role: role(1),
        target: Value::Text("permitted".into()),
    };
    let mut conditional = claim(1, scope.actor(), "clinic", Some("morning"));
    conditional.constraints.push(condition.clone());
    state.claims.insert(node(1), conditional);
    let ProcessingOutcome::NeedsResolution(missing) = engine(&state, module())
        .process_observation(&observation("reserve this"), &scope)
        .unwrap()
    else {
        panic!("unknown condition")
    };
    assert!(missing.bindings.is_empty());
    assert_eq!(
        missing.claim_constraints[0].status,
        ConstraintStatus::Unresolved
    );
    assert_eq!(missing.claim_constraints[0].constraint.value(), &condition);
    assert_eq!(missing.claim_constraints[0].claim_ref, node(1));
    let mut support = claim(2, scope.actor(), "permitted", None);
    support.predicate = term(3);
    state.claims.insert(node(2), support);
    scope.evidence.push(node(2));
    let ProcessingOutcome::Resolved(resolved) = engine(&state, module())
        .process_observation(&observation("reserve this"), &scope)
        .unwrap()
    else {
        panic!("grounded condition")
    };
    assert_eq!(
        resolved.claim_constraints[0].status,
        ConstraintStatus::Satisfied
    );
    assert!(
        resolved
            .bindings
            .iter()
            .any(|binding| binding.value == Value::Text("clinic".into()))
    );
    // Circular conditional claims cannot establish each other or themselves.
    state
        .claims
        .get_mut(&node(2))
        .unwrap()
        .constraints
        .push(Constraint::RelationConstraint {
            value: Value::TermRef(term(2)),
            role: role(1),
            target: Value::Text("clinic".into()),
        });
    let current = engine(&state, module());
    let before = current
        .current_compiler()
        .unwrap()
        .semantic_revision()
        .unwrap();
    let ProcessingOutcome::NeedsResolution(cycle) = current
        .process_observation(&observation("reserve this"), &scope)
        .unwrap()
    else {
        panic!("ungrounded cycle")
    };
    assert!(cycle.bindings.is_empty());
    assert!(
        cycle
            .claim_constraints
            .iter()
            .all(|condition| condition.status == ConstraintStatus::Circular)
    );
    assert_eq!(
        current
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap(),
        before
    );
    state.claims.get_mut(&node(1)).unwrap().constraints[0] = Constraint::TypeConstraint {
        value: Value::Text("no type guess".into()),
        ty: sem_lang_core::TypeId::new_random(9, 9),
    };
    assert_eq!(
        rejected_checkpoint(&state, module()),
        sem_lang::restoration::SemanticReferenceError::UnknownType(
            sem_lang_core::TypeId::new_random(9, 9)
        )
    );
}
