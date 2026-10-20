#[path = "../../../test-support/adoption.rs"]
mod adoption_fixture;
use serde_json::{Value, json};
use std::process::Command;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for item in std::fs::read_dir(path).unwrap() {
            let path = item.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_owned();
            if path.is_dir() {
                result.insert(relative, None);
                visit(root, &path, result);
            } else {
                result.insert(relative, Some(std::fs::read(path).unwrap()));
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

#[test]
fn definition_adoption_restores_typed_meaning_after_process_exit_and_rejects_stale_writes() {
    let temporary = tempfile::Builder::new()
        .prefix("sem-lang-definitions-")
        .tempdir()
        .unwrap();
    let directory = temporary.path();
    let relation = sem_lang_core::TermId::from_declaration("independent", "Relation");
    let designation = sem_lang_core::TermId::from_declaration("independent", "Designation");
    let modules = json!([{
        "schema_version":"0.10.0", "vocabulary_id":"independent", "version":"0.10.0", "dependencies":[], "types":[], "roles":[
            {"id":"43000000000000000000000000000000","label":"subject","cardinality":"ExactlyOne","description":""},
            {"id":"43000000000000000000000000000001","label":"value","cardinality":"ExactlyOne","description":""}
        ], "lexical_bindings":[],
        "terms":[{"id":relation, "label":"Relation", "description":"A relation",
            "deprecation":null,"replacement":null,"semantic_definition":{"Primitive":"Relation"}},
            {"id":designation, "label":"Designation", "description":"A designation of a subject",
            "deprecation":null,"replacement":null,"semantic_definition":{"Composition":{"Apply":{
                "predicate":relation, "arguments":[["43000000000000000000000000000000",{"Variable":"person"}],["43000000000000000000000000000001",{"Variable":"designation"}]]}}}}]
    }]);
    let language: sem_lang_vocab::VocabularyModule =
        serde_json::from_value(modules[0].clone()).unwrap();
    let file = directory.join("module.sem");
    std::fs::write(&file, language.to_sem().unwrap()).unwrap();
    let run = |args: &[&str]| {
        let mut args: Vec<String> = args.iter().map(|s| (*s).into()).collect();
        if args[0] == "adopt" {
            let get = |key: &str| args.windows(2).find(|p| p[0] == key).map(|p| p[1].clone());
            let repo =
                sem_lang_store_fs::FsRepository::open_existing(directory.join("memory")).unwrap();
            let revision = adoption_fixture::base(
                &repo,
                "definitions-owner",
                get("--expected-revision").as_ref(),
                &get("--decision-ref").unwrap(),
            );
            drop(repo);
            args.extend([
                "--expected-commit-revision".into(),
                serde_json::to_string(&revision).unwrap(),
            ]);
        }
        Command::new(env!("CARGO_BIN_EXE_sem-lang-cli"))
            .current_dir(directory)
            .args([
                "--format",
                "json",
                "--language",
                "does-not-exist.sem",
                "definition",
            ])
            .args(args)
            .args(["--repo", "memory", "--namespace", "owner"])
            .output()
            .unwrap()
    };
    // Inspecting an absent repository must not manufacture an empty one.
    let absent = run(&["read"]);
    assert!(!absent.status.success());
    assert!(absent.stdout.is_empty());
    assert!(!directory.join("memory").exists());
    let verify_absent = Command::new(env!("CARGO_BIN_EXE_sem-lang-cli"))
        .current_dir(directory)
        .args([
            "--format",
            "json",
            "repository",
            "verify",
            "--repo",
            "memory",
        ])
        .output()
        .unwrap();
    assert!(!verify_absent.status.success());
    assert!(!directory.join("memory").exists());
    // Definition inspection is not current-state restoration. It reads only
    // its supplied declarations, without a fake repository or context writer.
    std::fs::write(directory.join("language.sem"), language.to_sem().unwrap()).unwrap();
    let inspect = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_sem-lang-cli"))
            .current_dir(directory)
            .args(["--format", "json", "--language", "language.sem", "language"])
            .args(args)
            .output()
            .unwrap()
    };
    let no_memory = snapshot(directory);
    let build = inspect(&["build"]);
    let inspection = inspect(&["inspect"]);
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(
        inspection.status.success(),
        "{}",
        String::from_utf8_lossy(&inspection.stderr)
    );
    let build: Value = serde_json::from_slice(&build.stdout).unwrap();
    let inspection: Value = serde_json::from_slice(&inspection.stdout).unwrap();
    assert_eq!(build["digest"], inspection["digest"]);
    assert_eq!(build["revision"], inspection["revision"]);
    let unknown = inspect(&[
        "explain",
        "--locale",
        "en",
        "--text",
        "not a declared meaning",
    ]);
    assert!(unknown.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&unknown.stdout).unwrap(),
        json!({
            "text":"not a declared meaning", "locale":"en", "matches":[]
        })
    );
    for args in [
        vec!["inspect", "--repo", "memory"],
        vec![
            "explain",
            "--locale",
            "en",
            "--text",
            "not a declared meaning",
            "--expand",
        ],
        vec![
            "normalize",
            "--locale",
            "en",
            "--text",
            "not a declared meaning",
        ],
    ] {
        assert!(!inspect(&args).status.success());
    }
    assert_eq!(snapshot(directory), no_memory);
    let initialized = Command::new(env!("CARGO_BIN_EXE_sem-lang-cli"))
        .current_dir(directory)
        .args(["init", "memory"])
        .output()
        .unwrap();
    assert!(initialized.status.success());
    let write = run(&[
        "adopt",
        "--file",
        "module.sem",
        "--decision-ref",
        "decision:1",
    ]);
    assert!(
        write.status.success(),
        "{}",
        String::from_utf8_lossy(&write.stderr)
    );
    let revision = serde_json::from_slice::<Value>(&write.stdout).unwrap()["revision"]
        .as_str()
        .unwrap()
        .to_owned();
    let restored = run(&["read"]);
    assert!(restored.status.success());
    let restored: Value = serde_json::from_slice(&restored.stdout).unwrap();
    assert_eq!(restored["memory"]["modules"], modules);
    assert_eq!(restored["revision"], revision);
    assert_eq!(restored["memory"]["decision_ref"], "decision:1");
    let healthy = snapshot(directory);
    assert_eq!(
        serde_json::from_slice::<Value>(&run(&["read"]).stdout).unwrap(),
        restored
    );
    let verify = || {
        Command::new(env!("CARGO_BIN_EXE_sem-lang-cli"))
            .current_dir(directory)
            .args([
                "--format",
                "json",
                "repository",
                "verify",
                "--repo",
                "memory",
            ])
            .output()
            .unwrap()
    };
    assert!(verify().status.success());
    assert_eq!(snapshot(directory), healthy);
    for part in [
        "format.json",
        "locks/repository.lock",
        "objects/sha256",
        "commits.json",
    ] {
        let from = directory.join("memory/.sem-lang").join(part);
        let saved = directory.join("removed-for-test");
        std::fs::rename(&from, &saved).unwrap();
        let damaged = snapshot(directory);
        assert!(!run(&["read"]).status.success(), "{part}");
        assert!(!verify().status.success(), "{part}");
        assert_eq!(snapshot(directory), damaged, "a read repaired {part}");
        std::fs::rename(saved, from).unwrap();
        assert_eq!(snapshot(directory), healthy);
    }
    // Exact replay uses the original Foundation receipt without rewriting storage.
    let recovered = run(&[
        "adopt",
        "--file",
        "module.sem",
        "--decision-ref",
        "decision:1",
    ]);
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&recovered.stdout).unwrap()["revision"],
        revision
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&run(&["read"]).stdout).unwrap(),
        restored
    );
    for args in [
        vec![
            "adopt",
            "--file",
            "module.sem",
            "--decision-ref",
            "decision:2",
        ],
        vec![
            "adopt",
            "--file",
            "module.sem",
            "--decision-ref",
            "",
            "--expected-revision",
            &revision,
        ],
    ] {
        assert!(!run(&args).status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&run(&["read"]).stdout).unwrap(),
            restored
        );
    }
    let updated = run(&[
        "adopt",
        "--file",
        "module.sem",
        "--decision-ref",
        "decision:2",
        "--expected-revision",
        &revision,
    ]);
    assert!(
        updated.status.success(),
        "{}",
        String::from_utf8_lossy(&updated.stderr)
    );
    let updated: Value = serde_json::from_slice(&run(&["read"]).stdout).unwrap();
    assert_eq!(updated["memory"]["previous_revision"], revision);
    assert_eq!(updated["memory"]["modules"], modules);
    // A tampered revision must be an error, not absence or a generated default.
    let head = updated["revision"].as_str().unwrap();
    let revision_file = directory.join("memory/.sem-lang/revisions").join(head);
    let mut corrupt: Value =
        serde_json::from_slice(&std::fs::read(&revision_file).unwrap()).unwrap();
    corrupt["parent"] = Value::Null;
    std::fs::write(&revision_file, serde_json::to_vec(&corrupt).unwrap()).unwrap();
    let rejected = run(&["read"]);
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
    assert!(
        !run(&[
            "adopt",
            "--file",
            "module.sem",
            "--decision-ref",
            "decision:3",
            "--expected-revision",
            head
        ])
        .status
        .success()
    );
    // The guard also cleans this test-created directory during assertion failure.
    temporary.close().unwrap();
}
