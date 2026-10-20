//! One compound owner scenario, every limit family with boundary triples.
use super::*;
use sem_lang::{CompileContext, SurfaceInput};
use sem_lang_core::{Constraint, TypeId};

fn composition(module: &mut VocabularyModule, expression: SemanticExpr) {
    module.terms[0].semantic_definition = Some(SemanticDefinition::Composition(expression));
}
fn pattern(arguments: &[(RoleId, &str)]) -> SemanticExpr {
    SemanticExpr::Apply {
        predicate: term(2),
        arguments: arguments
            .iter()
            .map(|(role, name)| (*role, Box::new(SemanticExpr::Variable((*name).into()))))
            .collect(),
    }
}

fn value_tree(visits: usize) -> Value {
    let leaves = visits - 65;
    Value::Sequence(
        (0..64)
            .map(|index| {
                Value::Sequence(
                    (0..leaves / 64 + usize::from(index < leaves % 64))
                        .map(|i| Value::Integer(i as i64))
                        .collect(),
                )
            })
            .collect(),
    )
}
fn checked(
    definitions: VocabularyModule,
    state: &RuntimeState,
    scope: &ObservationScope,
    bound: EvaluationBound,
    observed: usize,
    limit: usize,
) -> Option<sem_lang::current::SemanticResolution> {
    let current = engine(state, definitions);
    let revision = current
        .current_compiler()
        .unwrap()
        .semantic_revision()
        .unwrap();
    let result = current.process_observation(&observation("reserve this"), scope);
    assert_eq!(
        result,
        current.process_observation(&observation("reserve this"), scope)
    );
    assert_eq!(
        current
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap(),
        revision
    );
    if observed > limit {
        assert_eq!(
            result,
            Err(ProcessingError::EvaluationLimitExceeded {
                bound,
                limit,
                observed
            })
        );
        None
    } else {
        let outcome = result.unwrap_or_else(|error| panic!("{bound:?} at {observed}: {error}"));
        Some(match outcome {
            ProcessingOutcome::Resolved(result) | ProcessingOutcome::NeedsResolution(result) => {
                result
            }
        })
    }
}

