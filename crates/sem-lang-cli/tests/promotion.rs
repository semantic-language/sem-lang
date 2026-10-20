#[path = "../../../test-support/context.rs"]
mod context_fixture;
use context_fixture::ContextFixture;
use std::process::{Command, Output};

#[test]
fn hundred_cli_processes_cannot_adopt_one_candidate_more_than_once() {
    competing_writers(false);
    competing_writers(true);
}

fn competing_writers(no_change: bool) {
    use sem_lang_context::*;
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("memory.json");
    let mut engine = ContextEngine::new(ContextStore::default());
    engine.record_timezone_confirmation("UTC");
    engine.record_timezone_confirmation("UTC");
    let candidate = engine.induce_defaults().remove(0);
    if no_change {
        let seed = PromotionRequest {
            subject_ref: ContextSubject {
                id: "person:one".into(),
                class: "person".into(),
            },
            candidate_ref: candidate.reference().unwrap(),
            expected_memory_revision: engine.memory_revision().unwrap(),
            expected_commit_revision: engine.commit_revision().unwrap(),
            decision_ref: "decision:seed".into(),
            operation: PromotionOperation::PromoteLongTerm,
        };
        engine.promote(candidate.clone(), &seed).unwrap();
    }
    engine.store.save(&path).unwrap();
    let candidate_path = scratch.path().join("candidate.json");
    std::fs::write(&candidate_path, serde_json::to_vec(&candidate).unwrap()).unwrap();
    let mut children = Vec::new();
    for number in 0..100 {
        let request = PromotionRequest {
            subject_ref: ContextSubject {
                id: "person:one".into(),
                class: "person".into(),
            },
            candidate_ref: candidate.reference().unwrap(),
            expected_memory_revision: engine.memory_revision().unwrap(),
            expected_commit_revision: engine.commit_revision().unwrap(),
            decision_ref: format!("decision:{number}"),
            operation: PromotionOperation::PromoteLongTerm,
        };
        let request_path = scratch.path().join(format!("request-{number}.json"));
        std::fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_sem-lang-cli"))
            .args(["--format", "json", "context", "promote", "--candidate-file"])
            .arg(&candidate_path)
            .arg("--request")
            .arg(&request_path)
            .arg("--repo")
            .arg(&path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        children.push((request, child));
    }
    let mut successes = Vec::new();
    let outputs: Vec<_> = children
        .into_iter()
        .map(|(request, child)| (request, child.wait_with_output()))
        .collect();
    for (request, output) in outputs {
        let output = output.unwrap();
        if output.status.success() {
            let receipt: PromotionReceipt = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(receipt.request.candidate_ref, request.candidate_ref);
            assert_eq!(receipt.request.subject_ref, request.subject_ref);
            assert_eq!(receipt.changed, !no_change);
            successes.push(request.decision_ref);
        } else {
            let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
            if error["error"]["Busy"]["wait_millis"] == 500 {
                // A bounded lock refusal is an explicit non-acceptance. Under
                // heavy process contention it is as valid as losing the CAS;
                // neither outcome can duplicate the promotion.
            } else if no_change {
                assert!(
                    error["error"]["CommitConflict"]["Stale"].is_object(),
                    "{error}"
                );
            } else {
                assert_eq!(error["error"], serde_json::json!("RevisionConflict"));
            }
        }
    }
    assert_eq!(successes.len(), 1);
    let state = ContextStore::load(&path).unwrap();
    assert_eq!(state.persistent_defaults.len(), 1);
    assert_eq!(
        state.persistent_defaults[0].promotion_ref,
        if no_change {
            "decision:seed"
        } else {
            &successes[0]
        }
    );
    assert_eq!(state.persistent_defaults[0].value, "UTC");
    assert_eq!(
        ContextEngine::new(state)
            .commit_revision()
            .unwrap()
            .sequence,
        if no_change { 4 } else { 3 }
    );
}

#[test]
fn candidate_review_is_read_only_and_promotion_requires_a_recorded_decision() {
    let scratch = tempfile::tempdir().unwrap();
    let repo = scratch.path().join("context.json");
    let run = |args: &[&str]| -> Output {
        Command::new(env!("CARGO_BIN_EXE_sem-lang-cli"))
            .args(["--format", "json"])
            .args(args)
            .arg("--repo")
            .arg(&repo)
            .output()
            .unwrap()
    };
    assert!(run(&["context", "init"]).status.success());
    for n in 0..2 {
        let engine = sem_lang_context::ContextEngine::new(
            sem_lang_context::ContextStore::load(&repo).unwrap(),
        );
        let request = context_fixture::formal(
            &engine,
            sem_lang_context::ContextSubject {
                id: "person:cli".into(),
                class: "person".into(),
            },
            sem_lang_context::ContextScope::PersistentAgentDefault,
            sem_lang_context::ContextOperation::ObserveTimezone {
                value: "UTC".into(),
                evidence_ref: format!("evidence:{n}"),
            },
        );
        let output = run(&[
            "context",
            "apply",
            "--request-json",
            &serde_json::to_string(&request).unwrap(),
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let before = std::fs::read(&repo).unwrap();
    let review = run(&["learning", "run"]);
    assert!(review.status.success());
    let report: serde_json::Value = serde_json::from_slice(&review.stdout).unwrap();
    assert_eq!(report["promoted"], 0);
    assert_eq!(report["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(std::fs::read(&repo).unwrap(), before);
    for args in [
        vec!["learning", "run", "--policy", "conservative-auto"],
        vec!["context", "promote", "0"],
        vec!["context", "promote", "0", "--promotion-ref", " "],
        vec![
            "context",
            "bind",
            "--timezone",
            "UTC",
            "--scope",
            "persistent",
        ],
    ] {
        assert!(!run(&args).status.success());
        assert_eq!(std::fs::read(&repo).unwrap(), before);
    }
    let engine =
        sem_lang_context::ContextEngine::new(sem_lang_context::ContextStore::load(&repo).unwrap());
    let candidate = engine.induce_defaults().remove(0);
    let request = sem_lang_context::PromotionRequest {
        subject_ref: sem_lang_context::ContextSubject {
            id: "person:cli".into(),
            class: "person".into(),
        },
        candidate_ref: candidate.reference().unwrap(),
        expected_memory_revision: engine.memory_revision().unwrap(),
        expected_commit_revision: engine.commit_revision().unwrap(),
        decision_ref: "decision:cli:1".into(),
        operation: sem_lang_context::PromotionOperation::PromoteLongTerm,
    };
    let candidate_path = scratch.path().join("candidate.json");
    let request_path = scratch.path().join("request.json");
    std::fs::write(&candidate_path, serde_json::to_vec(&candidate).unwrap()).unwrap();
    std::fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let arguments = [
        "context",
        "promote",
        "--candidate-file",
        candidate_path.to_str().unwrap(),
        "--request",
        request_path.to_str().unwrap(),
    ];
    let promoted = run(&arguments);
    assert!(
        promoted.status.success(),
        "{}",
        String::from_utf8_lossy(&promoted.stderr)
    );
    let store: serde_json::Value = serde_json::from_slice(&std::fs::read(&repo).unwrap()).unwrap();
    assert_eq!(
        store["persistent_defaults"][0]["promotion_ref"],
        "decision:cli:1"
    );
    assert_eq!(store["revisions"][0]["defaults"][0]["value"], "UTC");
    assert_eq!(
        store["persistent_defaults"][0]["subject"]["id"],
        "person:cli"
    );
    let retry = run(&arguments);
    assert!(retry.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&retry.stdout).unwrap(),
        serde_json::from_slice::<serde_json::Value>(&promoted.stdout).unwrap()
    );
    let before_review = std::fs::read(&repo).unwrap();
    assert!(run(&["learning", "run"]).status.success());
    assert_eq!(std::fs::read(&repo).unwrap(), before_review);
    // The checkpoint's persistent lock inode belongs to this isolated fixture.
    // Do not unlink a live repository lock merely to satisfy test cleanup.
    scratch.close().unwrap();
}
