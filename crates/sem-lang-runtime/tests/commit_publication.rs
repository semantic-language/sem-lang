use sem_lang_core::{Expr, PacketId, Resolution, RuntimeState, SemanticPacket, Value};
use sem_lang_runtime::{PacketReducer, PacketRequest};
fn packet(n: u64, text: &str) -> SemanticPacket {
    SemanticPacket {
        id: PacketId::new_random(n, 1),
        conversation_id: None,
        sender: None,
        recipient: None,
        cause: None,
        correlation: None,
        act: None,
        content: Expr::Value(Value::Text(text.into())),
        context_delta: None,
        vocabulary_manifest: None,
        resolution: Resolution::Resolved,
        pragmatics: vec![],
        provenance: None,
    }
}
fn request(state: &RuntimeState, id: &str) -> PacketRequest {
    PacketRequest {
        operation_id: id.into(),
        expected_commit_revision: PacketReducer::commit_revision(state).unwrap(),
        expected_semantic_digest: state.semantic_state_digest().unwrap(),
    }
}
#[test]
fn reducer_replay_returns_exact_result_after_newer_commit_without_repeating_meaning() {
    let initial = RuntimeState::new();
    let p = packet(1, "first");
    let r = request(&initial, "operation:1");
    let transition = PacketReducer::reduce(&initial, &p, &r).unwrap();
    let first = transition.receipt().clone();
    let state = transition.into_state();
    let p2 = packet(2, "second");
    let r2 = request(&state, "operation:2");
    let state = PacketReducer::reduce(&state, &p2, &r2)
        .unwrap()
        .into_state();
    let bytes = serde_json::to_vec(&state).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("\"receipts\""));
    let restored: RuntimeState = serde_json::from_slice(&bytes).unwrap();
    for _ in 0..100 {
        let retry = PacketReducer::reduce(&restored, &p, &r).unwrap();
        assert_eq!(retry.receipt(), &first);
        assert_eq!(serde_json::to_vec(&retry.into_state()).unwrap(), bytes);
        let mut reused = r.clone();
        reused.expected_semantic_digest = restored.semantic_state_digest().unwrap();
        assert!(PacketReducer::reduce(&restored, &p, &reused).is_err());
        let mut stale = request(&restored, "stale");
        stale.expected_commit_revision = r.expected_commit_revision.clone();
        assert!(PacketReducer::reduce(&restored, &packet(3, "third"), &stale).is_err());
    }
    assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
    assert_eq!(restored.packets.len(), 2);
    assert_eq!(
        PacketReducer::commit_revision(&restored).unwrap().sequence,
        2
    );
}

#[test]
fn reducer_separates_semantic_no_change_from_commit_acceptance_and_rejects_invalid_images() {
    use sem_lang_core::ApplicationOutcome;
    use sem_lang_runtime::PacketApplyError;
    use zixcel_revision::{Conflict, Failure};
    let initial = RuntimeState::new();
    let initial_bytes = serde_json::to_vec(&initial).unwrap();
    for resolution in [Resolution::Pending, Resolution::Unknown, Resolution::Failed] {
        let mut p = packet(1, "not accepted");
        p.resolution = resolution;
        assert!(
            matches!(PacketReducer::reduce(&initial, &p, &request(&initial, "unresolved")),
            Err(PacketApplyError::Unresolved(actual)) if actual == resolution)
        );
    }
    assert_eq!(serde_json::to_vec(&initial).unwrap(), initial_bytes);
    let state = PacketReducer::reduce(&initial, &packet(1, "same"), &request(&initial, "first"))
        .unwrap()
        .into_state();
    let r = request(&state, "same");
    let next = PacketReducer::reduce(&state, &packet(2, "same"), &r).unwrap();
    assert_eq!(next.receipt().outcome, ApplicationOutcome::NoChange);
    assert_eq!(next.receipt().input_digest, next.receipt().output_digest);
    assert_eq!(next.receipt().commit.committed_revision.sequence, 2);
    let next = next.into_state();
    let expected = serde_json::to_vec(&next).unwrap();
    for _ in 0..100 {
        let stale = PacketRequest {
            operation_id: "stale".into(),
            ..r.clone()
        };
        assert!(matches!(
            PacketReducer::reduce(&next, &packet(3, "new"), &stale),
            Err(PacketApplyError::CommitConflict(Conflict::Stale { .. }))
        ));
        let mut wrong = request(&next, "semantic");
        wrong.expected_semantic_digest = initial.semantic_state_digest().unwrap();
        assert!(matches!(
            PacketReducer::reduce(&next, &packet(3, "new"), &wrong),
            Err(PacketApplyError::StaleSemanticInput)
        ));
        let mut forged = serde_json::to_value(&next).unwrap();
        forged["commit_state"]["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<RuntimeState>(forged).is_err());
    }
    assert_eq!(serde_json::to_vec(&next).unwrap(), expected);
    // Foundation validation is used on decoded state, including non-structural corruption.
    let mut corrupt = serde_json::to_value(&next).unwrap();
    let objects = corrupt["commit_state"]["heads"].as_object_mut().unwrap();
    objects.values_mut().next().unwrap()["sequence"] = serde_json::json!(99);
    let corrupt: RuntimeState = serde_json::from_value(corrupt).unwrap();
    assert!(matches!(
        PacketReducer::commit_revision(&corrupt),
        Err(PacketApplyError::CommitFailure(Failure::Corrupt))
    ));
}
