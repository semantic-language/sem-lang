use sem_lang_core::{Expr, PacketId, Resolution, RuntimeState, SemanticPacket, Value};
use sem_lang_runtime::{PacketApplyError, PacketReducer, PacketRequest};

fn request(state: &RuntimeState, id: &str) -> PacketRequest {
    PacketRequest {
        operation_id: id.into(),
        expected_commit_revision: PacketReducer::commit_revision(state).unwrap(),
        expected_semantic_digest: state.semantic_state_digest().unwrap(),
    }
}
fn packet(seed: u64, value: &str) -> SemanticPacket {
    SemanticPacket {
        id: PacketId::new_random(seed, 1),
        conversation_id: None,
        sender: None,
        recipient: None,
        cause: None,
        correlation: None,
        act: None,
        content: Expr::Value(Value::Text(value.into())),
        context_delta: None,
        vocabulary_manifest: None,
        resolution: Resolution::Resolved,
        pragmatics: vec![],
        provenance: None,
    }
}

#[test]
fn digest_covers_expression_and_edge_targets_and_rejects_non_finite_state() {
    use sem_lang_core::{GraphEdge, RoleId};
    let mut state = PacketReducer::reduce(
        &RuntimeState::new(),
        &packet(1, "first"),
        &request(&RuntimeState::new(), "operation:1"),
    )
    .unwrap()
    .into_state();
    let before = state.semantic_state_digest().unwrap();
    state.graph.nodes.values_mut().next().unwrap().expr = Expr::Value(Value::Integer(1));
    assert_ne!(before, state.semantic_state_digest().unwrap());
    let id = *state.graph.nodes.keys().next().unwrap();
    state.graph.add_edge(GraphEdge {
        source: id,
        role: RoleId::new_random(1, 1),
        target: Value::Integer(1),
    });
    let before = state.semantic_state_digest().unwrap();
    state.graph.edges[0].target = Value::Integer(2);
    assert_ne!(before, state.semantic_state_digest().unwrap());
    state.graph.edges[0].target = Value::Float(f64::NAN);
    assert!(state.semantic_state_digest().is_err());
}

#[test]
fn contextual_updates_are_atomic_and_equal_expressions_keep_subject_boundaries() {
    use sem_lang_core::{Claim, ContextDelta, NodeId, TermId, Validity};
    use std::collections::BTreeMap;
    let claim = Claim {
        id: NodeId::new_random(1, 2),
        actor: None,
        predicate: TermId::new_random(1, 3),
        arguments: vec![],
        constraints: vec![],
        validity: Validity {
            valid_from: None,
            valid_until: None,
        },
    };
    let mut p = packet(1, "observation");
    p.sender = Some("subject-1".into());
    p.context_delta = Some(ContextDelta {
        added_claims: vec![claim.clone()],
        removed_claims: vec![],
        annotations: BTreeMap::from([("basis".into(), "source-1".into())]),
    });
    let state = PacketReducer::reduce(
        &RuntimeState::new(),
        &p,
        &request(&RuntimeState::new(), "operation:1"),
    )
    .unwrap()
    .into_state();
    assert_eq!(state.claims.get(&claim.id), Some(&claim));
    assert_eq!(
        state.annotations.get("basis").map(String::as_str),
        Some("source-1")
    );
    let mut remove = packet(2, "observation");
    remove.sender = p.sender.clone();
    remove.context_delta = Some(ContextDelta {
        added_claims: vec![],
        removed_claims: vec![claim.clone()],
        annotations: BTreeMap::new(),
    });
    let removed = PacketReducer::reduce(&state, &remove, &request(&state, "operation:2"))
        .unwrap()
        .into_state();
    assert!(removed.claims.is_empty());
    assert_ne!(
        removed.semantic_state_digest().unwrap(),
        state.semantic_state_digest().unwrap()
    );
    assert_eq!(removed.graph.nodes.len(), 1);
    remove.id = PacketId::new_random(3, 1);
    assert!(matches!(
        PacketReducer::reduce(&removed, &remove, &request(&removed, "operation:3")),
        Err(PacketApplyError::ClaimConflict)
    ));
    assert_eq!(
        PacketReducer::commit_revision(&removed).unwrap().sequence,
        2
    );
    let mut other = packet(4, "observation");
    other.sender = Some("subject-2".into());
    let separate = PacketReducer::reduce(&removed, &other, &request(&removed, "operation:4"))
        .unwrap()
        .into_state();
    assert_eq!(separate.graph.nodes.len(), 2);
}
