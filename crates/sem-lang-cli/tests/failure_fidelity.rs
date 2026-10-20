use sem_lang_context::*;
use serde_json::{Value, json};

#[test]
fn actual_cli_preserves_owner_failures_and_never_reclassifies_them_as_missing_meaning() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("context.json");
    let path = path.to_str().unwrap();
    let run = |request: &ContextRequest| {
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_sem-lang-cli"))
            .args([
                "--format",
                "json",
                "context",
                "apply",
                "--repo",
                path,
                "--request-json",
                &serde_json::to_string(request).unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        serde_json::from_slice::<Value>(&result.stderr).unwrap()
    };
    let initial = ContextEngine::new(ContextStore::default());
    let make = |id: &str| ContextRequest {
        operation_id: id.into(),
        expected_commit_revision: initial.commit_revision().unwrap(),
        expected_memory_revision: initial.memory_revision().unwrap(),
        subject_ref: ContextSubject {
            id: "person:1".into(),
            class: "person".into(),
        },
        scope: ContextScope::Session,
        operation: ContextOperation::BindTimezone {
            value: "Asia/Tokyo".into(),
        },
    };
    let request = make("test:first");
    let missing = ContextStore::publish(path, &request).unwrap_err();
    assert!(matches!(
        missing,
        ContextError::RepositoryUnavailable { .. }
    ));
    assert_eq!(
        run(&request)["error"],
        serde_json::to_value(&missing).unwrap()
    );
    assert!(!std::path::Path::new(path).exists());

    ContextStore::create(path).unwrap();
    ContextStore::publish(path, &request).unwrap();
    let engine = ContextEngine::new(ContextStore::load(path).unwrap());
    let mut stale = make("test:stale");
    stale.expected_memory_revision = engine.memory_revision().unwrap();
    let mut semantic = make("test:semantic");
    semantic.expected_commit_revision = engine.commit_revision().unwrap();
    let mut reused = request.clone();
    reused.subject_ref.id = "another".into();
    let mut invalid = make("");
    invalid.expected_memory_revision = engine.memory_revision().unwrap();
    invalid.expected_commit_revision = engine.commit_revision().unwrap();
    let before = std::fs::read(path).unwrap();
    let mtime = std::fs::metadata(path).unwrap().modified().unwrap();
    for request in [&stale, &semantic, &reused, &invalid] {
        let owner =
            serde_json::to_value(ContextStore::publish(path, request).unwrap_err()).unwrap();
        for _ in 0..100 {
            assert_eq!(run(request)["error"], owner);
        }
    }
    assert_eq!(run(&semantic)["error"], json!("RevisionConflict"));
    assert!(run(&stale)["error"]["CommitConflict"]["Stale"].is_object());
    assert!(run(&reused)["error"]["CommitConflict"]["OperationReuse"].is_object());
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert_eq!(std::fs::metadata(path).unwrap().modified().unwrap(), mtime);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::metadata(path).unwrap().permissions();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let error = ContextStore::publish(path, &stale).unwrap_err();
        assert!(
            matches!(&error, ContextError::RepositoryUnavailable { kind, .. } if kind == "PermissionDenied")
        );
        assert_eq!(run(&stale)["error"], serde_json::to_value(error).unwrap());
        std::fs::set_permissions(path, permissions).unwrap();
    }
    let mut corrupt: Value = serde_json::from_slice(&before).unwrap();
    corrupt["commit_state"]["format"] = json!(999);
    std::fs::write(path, serde_json::to_vec(&corrupt).unwrap()).unwrap();
    let owner = ContextStore::publish(path, &stale).unwrap_err();
    assert!(
        matches!(owner, ContextError::Commit(CommitFailure::Corrupt)),
        "{owner:?}"
    );
    assert_eq!(run(&stale)["error"], serde_json::to_value(owner).unwrap());
}
