use sem_lang_core::{Constraint, RoleId, TermId, TypeId, Value, Variable};
use sem_lang_linguistics::{PrimitiveSemantic, SemanticDefinition, SemanticExpander, SemanticExpr};

#[test]
fn refined_constraints_remain_typed_nested_and_reject_old_debug_strings() {
    let base = TermId::new_random(1, 1);
    let refined = TermId::new_random(2, 1);
    let nested = TermId::new_random(3, 1);
    let constraints = vec![
        Constraint::ScopeConstraint {
            variable: Variable {
                id: "target".into(),
                expected: None,
            },
            type_id: TypeId::new_random(1, 2),
        },
        Constraint::RelationConstraint {
            value: Value::TermRef(base),
            role: RoleId::new_random(1, 3),
            target: Value::Text("a; b = \"c\"".into()),
        },
    ];
    let mut expander = SemanticExpander::new();
    expander.insert(
        base,
        SemanticDefinition::Primitive(PrimitiveSemantic::Relation),
    );
    expander.insert(
        refined,
        SemanticDefinition::Refined {
            base,
            constraints: constraints.clone(),
        },
    );
    expander.insert(
        nested,
        SemanticDefinition::Composition(SemanticExpr::Conjunction(vec![SemanticExpr::Concept(
            refined,
        )])),
    );
    let expanded = expander.expand(nested).unwrap().result;
    let encoded = serde_json::to_value(&expanded).unwrap();
    let found =
        &encoded["Defined"]["expansion"]["Conjunction"][0]["Defined"]["expansion"]["Conjunction"];
    assert_eq!(
        found[1]["Constraint"],
        serde_json::to_value(&constraints[0]).unwrap()
    );
    assert_eq!(
        found[2]["Constraint"],
        serde_json::to_value(&constraints[1]).unwrap()
    );
    assert_eq!(
        serde_json::from_value::<SemanticExpr>(encoded).unwrap(),
        expanded
    );
    assert!(
        serde_json::from_str::<SemanticExpr>(
            r#"{"Constraint":"ScopeConstraint { variable: target }"}"#
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<SemanticExpr>(r#"{"Constraint":{"UnknownConstraint":{}}}"#).is_err()
    );
    let invalid = Constraint::TypeConstraint {
        value: Value::Float(f64::NAN),
        ty: TypeId::new_random(1, 2),
    };
    assert!(sem_lang_linguistics::SemanticConstraint::try_from(invalid.clone()).is_err());
    expander.insert(
        refined,
        SemanticDefinition::Refined {
            base,
            constraints: vec![invalid],
        },
    );
    assert_eq!(
        expander.expand(refined),
        Err(sem_lang_linguistics::ExpansionError::InvalidConstraint)
    );
}
