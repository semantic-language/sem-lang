use super::*;
use sem_lang::current::ConstraintStatus;
use sem_lang_core::{Constraint, TypeId};

fn requires(predicate: TermId, target: &str) -> Constraint {
    Constraint::RelationConstraint {
        value: Value::TermRef(predicate),
        role: role(1),
        target: Value::Text(target.into()),
    }
}
fn outcome(
    state: &RuntimeState,
    definitions: VocabularyModule,
    selected: &ObservationScope,
) -> sem_lang::current::SemanticResolution {
    let current = engine(state, definitions);
    let before = current
        .current_compiler()
        .unwrap()
        .semantic_revision()
        .unwrap();
    let output = current
        .process_observation(&observation("reserve this"), selected)
        .unwrap();
    assert_eq!(
        before,
        current
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap()
    );
    match output {
        ProcessingOutcome::Resolved(r) | ProcessingOutcome::NeedsResolution(r) => r,
    }
}

#[test]
fn conditional_current_state_keeps_unknown_conflict_cycles_types_and_scopes_distinct() {
    explicit_claim_binding_scope();
    let mut selected = scope();
    let mut state = RuntimeState::new();
    let mut dependent = claim(1, selected.actor(), "clinic", Some("morning"));
    dependent.constraints.push(requires(term(3), "yes"));
    state.claims.insert(node(1), dependent);
    let unknown = outcome(&state, module(), &selected);
    assert_eq!(
        unknown.claim_constraints[0].status,
        ConstraintStatus::Unresolved
    );
    let mut proof = claim(2, selected.actor(), "yes", None);
    proof.predicate = term(3);
    state.claims.insert(node(2), proof);
    // Existing but unselected evidence is not current evidence.
    let unselected = outcome(&state, module(), &selected);
    assert_eq!(unknown.claim_constraints, unselected.claim_constraints);
    assert_eq!(unknown.bindings, unselected.bindings);
    assert_eq!(unknown.issues, unselected.issues);
    assert_ne!(unknown.semantic_revision, unselected.semantic_revision);
    selected.evidence.push(node(2));
    let satisfied = outcome(&state, module(), &selected);
    assert_eq!(
        satisfied.claim_constraints[0].status,
        ConstraintStatus::Satisfied
    );
    assert!(satisfied.issues.is_empty());

    let mut contrary = claim(3, selected.actor(), "no", None);
    contrary.predicate = term(3);
    state.claims.insert(node(3), contrary);
    selected.evidence.push(node(3));
    let conflict = outcome(&state, module(), &selected);
    assert_eq!(
        conflict.claim_constraints[0].status,
        ConstraintStatus::Conflicting
    );
    assert!(conflict.bindings.is_empty());
    assert!(
        conflict
            .issues
            .iter()
            .any(|i| i.reason == Reason::ConflictingEvidence)
    );
    selected.evidence.reverse();
    assert_eq!(conflict, outcome(&state, module(), &selected));
    let mut many = module();
    many.roles[0].cardinality = Cardinality::OneOrMany;
    assert_eq!(
        outcome(&state, many, &selected).claim_constraints[0].status,
        ConstraintStatus::Satisfied
    );

    // A conditional alternative cannot temporarily become certain and then
    // leave a stale accepted dependent behind when its contrary is discovered.
    state
        .claims
        .get_mut(&node(3))
        .unwrap()
        .constraints
        .push(requires(term(2), "clinic"));
    let blocked = outcome(&state, module(), &selected);
    assert!(
        !blocked
            .claim_constraints
            .iter()
            .any(|c| c.status == ConstraintStatus::Satisfied)
    );
    assert!(blocked.bindings.is_empty());

    // A truly unrelated unknown remains distinct from a dependency cycle.
    selected.evidence = vec![node(1), node(2)];
    state
        .claims
        .get_mut(&node(2))
        .unwrap()
        .constraints
        .push(requires(term(2), "clinic"));
    let cycle = outcome(&state, module(), &selected);
    assert!(
        cycle
            .claim_constraints
            .iter()
            .all(|c| c.status == ConstraintStatus::Circular)
    );
    assert!(
        cycle
            .issues
            .iter()
            .any(|i| i.reason == Reason::CircularDependency)
    );
    state.claims.get_mut(&node(2)).unwrap().constraints = vec![requires(term(2), "unprovided")];
    let missing = outcome(&state, module(), &selected);
    assert!(
        missing
            .claim_constraints
            .iter()
            .all(|c| c.status == ConstraintStatus::Unresolved)
    );

    // A typed condition can be demonstrably false without interpreting text.
    let a = TypeId::new_random(81, 1);
    let b = TypeId::new_random(82, 1);
    let mut definitions = module();
    definitions.types = [a, b]
        .into_iter()
        .map(|id| TypeDefinition {
            id,
            label: id.to_string(),
            super_types: vec![],
            description: String::new(),
            deprecation: None,
        })
        .collect();
    state.claims.get_mut(&node(1)).unwrap().constraints = vec![Constraint::TypeConstraint {
        value: Value::TypeRef(a),
        ty: b,
    }];
    selected.evidence = vec![node(1)];
    assert_eq!(
        outcome(&state, definitions.clone(), &selected).claim_constraints[0].status,
        ConstraintStatus::Unsatisfied
    );
    state.claims.get_mut(&node(1)).unwrap().constraints = vec![Constraint::TypeConstraint {
        value: Value::Text("a".into()),
        ty: b,
    }];
    assert_eq!(
        outcome(&state, definitions.clone(), &selected).claim_constraints[0].status,
        ConstraintStatus::Unresolved
    );

    selected.evidence = vec![node(1), node(2)];
    selected.subject.id = "another owner".into();
    assert_eq!(
        engine(&state, definitions).process_observation(&observation("reserve this"), &selected),
        Err(ProcessingError::InvalidReference)
    );
}

