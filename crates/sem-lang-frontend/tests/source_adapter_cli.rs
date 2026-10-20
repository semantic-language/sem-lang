use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};
use zixcel_json_structure::{Decomposition, StructureBudget, decompose};
fn request() -> Value {
    let tree = decompose(&Decomposition {
        reference: "source:fixture".into(),
        document: json!({"flag": true}),
        pointer: String::new(),
        budget: StructureBudget::default(),
    })
    .unwrap();
    json!({"tree":tree,"context":{"subject":{"id":"fixture-owner","class":"person"},
        "evidence":[],"at":null,"locale":"en","semantic_revision":"a".repeat(64),
        "context_refs":[],"context_scope":"CurrentRequest","observation_evidence":[]},
        "budget":sem_lang_frontend::current::structural::StructuralBudget::default()})
}
fn run(bytes: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sem-source-adapter"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn explicit_owner_context_survives_the_process_boundary() {
    let request = request();
    let output = run(&serde_json::to_vec(&request).unwrap());
    assert!(output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["context"], request["context"]);
    assert_eq!(result["reference"], "source:fixture");
}
#[test]
fn no_context_defaults_unknown_fields_or_source_echo_on_failure() {
    let mut input = request();
    input.as_object_mut().unwrap().remove("context");
    input["synthetic-secret"] = json!("must-never-be-echoed");
    let output = run(&serde_json::to_vec(&input).unwrap());
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"source-adapter-rejected\n");
    let mut input = request();
    input["extra"] = json!(true);
    assert!(!run(&serde_json::to_vec(&input).unwrap()).status.success());
}
