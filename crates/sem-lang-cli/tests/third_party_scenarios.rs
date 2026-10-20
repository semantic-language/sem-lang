//! One actual-process journey: formal context operations, restart/replay, limits,
//! competing writers, no read-side initialization and no retired command fallback.
use sem_lang_context::*;
use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Output, Stdio},
};

fn invoke(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sem-lang-cli"))
        .current_dir(dir)
        .args(["--format", "json"])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn apply(dir: &std::path::Path, request: &ContextRequest) -> Output {
    invoke(
        dir,
        &[
            "context",
            "apply",
            "--repo",
            "memory.json",
            "--request-json",
            &serde_json::to_string(request).unwrap(),
        ],
    )
}
fn current(dir: &std::path::Path) -> ContextEngine {
    ContextEngine::new(ContextStore::load(dir.join("memory.json")).unwrap())
}
fn request(
    engine: &ContextEngine,
    operation_id: &str,
    operation: ContextOperation,
) -> ContextRequest {
    ContextRequest {
        operation_id: operation_id.into(),
        expected_commit_revision: engine.commit_revision().unwrap(),
        expected_memory_revision: engine.memory_revision().unwrap(),
        subject_ref: ContextSubject {
            id: "person:cli".into(),
            class: "person".into(),
        },
        scope: ContextScope::CurrentRequest,
        operation,
    }
}
fn observation(reference: &str, text: &str, retention: SurfaceRetention) -> ContextOperation {
    ContextOperation::Retain {
        observation: ObservationRequest {
            input: SurfaceInput::new(reference, text),
            context: CompileContext {
                subject: ContextSubject {
                    id: "person:cli".into(),
                    class: "person".into(),
                },
                evidence: vec![],
                at: Some(100),
                locale: "en".into(),
                semantic_revision: "a".repeat(64),
                context_refs: vec![],
                context_scope: ContextScope::CurrentRequest,
                observation_evidence: vec![],
            },
            retention,
        },
    }
}
#[test]
fn formal_context_cli_replays_across_restart_and_clear_and_fences_hundred_writers() {
    let scratch = tempfile::tempdir().unwrap();
    let dir = scratch.path();
    for operation in [
        "show",
        "defaults",
        "short-term",
        "candidates",
        "history",
        "learn",
    ] {
        assert!(
            !invoke(dir, &["context", operation, "--repo", "memory.json"])
                .status
                .success()
        );
        assert_eq!(fs::read_dir(dir).unwrap().count(), 0);
    }
    success(invoke(dir, &["context", "init", "--repo", "memory.json"]));
    let initial = fs::read(dir.join("memory.json")).unwrap();
    assert!(
        !invoke(dir, &["context", "init", "--repo", "memory.json"])
            .status
            .success()
    );
    assert_eq!(fs::read(dir.join("memory.json")).unwrap(), initial);
    let text = "x".repeat(65_536);
    let retain = request(
        &current(dir),
        "operation:retain",
        observation("input:1", &text, SurfaceRetention::KeepAsEvidence),
    );
    let accepted = success(apply(dir, &retain));
    assert_eq!(accepted["changed"], true);
    assert_eq!(accepted["commit"]["committed_revision"]["sequence"], 1);
    let read = success(invoke(
        dir,
        &["context", "short-term", "--repo", "memory.json"],
    ));
    assert_eq!(read["short_term"].as_array().unwrap().len(), 1);
    assert_eq!(read["short_term"][0]["surface"], text);
    assert_eq!(read["short_term"][0]["input_ref"], "input:1");
    assert_eq!(
        read["short_term"][0]["reference"],
        accepted["observation_ref"]
    );
    assert_eq!(
        read["short_term"][0]["context"]["subject"],
        json!({"id":"person:cli","class":"person"})
    );
    let before = fs::read(dir.join("memory.json")).unwrap();
    let modified = fs::metadata(dir.join("memory.json"))
        .unwrap()
        .modified()
        .unwrap();
    for _ in 0..100 {
        assert_eq!(success(apply(dir, &retain)), accepted);
    }
    assert_eq!(fs::read(dir.join("memory.json")).unwrap(), before);
    assert_eq!(
        fs::metadata(dir.join("memory.json"))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    // Previous un-fenced mutation commands and implicit stdin paths do not exist.
    for args in [
        vec!["context", "retain"],
        vec!["context", "clear-short-term"],
        vec!["context", "confirm", "--timezone", "UTC"],
        vec!["context", "bind", "--timezone", "UTC"],
        vec!["context", "apply", "--stdin"],
        vec!["context", "apply", "--request-json", "{}"],
        vec!["packet", "validate", "missing"],
    ] {
        assert!(!invoke(dir, &args).status.success());
    }
    let too_large = request(
        &current(dir),
        "operation:large",
        observation(
            "input:large",
            &"x".repeat(65_537),
            SurfaceRetention::KeepAsEvidence,
        ),
    );
    assert!(!apply(dir, &too_large).status.success());
    let mut wrong = retain.clone();
    wrong.subject_ref.id = "wrong".into();
    assert!(!apply(dir, &wrong).status.success());
    assert_eq!(fs::read(dir.join("memory.json")).unwrap(), before);
    let clear = request(
        &current(dir),
        "operation:clear",
        ContextOperation::Clear {
            targets: vec![ContextReference::Observation(
                accepted["observation_ref"].as_str().unwrap().into(),
            )],
        },
    );
    let cleared = success(apply(dir, &clear));
    assert_eq!(cleared["changed"], true);
    let after = fs::read(dir.join("memory.json")).unwrap();
    assert_eq!(success(apply(dir, &retain)), accepted);
    assert_eq!(success(apply(dir, &clear)), cleared);
    assert_eq!(fs::read(dir.join("memory.json")).unwrap(), after);
    assert!(current(dir).short_term().is_empty());
    let discarded = request(
        &current(dir),
        "operation:discarded",
        observation(
            "input:discarded",
            "private material",
            SurfaceRetention::DiscardAfterCompile,
        ),
    );
    let discard = success(apply(dir, &discarded));
    let stored = current(dir);
    assert!(stored.short_term()[0].surface.is_none());
    assert_eq!(
        stored.short_term()[0].reference,
        discard["observation_ref"].as_str().unwrap()
    );
    assert!(
        !String::from_utf8(fs::read(dir.join("memory.json")).unwrap())
            .unwrap()
            .contains("private material")
    );
    // Same semantic image, distinct formal requests: exactly one CAS winner.
    let engine = current(dir);
    let mut children = Vec::new();
    for n in 0..100 {
        let request = request(
            &engine,
            &format!("race:{n}"),
            observation(
                "input:discarded",
                "private material",
                SurfaceRetention::DiscardAfterCompile,
            ),
        );
        children.push(
            Command::new(env!("CARGO_BIN_EXE_sem-lang-cli"))
                .current_dir(dir)
                .args([
                    "--format",
                    "json",
                    "context",
                    "apply",
                    "--repo",
                    "memory.json",
                    "--request-json",
                    &serde_json::to_string(&request).unwrap(),
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    let mut winners = 0;
    let mut rejections = 0;
    for child in children {
        let output = child.wait_with_output().unwrap();
        if output.status.success() {
            let receipt = success(output);
            assert_eq!(receipt["changed"], false);
            assert_eq!(
                receipt["output_revision"],
                engine.memory_revision().unwrap()
            );
            assert_eq!(
                receipt["commit"]["committed_revision"]["sequence"],
                engine.commit_revision().unwrap().sequence + 1
            );
            winners += 1;
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains("commit conflict") || error.contains("\"Busy\""),
                "{}",
                error
            );
            rejections += 1;
        }
    }
    assert_eq!((winners, rejections), (1, 99));
    assert_eq!(
        current(dir).memory_revision().unwrap(),
        engine.memory_revision().unwrap()
    );
    assert_eq!(fs::read_dir(dir).unwrap().count(), 2);
    fs::write(dir.join("memory.json"), b"{corrupt").unwrap();
    assert!(
        !invoke(dir, &["context", "show", "--repo", "memory.json"])
            .status
            .success()
    );
    assert!(!apply(dir, &retain).status.success());
    assert_eq!(fs::read(dir.join("memory.json")).unwrap(), b"{corrupt");
}