fn explicit_claim_binding_scope() {
    use sem_lang_core::{Binding, Expr, GraphNode, Variable};
    let a = TypeId::new_random(501, 1);
    let b = TypeId::new_random(502, 1);
    let mut definitions = module();
    definitions.types = [a, b]
        .into_iter()
        .map(|id| TypeDefinition {
            id,
            label: "same label".into(),
            super_types: vec![],
            description: String::new(),
            deprecation: None,
        })
        .collect();
    let selected = scope();
    let variable = Variable {
        id: "explicit-variable".into(),
        expected: None,
    };
    let mut root = claim(1, selected.actor(), "clinic", Some("morning"));
    root.constraints.push(Constraint::ScopeConstraint {
        variable: variable.clone(),
        type_id: a,
    });
    let mut state = RuntimeState::new();
    state.claims.insert(node(1), root);
    state.graph.nodes.insert(
        node(900),
        GraphNode {
            id: node(900),
            created_by: None,
            expr: Expr::Binding(Binding {
                variable: variable.clone(),
                value: Value::TypeRef(a),
            }),
        },
    );
    assert_eq!(
        outcome(&state, definitions.clone(), &selected).claim_constraints[0].status,
        ConstraintStatus::Unresolved,
        "an unreferenced binding cannot fill the Claim's scope"
    );
    state.claims.get_mut(&node(1)).unwrap().arguments[0].value = Value::Node(node(900));
    for (value, expected) in [
        (Value::TypeRef(a), ConstraintStatus::Satisfied),
        (Value::TypeRef(b), ConstraintStatus::Unsatisfied),
        (
            Value::Text("no type inference".into()),
            ConstraintStatus::Unresolved,
        ),
    ] {
        let Expr::Binding(binding) = &mut state.graph.nodes.get_mut(&node(900)).unwrap().expr
        else {
            unreachable!()
        };
        binding.value = value;
        let result = outcome(&state, definitions.clone(), &selected);
        assert_eq!(result.claim_constraints[0].status, expected);
        assert_eq!(
            result.bindings.is_empty(),
            expected != ConstraintStatus::Satisfied
        );
    }
    let Expr::Binding(binding) = &mut state.graph.nodes.get_mut(&node(900)).unwrap().expr else {
        unreachable!()
    };
    binding.value = Value::TypeRef(a);
    state.graph.nodes.insert(
        node(901),
        GraphNode {
            id: node(901),
            created_by: None,
            expr: Expr::Binding(Binding {
                variable,
                value: Value::TypeRef(b),
            }),
        },
    );
    state.claims.get_mut(&node(1)).unwrap().arguments[1].value = Value::Node(node(901));
    assert_eq!(
        outcome(&state, definitions.clone(), &selected).claim_constraints[0].status,
        ConstraintStatus::Conflicting
    );
    state.graph.nodes.remove(&node(901));
    assert_eq!(
        engine(&state, definitions).process_observation(&observation("reserve this"), &selected),
        Err(ProcessingError::InvalidReference)
    );
}
