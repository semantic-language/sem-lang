#[path = "../../../test-support/adoption.rs"]
mod adoption_fixture;
use serde_json::{Value, json};
use std::{fs, process::Command};

fn run(args: &[&str]) -> std::process::Output {
    let mut args: Vec<String> = args.iter().map(|s| (*s).into()).collect();
    if args.get(1).is_some_and(|s| s == "write" || s == "adopt") {
        let get = |key: &str| args.windows(2).find(|p| p[0] == key).map(|p| p[1].clone());
        let repo = sem_lang_store_fs::FsRepository::open_existing(get("--repo").unwrap()).unwrap();
        let domain = format!("{}s-{}", args[0], get("--namespace").unwrap());
        let revision = adoption_fixture::base(
            &repo,
            &domain,
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
        .args(["--format", "json"])
        .args(args)
        .output()
        .unwrap()
}
fn success(args: &[&str]) -> Value {
    let result = run(args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stderr.is_empty(), "unexpected diagnostic");
    serde_json::from_slice(&result.stdout).unwrap()
}

#[test]
fn b9_cli_separate_process_restore_exact_retry_and_corruption_rejection() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_str().unwrap();
    let missing = run(&["binding", "read", "--repo", path, "--namespace", "owner"]);
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    assert!(!directory.path().join(".sem-lang").exists());
    let mut module: sem_lang_vocab::VocabularyModule = serde_json::from_str(r#"{"schema_version":"0.10.0","vocabulary_id":"example","version":"0.10.0","dependencies":[],"terms":[{"id":"42436820882964cfc19b6db3750b8d17","label":"Relation","description":"Relation","deprecation":null,"replacement":null,"semantic_definition":null}],"types":[],"roles":[],"lexical_bindings":[]}"#).unwrap();
    module.terms[0].id = sem_lang_core::TermId::from_declaration("example", "Relation");
    module.terms[0].semantic_definition =
        Some(serde_json::from_str(r#"{"Primitive":"Relation"}"#).unwrap());
    let term = module.terms[0].id;
    let definitions = directory.path().join("definitions.sem");
    fs::write(&definitions, module.to_sem().unwrap()).unwrap();
    success(&["init", path]);
    let adopted = success(&[
        "definition",
        "adopt",
        "--repo",
        path,
        "--namespace",
        "owner",
        "--file",
        definitions.to_str().unwrap(),
        "--decision-ref",
        "decision:definition",
    ]);
    let bindings = directory.path().join("bindings.json");
    let definition_ref = json!({"namespace":"owner", "revision":adopted["revision"], "term":term});
    fs::write(
        &bindings,
        serde_json::to_vec(&json!([{
            "binding_id":"residence", "semantic_ref":definition_ref,
            "target_ref":{"namespace":"presentation", "capability":"location", "revision":null},
            "usage":"presentation", "direction":"output", "constraints":[], "provenance_ref":"publisher:1"
        }]))
        .unwrap(),
    )
    .unwrap();
    let mut previous: Option<String> = None;
    for state in ["proposed", "reviewed", "adopted"] {
        let mut args = vec![
            "binding",
            "write",
            "--repo",
            path,
            "--namespace",
            "owner",
            "--file",
            bindings.to_str().unwrap(),
            "--state",
            state,
            "--decision-ref",
            state,
        ];
        if let Some(expected) = &previous {
            args.extend(["--expected-revision", expected]);
        }
        let result = success(&args);
        assert_eq!(success(&args), result, "exact retry through a new process");
        previous = Some(result["revision"].as_str().unwrap().into());
    }
    let read_args = ["binding", "read", "--repo", path, "--namespace", "owner"];
    let restored = success(&read_args);
    assert_eq!(restored["revision"], previous.unwrap());
    assert_eq!(restored["memory"]["state"], "adopted");
    assert_eq!(
        restored["memory"]["bindings"][0]["binding"]["semantic_ref"],
        definition_ref
    );
    assert_eq!(
        restored["memory"]["bindings"][0]["binding"]["target_ref"]["capability"],
        "location"
    );
    let parent = restored["memory"]["previous_revision"].as_str().unwrap();
    // Exact replay must not rewrite the immutable revision or canonical commit.
    let replay = success(&[
        "binding",
        "write",
        "--repo",
        path,
        "--namespace",
        "owner",
        "--file",
        bindings.to_str().unwrap(),
        "--state",
        "adopted",
        "--decision-ref",
        "adopted",
        "--expected-revision",
        parent,
    ]);
    assert_eq!(replay["revision"], restored["revision"]);
    assert_eq!(success(&read_args), restored);
    let digest = restored["revision"]
        .as_str()
        .unwrap()
        .strip_prefix("bindings-")
        .unwrap();
    let object = directory
        .path()
        .join(".sem-lang/objects/sha256")
        .join(&digest[..4])
        .join(&digest[4..]);
    fs::write(&object, b"corrupted").unwrap();
    let rejected = run(&read_args);
    assert!(!rejected.status.success());
    assert!(
        rejected.stdout.is_empty(),
        "corruption must not return empty successful memory"
    );
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("digest mismatch"));
}