#[test]
fn all_evaluation_limits_report_observed_counts_without_truncation_or_state_changes() {
    let one = scope();
    let mut state = RuntimeState::new();
    state
        .claims
        .insert(node(1), claim(1, one.actor(), "target", Some("now")));
    // Small row/role counts do not authorize multiple MiB of derived copies.
    let mut large = state.clone();
    large.claims.get_mut(&node(1)).unwrap().arguments[0].value =
        Value::Sequence((0..32).map(|_| Value::Text("x".repeat(16384))).collect());
    let current = engine(&large, module());
    let first = current.process_observation(&observation("reserve this"), &one);
    assert!(
        matches!(first, Err(ProcessingError::EvaluationLimitExceeded {
        bound: EvaluationBound::MaterializationBytes, limit: 1_048_576, observed
    }) if observed > 1_048_576)
    );
    assert_eq!(
        first,
        current.process_observation(&observation("reserve this"), &one)
    );
    for visits in [4095, 4096, 4097] {
        // One outer sequence + 64 inner sequences + leaves + datetime.
        let value = value_tree(visits - 1);
        let mut current = state.clone();
        current.claims.get_mut(&node(1)).unwrap().arguments[0].value = value;
        checked(
            module(),
            &current,
            &one,
            EvaluationBound::ValueWork,
            visits,
            4096,
        );
    }
    // The budget belongs to the request, not to each separately valid claim.
    let mut split = state.clone();
    split.claims.get_mut(&node(1)).unwrap().arguments[0].value = value_tree(2048);
    let mut second = claim(2, one.actor(), "unused", Some("later"));
    second.arguments[0].value = value_tree(2048);
    split.claims.insert(node(2), second);
    checked(
        module(),
        &split,
        &ObservationScope {
            evidence: vec![node(1), node(2)],
            ..one.clone()
        },
        EvaluationBound::ValueWork,
        4097,
        4096,
    );
    // A tiny shared graph must not expand to millions of validation visits.
    let mut shared = state.clone();
    for index in 0..20 {
        shared.graph.nodes.insert(
            node(index + 100),
            sem_lang_core::GraphNode {
                id: node(index + 100),
                created_by: None,
                expr: sem_lang_core::Expr::Apply(sem_lang_core::Apply {
                    operator: term(2),
                    arguments: [role(1), role(2)]
                        .into_iter()
                        .map(|role| Argument {
                            role: Some(role),
                            value: if index == 19 {
                                Value::Text("leaf".into())
                            } else {
                                Value::Node(node(index + 101))
                            },
                        })
                        .collect(),
                }),
            },
        );
    }
    shared.claims.get_mut(&node(1)).unwrap().arguments[0].value = Value::Node(node(100));
    checked(
        module(),
        &shared,
        &one,
        EvaluationBound::ValueWork,
        4097,
        4096,
    );
    for n in [63, 64, 65] {
        // Argument count: both current assertions and adopted expressions.
        let mut definitions = module();
        definitions.roles = (0..n)
            .map(|i| RoleDefinition {
                id: role(i as u64 + 1),
                label: format!("r{i}"),
                cardinality: Cardinality::ExactlyOne,
                description: String::new(),
            })
            .collect();
        let arguments = (0..n)
            .map(|i| {
                (
                    role(i as u64 + 1),
                    Box::new(SemanticExpr::Variable(format!("v{i}"))),
                )
            })
            .collect();
        composition(
            &mut definitions,
            SemanticExpr::Apply {
                predicate: term(2),
                arguments,
            },
        );
        let mut many = state.clone();
        many.claims.get_mut(&node(1)).unwrap().arguments = (0..n)
            .map(|i| Argument {
                role: Some(role(i as u64 + 1)),
                value: Value::Integer(i as i64),
            })
            .collect();
        if let Some(result) = checked(
            definitions.clone(),
            &many,
            &one,
            EvaluationBound::Arguments,
            n,
            64,
        ) {
            assert!(result.issues.is_empty());
            assert_eq!(result.bindings.len(), n);
            for i in 0..n {
                assert!(result.bindings.iter().any(
                    |b| b.variable.id == format!("v{i}") && b.value == Value::Integer(i as i64)
                ));
            }
        }
        if n == 65 {
            checked(
                definitions,
                &RuntimeState::new(),
                &ObservationScope {
                    evidence: vec![],
                    ..one.clone()
                },
                EvaluationBound::Arguments,
                n,
                64,
            );
        }

        // Pattern count and total variable occurrences are distinct budgets.
        let mut definitions = module();
        composition(
            &mut definitions,
            SemanticExpr::Conjunction(vec![pattern(&[(role(1), "x")]); n]),
        );
        if let Some(result) = checked(definitions, &state, &one, EvaluationBound::Patterns, n, 64) {
            assert!(result.issues.is_empty());
            assert_eq!(result.bindings.len(), 1);
            assert_eq!(result.bindings[0].value, Value::Text("target".into()));
        }
        let mut expressions = vec![pattern(&[(role(1), "x"), (role(2), "y")]); n / 2];
        if n % 2 == 1 {
            expressions.push(pattern(&[(role(1), "x")]));
        }
        let mut definitions = module();
        composition(&mut definitions, SemanticExpr::Conjunction(expressions));
        if let Some(result) = checked(
            definitions,
            &state,
            &one,
            EvaluationBound::VariableOccurrences,
            n,
            64,
        ) {
            assert!(result.issues.is_empty());
            assert_eq!(result.bindings.len(), 2);
        }

        // Both sequence and map values retain exact contents at the boundary.
        for value in [
            Value::Sequence((0..n).map(|i| Value::Integer(i as i64)).collect()),
            Value::Map(
                (0..n)
                    .map(|i| (format!("k{i}"), Value::Integer(i as i64)))
                    .collect(),
            ),
        ] {
            let mut values = state.clone();
            values.claims.get_mut(&node(1)).unwrap().arguments[0].value = value.clone();
            if let Some(result) =
                checked(module(), &values, &one, EvaluationBound::ValueItems, n, 64)
            {
                assert!(result.issues.is_empty());
                assert!(
                    result
                        .bindings
                        .iter()
                        .any(|b| b.variable.id == "target" && b.value == value)
                );
            }
        }

        // Current Claim references have their own bound, not InvalidInput.
        let mut claims = RuntimeState::new();
        let mut selected = one.clone();
        selected.evidence.clear();
        for i in 1..=n {
            claims.claims.insert(
                node(i as u64),
                claim(i as u64, one.actor(), "target", Some("now")),
            );
            selected.evidence.push(node(i as u64));
        }
        if let Some(result) = checked(
            module(),
            &claims,
            &selected,
            EvaluationBound::EvidenceReferences,
            n,
            64,
        ) {
            assert!(result.issues.is_empty());
            assert_eq!(
                result.evidence_refs,
                selected
                    .evidence
                    .iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
            );
        }

        // Reference depth in an explicit type graph, not primitive-type guessing.
        let mut definitions = module();
        let types: Vec<_> = (0..n).map(|i| TypeId::new_random(i as u64, 4)).collect();
        definitions.types = types
            .iter()
            .enumerate()
            .map(|(i, id)| TypeDefinition {
                id: *id,
                label: format!("type{i}"),
                description: String::new(),
                deprecation: None,
                super_types: types.get(i + 1).copied().into_iter().collect(),
            })
            .collect();
        let condition = Constraint::TypeConstraint {
            value: Value::TypeRef(types[0]),
            ty: types[n - 1],
        };
        composition(
            &mut definitions,
            SemanticExpr::Constraint(condition.clone().try_into().unwrap()),
        );
        if let Some(result) = checked(
            definitions,
            &state,
            &one,
            EvaluationBound::TypeTraversal,
            n,
            64,
        ) {
            assert!(result.issues.is_empty());
        }

        // The same constraint bound applies to expressions and current Claims.
        let mut definitions = module();
        let ty = TypeId::new_random(1, 4);
        definitions.types.push(TypeDefinition {
            id: ty,
            label: "declared".into(),
            super_types: vec![],
            description: String::new(),
            deprecation: None,
        });
        let condition = Constraint::TypeConstraint {
            value: Value::TypeRef(ty),
            ty,
        };
        composition(
            &mut definitions,
            SemanticExpr::Conjunction(vec![
                SemanticExpr::Constraint(
                    condition.clone().try_into().unwrap()
                );
                n
            ]),
        );
        if let Some(result) = checked(
            definitions.clone(),
            &state,
            &one,
            EvaluationBound::Constraints,
            n,
            64,
        ) {
            assert!(result.issues.is_empty());
        }
        composition(&mut definitions, pattern(&[(role(1), "x")]));
        let mut claims = state.clone();
        claims.claims.get_mut(&node(1)).unwrap().constraints = vec![condition; n];
        if let Some(result) = checked(
            definitions,
            &claims,
            &one,
            EvaluationBound::Constraints,
            n,
            64,
        ) {
            assert!(result.issues.is_empty());
            assert_eq!(result.claim_constraints.len(), n);
        }

        let mut memory = ContextStore::default();
        for i in 0..n {
            memory
                .persistent_defaults
                .push(sem_lang_context::PersistentContextualDefault {
                    id: format!("default:{i}"),
                    subject: one.subject.clone(),
                    kind: sem_lang_context::ContextKind::TimeZone,
                    value: "UTC".into(),
                    scope: sem_lang_context::ContextScope::PersistentAgentDefault,
                    validity: sem_lang_context::ContextValidity::default(),
                    evidence: vec![],
                    revision: format!("revision:{i}"),
                    promotion_ref: format!("decision:{i}"),
                });
        }
        let current = engine_with_context(&state, module(), &memory);
        let input = CompileContext {
            subject: one.subject.clone(),
            evidence: one.evidence.clone(),
            observation_evidence: vec![],
            context_refs: (0..n)
                .map(|i| sem_lang_context::ContextReference::Default(format!("default:{i}")))
                .collect(),
            context_scope: sem_lang_context::ContextScope::CurrentRequest,
            locale: "en".into(),
            at: one.at,
            semantic_revision: current
                .current_compiler()
                .unwrap()
                .semantic_revision()
                .unwrap(),
        };
        let output =
            current.compile_surface(&SurfaceInput::new("ref:limits", "reserve this"), &input);
        if n > 64 {
            assert_eq!(
                output,
                Err(ProcessingError::EvaluationLimitExceeded {
                    bound: EvaluationBound::ContextReferences,
                    limit: 64,
                    observed: n
                })
            );
        } else {
            let ProcessingOutcome::Resolved(result) = output.unwrap() else {
                panic!("context refs")
            };
            assert_eq!(result.context_refs.len(), n);
        }
    }
    for n in [31, 32, 33] {
        let mut values = state.clone();
        let value = (0..n).fold(Value::Text("leaf".into()), |v, _| Value::Sequence(vec![v]));
        values.claims.get_mut(&node(1)).unwrap().arguments[0].value = value.clone();
        if let Some(result) = checked(
            module(),
            &values,
            &one,
            EvaluationBound::ExpressionDepth,
            n,
            32,
        ) {
            assert!(result.bindings.iter().any(|b| b.value == value));
        }
    }
    for (left, right) in [(7, 9), (8, 8), (5, 13)] {
        let mut definitions = module();
        composition(
            &mut definitions,
            SemanticExpr::Conjunction(vec![
                pattern(&[(role(1), "x"), (role(2), "y")]),
                pattern(&[(role(1), "y"), (role(2), "z")]),
            ]),
        );
        let mut state = RuntimeState::new();
        let mut selected = one.clone();
        selected.evidence.clear();
        for i in 0..left + right {
            let (target, time) = if i < left {
                (format!("left{i}"), "middle".into())
            } else {
                ("middle".into(), format!("right{i}"))
            };
            state.claims.insert(
                node(i as u64 + 1),
                claim(i as u64 + 1, one.actor(), &target, Some(&time)),
            );
            selected.evidence.push(node(i as u64 + 1));
        }
        if let Some(result) = checked(
            definitions,
            &state,
            &selected,
            EvaluationBound::Alternatives,
            left * right,
            64,
        ) {
            assert!(result.issues.iter().any(|i| i.reason == Reason::Ambiguous));
            assert!(
                result
                    .bindings
                    .iter()
                    .any(|b| b.variable.id == "y" && b.value == Value::Text("middle".into()))
            );
        }
    }
    // 63 + 63*64 = 4095; 64 + 64*63 = 4096; the next run stops at 4097.
    for (first_guard, second_guard, observed) in [
        (true, false, 4095),
        (false, true, 4096),
        (false, false, 4097),
    ] {
        let mut definitions = module();
        definitions.roles.push(RoleDefinition {
            id: role(3),
            label: "include".into(),
            cardinality: Cardinality::ExactlyOne,
            description: String::new(),
        });
        let patterns = [first_guard, second_guard].map(|guard| {
            let mut expr = pattern(&[(role(1), "x")]);
            if guard && let SemanticExpr::Apply { arguments, .. } = &mut expr {
                arguments.push((role(3), Box::new(SemanticExpr::Literal("yes".into()))));
            }
            expr
        });
        composition(
            &mut definitions,
            SemanticExpr::Conjunction(patterns.to_vec()),
        );
        let mut state = RuntimeState::new();
        let mut selected = one.clone();
        selected.evidence.clear();
        for i in 0..64 {
            let mut claim = claim(i + 1, one.actor(), &format!("target{i}"), Some("now"));
            claim.arguments.push(Argument {
                role: Some(role(3)),
                value: Value::Text(if i == 0 { "no" } else { "yes" }.into()),
            });
            state.claims.insert(node(i + 1), claim);
            selected.evidence.push(node(i + 1));
        }
        if let Some(result) = checked(
            definitions,
            &state,
            &selected,
            EvaluationBound::PairWork,
            observed,
            4096,
        ) {
            assert_eq!(result.issues.len(), 1);
            assert_eq!(result.issues[0].reason, Reason::Ambiguous);
            assert_eq!(result.issues[0].candidates.len(), 63);
        }
    }
}
