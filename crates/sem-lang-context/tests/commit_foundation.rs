//! Owner semantics plus the registry-provided commit foundation, not a second CAS.
#[path = "../../../test-support/context.rs"]
mod context_fixture;
use context_fixture::ContextFixture;
use sem_lang_context::{
    ContextEngine, ContextStore, ContextSubject, PromotionOperation, PromotionRequest,
};

mod observation_fixture;

#[test]
fn persisted_owner_rejects_before_write_and_clears_short_term_without_losing_long_term() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.json");
    ContextStore::create(&path).unwrap();
    assert!(ContextStore::create(&path).is_err());
    let (candidate, request, receipt) = context_fixture::update(&path, |engine| {
        engine.record_timezone_confirmation("UTC");
        engine.record_timezone_confirmation("UTC");
        let candidate = engine.induce_defaults().remove(0);
        let request = PromotionRequest {
            subject_ref: ContextSubject {
                id: "person:alice".into(),
                class: "person".into(),
            },
            candidate_ref: candidate.reference()?,
            expected_memory_revision: engine.memory_revision()?,
            expected_commit_revision: engine.commit_revision()?,
            decision_ref: "decision:durable".into(),
            operation: PromotionOperation::PromoteLongTerm,
        };
        let receipt = engine.promote(candidate.clone(), &request)?;
        Ok((candidate, request, receipt))
    })
    .unwrap();
    let before = std::fs::read(&path).unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    for _ in 0..100 {
        let replay =
            context_fixture::update(&path, |engine| engine.promote(candidate.clone(), &request))
                .unwrap();
        assert_eq!(
            serde_json::to_value(replay).unwrap(),
            serde_json::to_value(&receipt).unwrap()
        );
        let mut invalid = request.clone();
        invalid.decision_ref = "different-decision".into();
        invalid.subject_ref.id.clear();
        assert!(
            context_fixture::update(&path, |engine| engine.promote(candidate.clone(), &invalid))
                .is_err()
        );
    }
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified
    );
    context_fixture::update(&path, |engine| {
        engine.retain_observation(observation_fixture::request(
            "observation:retained",
            "private retained surface",
        ))?;
        let mut discard =
            observation_fixture::request("observation:discarded", "private discarded surface");
        discard.retention = sem_lang_context::SurfaceRetention::DiscardAfterCompile;
        engine.retain_observation(discard)?;
        Ok(())
    })
    .unwrap();
    let stored = ContextStore::load(&path).unwrap();
    assert_eq!(stored.short_term.len(), 2);
    assert!(
        !String::from_utf8_lossy(&std::fs::read(&path).unwrap())
            .contains("private discarded surface")
    );
    context_fixture::update(&path, |engine| {
        engine.clear_short_term();
        Ok(())
    })
    .unwrap();
    let restored = ContextStore::load(&path).unwrap();
    assert!(restored.short_term.is_empty());
    assert_eq!(restored.persistent_defaults[0].value, "UTC");
    assert!(
        !String::from_utf8_lossy(&std::fs::read(&path).unwrap())
            .contains("private retained surface")
    );
    let audit = dir.path().join("audit.txt");
    std::fs::write(&audit, "untrusted explanation").unwrap();
    let exact = std::fs::read(&path).unwrap();
    std::fs::remove_file(audit).unwrap();
    assert_eq!(
        ContextStore::load(&path).unwrap().persistent_defaults[0].value,
        "UTC"
    );
    assert_eq!(std::fs::read(&path).unwrap(), exact);
    let mut corrupted: serde_json::Value = serde_json::from_slice(&exact).unwrap();
    corrupted["commit_state"]["unexpected"] = serde_json::json!(true);
    std::fs::write(&path, serde_json::to_vec(&corrupted).unwrap()).unwrap();
    assert!(ContextStore::load(&path).is_err());
    assert!(context_fixture::update(&path, |_| Ok(())).is_err());
}

#[test]
fn promotion_replay_after_correction_returns_original_receipt_without_mutating_current_state() {
    let mut engine = ContextEngine::new(ContextStore::default());
    engine.record_timezone_confirmation("UTC");
    engine.record_timezone_confirmation("UTC");
    let candidate = engine.induce_defaults().remove(0);
    let request = PromotionRequest {
        subject_ref: ContextSubject {
            id: "person:alice".into(),
            class: "person".into(),
        },
        candidate_ref: candidate.reference().unwrap(),
        expected_memory_revision: engine.memory_revision().unwrap(),
        expected_commit_revision: engine.commit_revision().unwrap(),
        decision_ref: "decision:first".into(),
        operation: PromotionOperation::PromoteLongTerm,
    };
    let receipt = engine.promote(candidate.clone(), &request).unwrap();
    assert_eq!(engine.commit_revision().unwrap().sequence, 3);
    let mut corrected = candidate.clone();
    corrected.value = "Asia/Tokyo".into();
    let correction = PromotionRequest {
        candidate_ref: corrected.reference().unwrap(),
        expected_memory_revision: engine.memory_revision().unwrap(),
        expected_commit_revision: engine.commit_revision().unwrap(),
        decision_ref: "decision:correction".into(),
        operation: PromotionOperation::CorrectLongTerm {
            assertion_ref: engine.store.persistent_defaults[0].id.clone(),
        },
        ..request.clone()
    };
    engine.promote(corrected.clone(), &correction).unwrap();
    let bytes = serde_json::to_vec(&engine.store).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("promotion_receipts"));
    let mut restarted = ContextEngine::new(serde_json::from_slice(&bytes).unwrap());
    for _ in 0..100 {
        assert_eq!(
            serde_json::to_value(restarted.promote(candidate.clone(), &request).unwrap()).unwrap(),
            serde_json::to_value(&receipt).unwrap()
        );
    }
    assert_eq!(serde_json::to_vec(&restarted.store).unwrap(), bytes);
    let no_change = PromotionRequest {
        expected_memory_revision: restarted.memory_revision().unwrap(),
        expected_commit_revision: restarted.commit_revision().unwrap(),
        decision_ref: "decision:same-value".into(),
        operation: PromotionOperation::PromoteLongTerm,
        ..correction.clone()
    };
    let unchanged = restarted.promote(corrected.clone(), &no_change).unwrap();
    assert!(!unchanged.changed);
    assert_eq!(unchanged.input_revision, unchanged.output_revision);
    assert_eq!(restarted.commit_revision().unwrap().sequence, 5);
    let before = serde_json::to_vec(&restarted.store).unwrap();
    let mut stale = no_change.clone();
    stale.decision_ref = "decision:stale".into();
    assert!(restarted.promote(corrected.clone(), &stale).is_err());
    let mut reused = no_change.clone();
    reused.operation = PromotionOperation::Reject;
    assert!(restarted.promote(corrected.clone(), &reused).is_err());
    let mut invalid = no_change.clone();
    invalid.decision_ref = "decision:invalid".into();
    invalid.expected_commit_revision = restarted.commit_revision().unwrap();
    invalid.subject_ref.id.clear();
    assert!(restarted.promote(corrected, &invalid).is_err());
    assert_eq!(serde_json::to_vec(&restarted.store).unwrap(), before);
}
