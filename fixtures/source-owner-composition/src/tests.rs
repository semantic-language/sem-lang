//! Cross-owner acceptance through the actual pinned producer, without a model or Grant.
use super::*;
use sem_lang_frontend::current::structural::{SourceShape, StructuralInput};
use serde_json::{Value, json};

fn owner() -> Owner {
    let mut owner: Owner = serde_json::from_value(json!({
        "plan": {
            "declaration": {
                "action": {
                    "operation_id": "source/replace", "target": "source:selected",
                    "contract_revision": "pending", "availability": {"state": "available"},
                    "expected_revision_required": true,
                    "input_schema": {"type": "object", "fields": {
                        "opaque-field": {"type": "string", "max_length": 80}
                    }, "required": ["opaque-field"]}
                },
                "fields": {"opaque-field": {"label": "Contact", "sensitive": false}}
            },
            "source_revision": "original:one",
            "source": {
                "reference": "source:selected", "pointer": "/selected",
                "document": {"selected": {"marker": "@", "value": "before"},
                    "audit": {"value": "history-only"}, "profile": {"value": "profile-only"}},
                "budget": {"max_nodes": 128, "max_depth": 32, "max_bytes": 65536}
            },
            "bindings": {"opaque-field": "/value"}
        },
        "context": {
            "subject": {"id": "fixture-owner", "class": "person"},
            "evidence": [], "at": null, "locale": "en",
            "semantic_revision": "a".repeat(64), "context_refs": [],
            "context_scope": "CurrentRequest", "observation_evidence": []
        },
        "budget": {"max_nodes": 128, "max_edges": 256, "max_candidates": 128,
            "max_steps": 32768, "max_depth": 32, "max_bytes": 65536}
    }))
    .unwrap();
    owner.plan.declaration.action.contract_revision = owner.plan.contract_ref().unwrap();
    owner
}

fn submission(owner: &Owner) -> Value {
    json!({"operation_id": "source/replace", "target": "source:selected",
        "contract_revision": owner.plan.declaration.action.contract_revision,
        "expected_revision": "original:one", "request_reference": "request:one",
        "input": {"opaque-field": "after"}})
}

fn bind(owner: &Owner, request: Value) -> Result<StructuralInput, &'static str> {
    let tree = owner.plan.bind(&serde_json::from_value(request).unwrap())?;
    source_to_structural(&tree, owner.context.clone(), owner.budget.clone())
}

fn image(owner: &Owner) -> Value {
    json!({"plan": owner.plan, "context": owner.context, "budget": owner.budget})
}

#[test]
fn actual_producer_binding_selects_current_source_without_mutating_or_adopting_it() {
    let owner = owner();
    let before = image(&owner);
    let result = bind(&owner, submission(&owner)).unwrap();
    assert_eq!(result.reference, "source:selected");
    assert_eq!(result.context, owner.context);
    assert!(result.constraints.is_empty());
    let encoded = serde_json::to_value(&result).unwrap();
    let values: Vec<_> = encoded["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|node| node["shape"]["Value"]["Text"].as_str())
        .collect();
    assert!(values.contains(&"after"));
    assert!(values.contains(&"@"));
    for excluded in ["before", "history-only", "profile-only"] {
        assert!(!values.contains(&excluded));
    }
    assert!(matches!(result.nodes[0].shape, SourceShape::Record(_)));
    assert_eq!(image(&owner), before);
}

#[test]
fn producer_rejects_stale_substituted_or_undeclared_requests_before_composition() {
    let owner = owner();
    let before = image(&owner);
    for (key, value) in [
        ("expected_revision", json!("original:stale")),
        ("contract_revision", json!("source-input:substituted")),
        ("target", json!("source:other")),
        ("operation_id", json!("source/delete")),
        ("input", json!({"Contact": "after"})),
        ("input", json!({"/value": "after"})),
        (
            "input",
            json!({"opaque-field": "after", "roleRef": "forged"}),
        ),
        ("input", json!({"opaque-field": "x".repeat(81)})),
    ] {
        let mut request = submission(&owner);
        request[key] = value;
        assert!(bind(&owner, request).is_err(), "accepted substituted {key}");
        assert_eq!(image(&owner), before);
    }
}

#[test]
fn sealed_source_and_binding_changes_require_a_new_contract_identity() {
    for mutation in ["source", "binding", "revision"] {
        let mut owner = owner();
        let request = submission(&owner);
        match mutation {
            "source" => owner.plan.source.document["selected"]["marker"] = json!("changed"),
            "binding" => {
                owner
                    .plan
                    .bindings
                    .insert("opaque-field".into(), "/marker".into());
            }
            _ => owner.plan.source_revision = "original:two".into(),
        }
        assert!(
            bind(&owner, request).is_err(),
            "accepted changed {mutation}"
        );
    }
}

#[test]
fn exact_serialized_owner_reloads_and_replays_without_implicit_persistence() {
    let owner = owner();
    let bytes = serde_json::to_vec(&image(&owner)).unwrap();
    let restored: Owner = serde_json::from_slice(&bytes).unwrap();
    restored.plan.validate().unwrap();
    let original = bind(&owner, submission(&owner)).unwrap();
    for _ in 0..3 {
        assert_eq!(bind(&restored, submission(&restored)).unwrap(), original);
        assert_eq!(serde_json::to_vec(&image(&restored)).unwrap(), bytes);
    }
    let mut old_request = submission(&owner);
    old_request["expected_revision"] = json!("original:stale");
    assert!(bind(&restored, old_request).is_err());
}

#[test]
fn language_owner_rejects_supported_producer_shapes_it_cannot_interpret() {
    for (value, expected) in [
        (json!(["one", "two"]), "source-sequence-unsupported"),
        (json!(1.5), "source-number-unsupported"),
    ] {
        let mut owner = owner();
        owner.plan.source.document["selected"]["fixed"] = value;
        owner.plan.declaration.action.contract_revision = owner.plan.contract_ref().unwrap();
        owner.plan.validate().unwrap();
        assert_eq!(bind(&owner, submission(&owner)).unwrap_err(), expected);
    }
}

#[test]
fn producer_and_consumer_budgets_are_independently_enforced() {
    let mut consumer_limited = owner();
    consumer_limited.budget.max_nodes = 1;
    assert_eq!(
        bind(&consumer_limited, submission(&consumer_limited)).unwrap_err(),
        "source-semantic-budget-invalid"
    );
    let mut producer_limited = owner();
    producer_limited.plan.source.budget.max_nodes = 1;
    producer_limited.plan.declaration.action.contract_revision =
        producer_limited.plan.contract_ref().unwrap();
    assert!(bind(&producer_limited, submission(&producer_limited)).is_err());
}

#[test]
fn generic_producer_does_not_accept_language_context_or_execution_authority() {
    let owner = owner();
    let declaration = serde_json::to_string(&owner.plan.declaration).unwrap();
    for private in [
        "fixture-owner",
        "semantic_revision",
        "/selected",
        "history-only",
    ] {
        assert!(!declaration.contains(private));
    }
    for field in ["context", "grant", "credentials"] {
        let mut plan = serde_json::to_value(&owner.plan).unwrap();
        plan[field] = json!({"forged": true});
        assert!(serde_json::from_value::<hat_source_curator::input::OwnerInput>(plan).is_err());
    }
}
