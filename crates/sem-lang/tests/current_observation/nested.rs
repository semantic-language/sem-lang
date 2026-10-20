use super::*;
use sem_lang_core::{Apply, Expr, GraphNode};

fn application(predicate: TermId, arguments: &[(RoleId, &str)]) -> SemanticExpr {
    SemanticExpr::Apply {
        predicate,
        arguments: arguments
            .iter()
            .map(|(role, variable)| (*role, Box::new(SemanticExpr::Variable((*variable).into()))))
            .collect(),
    }
}
fn result(engine: &Engine, scope: &ObservationScope) -> sem_lang::current::SemanticResolution {
    let output = engine
        .process_observation(&observation("reserve this"), scope)
        .unwrap();
    match output {
        ProcessingOutcome::Resolved(result) | ProcessingOutcome::NeedsResolution(result) => result,
    }
}
fn value<'a>(result: &'a sem_lang::current::SemanticResolution, name: &str) -> Option<&'a Value> {
    result
        .bindings
        .iter()
        .find(|b| b.variable.id == name)
        .map(|b| &b.value)
}

#[test]
fn nested_current_references_preserve_outer_values_inner_residuals_and_tuple_correlations() {
    ground_structure();
    let mut definitions = module();
    definitions.roles.push(RoleDefinition {
        id: role(3),
        label: "inner".into(),
        cardinality: Cardinality::ExactlyOne,
        description: String::new(),
    });
    let mut outer = application(term(2), &[(role(1), "outer_a"), (role(2), "outer_b")]);
    if let SemanticExpr::Apply { arguments, .. } = &mut outer {
        arguments.push((
            role(3),
            Box::new(application(term(3), &[(role(1), "x"), (role(2), "y")])),
        ));
    }
    definitions.terms[0].semantic_definition = Some(SemanticDefinition::Composition(outer.clone()));
    let mut selected = scope();
    let mut state = RuntimeState::new();
    let mut root = claim(1, selected.actor(), "a", Some("b"));
    root.arguments.push(Argument {
        role: Some(role(3)),
        value: Value::Node(node(99)),
    });
    state.claims.insert(node(1), root);
    state.graph.nodes.insert(
        node(99),
        GraphNode {
            id: node(99),
            created_by: None,
            expr: Expr::Apply(Apply {
                operator: term(3),
                arguments: vec![Argument {
                    role: Some(role(1)),
                    value: Value::Text("inner-a".into()),
                }],
            }),
        },
    );
    // An unrelated current node cannot fill the selected inner reference.
    state.graph.nodes.insert(
        node(100),
        GraphNode {
            id: node(100),
            created_by: None,
            expr: Expr::Apply(Apply {
                operator: term(3),
                arguments: vec![
                    Argument {
                        role: Some(role(1)),
                        value: Value::Text("inner-b".into()),
                    },
                    Argument {
                        role: Some(role(2)),
                        value: Value::Text("time-b".into()),
                    },
                ],
            }),
        },
    );
    let partial = result(&engine(&state, definitions.clone()), &selected);
    assert_eq!(value(&partial, "outer_a"), Some(&Value::Text("a".into())));
    assert_eq!(value(&partial, "outer_b"), Some(&Value::Text("b".into())));
    assert_eq!(value(&partial, "x"), Some(&Value::Text("inner-a".into())));
    assert_eq!(value(&partial, "y"), None);
    assert_eq!(partial.issues.len(), 1);
    assert_eq!(partial.issues[0].variable.as_deref(), Some("y"));
    assert_eq!(partial.issues[0].reason, Reason::MissingContext);
    assert!(partial.expression.as_ref().unwrap().contains_term(term(3)));

    let Expr::Apply(inner) = &mut state.graph.nodes.get_mut(&node(99)).unwrap().expr else {
        unreachable!()
    };
    inner.arguments.push(Argument {
        role: Some(role(2)),
        value: Value::Text("time-a".into()),
    });
    let complete = result(&engine(&state, definitions.clone()), &selected);
    assert!(complete.issues.is_empty());
    assert_eq!(value(&complete, "y"), Some(&Value::Text("time-a".into())));
    assert_ne!(complete.semantic_revision, partial.semantic_revision);
    assert_eq!(
        complete,
        result(&engine(&state, definitions.clone()), &selected)
    );

    // A typed residual remains unknown both inline and through its exact node.
    let hole = sem_lang_core::TypedHole {
        id: "unresolved-inner".into(),
        expected: None,
        surface: None,
    };
    for through_node in [false, true] {
        let mut partial_state = state.clone();
        if through_node {
            partial_state.graph.nodes.get_mut(&node(99)).unwrap().expr =
                Expr::TypedHole(hole.clone());
        } else {
            partial_state.claims.get_mut(&node(1)).unwrap().arguments[2].value =
                Value::TypedHole(hole.clone());
        }
        let residual = result(&engine(&partial_state, definitions.clone()), &selected);
        assert_eq!(value(&residual, "outer_a"), Some(&Value::Text("a".into())));
        assert_eq!(value(&residual, "outer_b"), Some(&Value::Text("b".into())));
        assert_eq!(residual.issues.len(), 2);
        assert!(
            residual
                .issues
                .iter()
                .all(|issue| issue.reason == Reason::MissingContext)
        );
        assert_eq!(value(&residual, "x"), None);
        assert_eq!(value(&residual, "y"), None);
    }

    let mut second = claim(2, selected.actor(), "c", Some("d"));
    second.arguments.push(Argument {
        role: Some(role(3)),
        value: Value::Node(node(100)),
    });
    state.claims.insert(node(2), second);
    let mut filter = claim(3, selected.actor(), "inner-b", None);
    filter.predicate = term(3);
    state.claims.insert(node(3), filter);
    selected.evidence.extend([node(2), node(3)]);
    definitions.terms[0].semantic_definition = Some(SemanticDefinition::Composition(
        SemanticExpr::Conjunction(vec![outer.clone(), application(term(3), &[(role(1), "x")])]),
    ));
    let filtered = result(&engine(&state, definitions.clone()), &selected);
    assert!(filtered.issues.is_empty());
    assert_eq!(value(&filtered, "outer_a"), Some(&Value::Text("c".into())));
    assert_eq!(value(&filtered, "outer_b"), Some(&Value::Text("d".into())));
    assert_eq!(value(&filtered, "x"), Some(&Value::Text("inner-b".into())));
    assert_eq!(value(&filtered, "y"), Some(&Value::Text("time-b".into())));
    selected.evidence.reverse();
    assert_eq!(
        filtered,
        result(&engine(&state, definitions.clone()), &selected)
    );

    for corrupt in ["missing", "wrong-id", "dangling-child"] {
        let mut bad = state.clone();
        match corrupt {
            "missing" => {
                bad.graph.nodes.remove(&node(99));
            }
            "wrong-id" => bad.graph.nodes.get_mut(&node(99)).unwrap().id = node(500),
            _ => {
                let Expr::Apply(inner) = &mut bad.graph.nodes.get_mut(&node(99)).unwrap().expr
                else {
                    unreachable!()
                };
                inner.arguments[0].value = Value::Node(node(500));
            }
        }
        assert_eq!(
            engine(&bad, definitions.clone())
                .process_observation(&observation("reserve this"), &selected),
            Err(ProcessingError::InvalidReference),
            "{corrupt}"
        );
    }
    // A selected cyclic value is bounded failure, not an infinite traversal or
    // a reason to search other nodes for an apparently compatible value.
    let Expr::Apply(inner) = &mut state.graph.nodes.get_mut(&node(99)).unwrap().expr else {
        unreachable!()
    };
    inner.arguments[0].value = Value::Node(node(99));
    assert_eq!(
        engine(&state, definitions).process_observation(&observation("reserve this"), &selected),
        Err(ProcessingError::EvaluationLimitExceeded {
            bound: EvaluationBound::ExpressionDepth,
            limit: 32,
            observed: 33
        })
    );
}

