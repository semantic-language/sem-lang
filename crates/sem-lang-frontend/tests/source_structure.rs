use sem_lang_frontend::{
    CompileContext,
    current::{
        source_structure::source_to_structural,
        structural::{SourceShape, StructuralBudget},
    },
};
use serde_json::json;
use zixcel_json_structure::{Decomposition, StructureBudget, decompose};
fn context() -> CompileContext {
    serde_json::from_value(
        json!({"subject":{"id":"fixture-owner","class":"person"},"evidence":[],
        "at":null,"locale":"en","semantic_revision":"a".repeat(64),"context_refs":[],
        "context_scope":"CurrentRequest","observation_evidence":[]}),
    )
    .unwrap()
}
fn source(document: serde_json::Value) -> zixcel_json_structure::SourceTree {
    decompose(&Decomposition {
        reference: "source:fixture".into(),
        document,
        pointer: String::new(),
        budget: StructureBudget::default(),
    })
    .unwrap()
}
#[test]
fn context_is_supplied_only_by_the_language_owner_and_labels_do_not_become_meaning() {
    let tree = source(json!({"pretend-type":"unresolved-value","flag":true}));
    let selected = context();
    let result =
        source_to_structural(&tree, selected.clone(), StructuralBudget::default()).unwrap();
    assert_eq!(result.context, selected);
    assert!(result.constraints.is_empty());
    assert!(matches!(result.nodes[0].shape, SourceShape::Record(_)));
    assert_eq!(result.nodes[1].locator, "/flag");
    assert_eq!(result.reference, tree.reference);
    assert!(!serde_json::to_string(&result).unwrap().contains("TermRef"));
}
#[test]
fn language_rejects_shapes_that_the_generic_source_layer_preserves() {
    let ordered = source(json!([1, 2]));
    assert_eq!(
        source_to_structural(&ordered, context(), StructuralBudget::default()).unwrap_err(),
        "source-sequence-unsupported"
    );
    let precise = source(serde_json::from_str("123456789012345678901234567890.123").unwrap());
    assert_eq!(
        source_to_structural(&precise, context(), StructuralBudget::default()).unwrap_err(),
        "source-number-unsupported"
    );
}
#[test]
fn invalid_tree_or_explicit_budget_is_not_silently_repaired() {
    let mut tree = source(json!({"flag":true}));
    tree.nodes[1].locator = "/unrelated".into();
    assert!(source_to_structural(&tree, context(), StructuralBudget::default()).is_err());
    let tree = source(json!({"flag":true}));
    let budget = StructuralBudget {
        max_nodes: 1,
        ..StructuralBudget::default()
    };
    assert!(source_to_structural(&tree, context(), budget).is_err());
}