fn ground_structure() {
    let mut selected = scope();
    let mut definitions = module();
    let ground = SemanticExpr::Apply {
        predicate: term(2),
        arguments: vec![(role(1), Box::new(SemanticExpr::Literal("clinic".into())))],
    };
    definitions.terms[0].semantic_definition =
        Some(SemanticDefinition::Composition(ground.clone()));
    let mut state = RuntimeState::new();
    let mut unrelated = claim(1, selected.actor(), "clinic", None);
    unrelated.predicate = term(3);
    state.claims.insert(node(1), unrelated);
    assert!(matches!(
        engine(&state, definitions.clone())
            .process_observation(&observation("reserve this"), &selected)
            .unwrap(),
        ProcessingOutcome::NeedsResolution(_)
    ));
    state
        .claims
        .insert(node(1), claim(1, selected.actor(), "clinic", None));
    assert!(matches!(
        engine(&state, definitions.clone())
            .process_observation(&observation("reserve this"), &selected)
            .unwrap(),
        ProcessingOutcome::Resolved(_)
    ));
    definitions.terms[0].semantic_definition =
        Some(SemanticDefinition::Composition(SemanticExpr::Apply {
            predicate: term(2),
            arguments: vec![
                (role(1), Box::new(SemanticExpr::Variable("outer".into()))),
                (role(2), Box::new(ground)),
            ],
        }));
    let unresolved = result(&engine(&state, definitions.clone()), &selected);
    assert_eq!(
        value(&unresolved, "outer"),
        Some(&Value::Text("clinic".into()))
    );
    assert!(
        unresolved
            .issues
            .iter()
            .any(|i| i.reason == Reason::MissingContext)
    );
    state.graph.nodes.insert(
        node(99),
        GraphNode {
            id: node(99),
            created_by: None,
            expr: Expr::Apply(Apply {
                operator: term(2),
                arguments: vec![Argument {
                    role: Some(role(1)),
                    value: Value::Text("clinic".into()),
                }],
            }),
        },
    );
    let mut complete = state.claims[&node(1)].clone();
    complete.id = node(2);
    complete.arguments.push(Argument {
        role: Some(role(2)),
        value: Value::Node(node(99)),
    });
    state.claims.insert(node(2), complete);
    let current = engine(&state, definitions);
    assert!(
        matches!(
            current
                .process_observation(&observation("reserve this"), &selected)
                .unwrap(),
            ProcessingOutcome::NeedsResolution(_)
        ),
        "unselected complete evidence must not leak into the request"
    );
    selected.evidence.push(node(2));
    let resolved = current
        .process_observation(&observation("reserve this"), &selected)
        .unwrap();
    assert!(
        matches!(&resolved, ProcessingOutcome::Resolved(_)),
        "complete evidence of the same tuple satisfies a partial alternative"
    );
    assert_eq!(
        resolved,
        current
            .process_observation(&observation("reserve this"), &selected)
            .unwrap()
    );
}
