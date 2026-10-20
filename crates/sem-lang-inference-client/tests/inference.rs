//! External consumer proof: exact adoption, owner IPC and an actual admitted
//! model. The real fixture is explicit/ignored, never a passing mock substitute.
//! canonical finite execution budget without UTC authority.
#[path = "support/inference.rs"]
mod fixture;
use sem_lang_execution::semantic::*;
use sem_lang_inference_client::inference::*;
use sem_lang_repository::{HeadStore, ObjectStore};
use sem_lang_store_fs::FsRepository;
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use zixcel_local_inference::{
    VerificationControl,
    process::{self, ProcessCommand, ProcessSnapshot, ProcessState},
};

fn control(id: &str) -> ExactRef {
    ExactRef {
        owner: "test".into(),
        id: id.into(),
        revision: "explicit:1".into(),
    }
}
fn options() -> InferenceOptions {
    InferenceOptions {
        max_output_tokens: 8,
        execution_budget_ms: 30_000,
        cost_rates: None,
    }
}

#[test]
fn consumer_requires_adopted_inference_target_and_rejects_effect_or_mutation() {
    let runtime = "a".repeat(64);
    let (repo, binding, target, contract, input) =
        fixture::fixture(TargetKind::InferenceRuntime, &runtime);
    let executor = InferenceExecutor::new(target.clone(), contract.clone()).expect("test fixture");
    let plan = executor
        .prepare(&repo, "owner", &binding, &input)
        .expect("test fixture");
    let snapshot = ProcessSnapshot {
        runtime_ref: runtime,
        process_ref: "b".repeat(64),
        state: ProcessState::Ready,
        reason: None,
        termination: None,
        rss_bytes: 0,
        peak_rss_bytes: 0,
        threads: 0,
        loaded_artifact_refs: vec![],
        observed_host_libraries: vec![],
    };
    let prepared = InferenceRequest::prepare(
        &repo,
        &executor,
        &plan,
        &input,
        &snapshot,
        control("A"),
        options(),
    )
    .expect("test fixture");
    let before = repo.head_revision("bindings-owner").expect("test fixture");
    check_context(&prepared, &input, &snapshot);
    let cancelled = VerificationControl::new(Duration::from_secs(1));
    cancelled.cancel();
    // One matrix applies identical guards to blocking and split execution.
    for split in [false, true] {
        let invoke = |request: &InferenceRequest, control: &VerificationControl| {
            let root = Path::new("/does-not-exist");
            if split {
                submit(root, &repo, &executor, &plan, request, control)
            } else {
                execute(root, &repo, &executor, &plan, request, control).map(|_| ())
            }
        };
        assert_eq!(
            invoke(&prepared, &cancelled),
            Err(InferenceError::Cancelled)
        );
        for mutated in mutations(&prepared) {
            assert!(matches!(
                invoke(&mutated, &VerificationControl::new(Duration::from_secs(1))),
                Err(InferenceError::InvalidRequest
                    | InferenceError::InvalidTypedInput
                    | InferenceError::TargetMismatch)
            ));
        }
    }
    let mut incompatible = snapshot.clone();
    incompatible.runtime_ref = "c".repeat(64);
    assert!(matches!(
        InferenceRequest::prepare(
            &repo,
            &executor,
            &plan,
            &input,
            &incompatible,
            control("A"),
            options()
        ),
        Err(InferenceError::TargetMismatch)
    ));
    for kind in [
        TargetKind::PureBuiltin,
        TargetKind::PureModule,
        TargetKind::Capability,
    ] {
        let mut rejected = target.clone();
        rejected.kind = kind;
        assert!(matches!(
            InferenceExecutor::new(rejected, contract.clone()),
            Err(ContractError::AuthorizationRequired)
        ));
    }
    assert!(PureExecutor::new(target, contract, |_| Err(ContractError::Handler)).is_err());
    assert_eq!(
        repo.head_revision("bindings-owner").expect("test fixture"),
        before
    );
}
fn check_context(prepared: &InferenceRequest, input: &TypedRequest, snapshot: &ProcessSnapshot) {
    assert_eq!(prepared.validate(), Ok(()));
    for altered in mutations(prepared) {
        assert!(altered.validate().is_err());
    }
    assert_eq!(
        prepared.validate_context(
            &control("A"),
            input,
            &snapshot.runtime_ref,
            &snapshot.process_ref,
            &options()
        ),
        Ok(())
    );
    assert_eq!(
        prepared.validate_context(
            &control("other"),
            input,
            &snapshot.runtime_ref,
            &snapshot.process_ref,
            &options()
        ),
        Err(InferenceError::InvalidRequest)
    );
    assert_eq!(
        prepared.validate_context(
            &control("A"),
            input,
            &snapshot.runtime_ref,
            &"c".repeat(64),
            &options()
        ),
        Err(InferenceError::InvalidRequest)
    );
    let mut changed = input.clone();
    changed
        .arguments
        .values_mut()
        .next()
        .expect("one argument")
        .value = sem_lang_core::Value::Text("substituted content".into());
    assert_eq!(
        prepared.validate_context(
            &control("A"),
            &changed,
            &snapshot.runtime_ref,
            &snapshot.process_ref,
            &options()
        ),
        Err(InferenceError::InvalidRequest)
    );
}

struct Owner {
    child: Child,
    root: PathBuf,
}

#[test]
fn reopened_semantic_owner_rejects_revoked_binding_without_execution() {
    use sem_lang_memory::bindings::{self, AdoptionState};
    let temp = tempfile::tempdir().expect("isolated repository");
    let mut repo = FsRepository::create(temp.path()).expect("fixture");
    let (binding, target, contract, input) =
        fixture::populate(&mut repo, TargetKind::InferenceRuntime, &"a".repeat(64));
    let executor = InferenceExecutor::new(target, contract.clone()).expect("fixture");
    let plan = executor
        .prepare(&repo, "owner", &binding, &input)
        .expect("fixture");
    drop(repo);

    let mut writer = FsRepository::open_existing(temp.path()).expect("independent writer");
    let parent = bindings::read(&writer, "owner")
        .expect("fixture")
        .expect("adopted")
        .0;
    let expected = writer.head_revision("bindings-owner").expect("exact head");
    bindings::write(
        &mut writer,
        "owner",
        Some(&parent),
        std::slice::from_ref(&binding.binding),
        AdoptionState::Rejected,
        "decision:explicit-revocation",
        &expected,
    )
    .expect("explicit owner revocation");
    drop(writer);

    let repo = FsRepository::open_existing(temp.path()).expect("fresh fence");
    assert_eq!(
        executor.validate(&repo, &plan, &input),
        Err(ContractError::Binding)
    );
    let output = TypedResult {
        contract_ref: contract.exact_ref().expect("fixture"),
        output: input.arguments["value"].clone(),
    };
    assert_eq!(
        executor.observe(&repo, &plan, &input, &output),
        Err(ContractError::Binding)
    );
}
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = process::request(&self.root, &ProcessCommand::Shutdown {});
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn ready(root: &Path) -> ProcessSnapshot {
    let deadline = Instant::now() + Duration::from_secs(110);
    loop {
        if let Ok(reply) = process::request(root, &ProcessCommand::Inspect {})
            && let Some(p) = reply.process
        {
            if p.state == ProcessState::Ready {
                return p;
            }
            assert!(
                !matches!(p.state, ProcessState::Failed | ProcessState::Stopped),
                "{p:?}"
            );
        }
        assert!(Instant::now() < deadline, "readiness timeout");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "requires explicit accepted M4-B registry; no model downloads"]
fn actual_owner_inference_replay_cancel_and_generation_fencing_leave_memory_unchanged() {
    let runtime = std::env::var("ZIXCEL_PROOF_RUNTIME").expect("explicit runtime");
    let (temp, _owner, first) = launch_fixture(&runtime);
    let root = temp.path();
    let mut repo = FsRepository::create(root.join("semantic")).expect("isolated owner repository");
    let (binding, target, contract, input) =
        fixture::populate(&mut repo, TargetKind::InferenceRuntime, &runtime);
    let before = repo.head_revision("bindings-owner").expect("test fixture");
    let executor = InferenceExecutor::new(target, contract).expect("test fixture");
    let plan = executor
        .prepare(&repo, "owner", &binding, &input)
        .expect("test fixture");
    let a = InferenceRequest::prepare(
        &repo,
        &executor,
        &plan,
        &input,
        &first,
        control("A"),
        options(),
    )
    .expect("test fixture");
    let (repo, result) = submit_with_released_writer(root, repo, &executor, &plan, &a);
    executor
        .observe(&repo, &plan, &input, &result.output)
        .expect("semantic revalidation");
    assert_result(&result, &a, &first, &plan);
    assert_eq!(
        inspect(root, &a).expect("test fixture"),
        Some(result.clone())
    );
    assert_eq!(
        wait(root, &a, &VerificationControl::new(Duration::from_secs(1)))
            .expect("retained exact result"),
        result
    );
    cancel(root, &a).expect("test fixture"); // Cancel a terminal A, then B must still run.
    let b = InferenceRequest::prepare(
        &repo,
        &executor,
        &plan,
        &input,
        &first,
        control("B"),
        cost_options(),
    )
    .expect("test fixture");
    let second = execute(
        root,
        &repo,
        &executor,
        &plan,
        &b,
        &VerificationControl::new(Duration::from_secs(30)),
    )
    .expect("test fixture");
    assert_ne!(result.request_ref, second.request_ref);
    assert_ne!(result.result_ref, second.result_ref);
    assert_eq!(second.process_ref, first.process_ref);
    let cost = second.cost.as_ref().expect("explicit estimate");
    assert!(!cost.billing_authoritative);
    assert_eq!(cost.usage_digest, second.usage.digest());
    assert_eq!(
        cost.total_micro_units,
        second.usage.input_tokens + 2 * second.usage.output_tokens
    );
    let stale = InferenceRequest::prepare(
        &repo,
        &executor,
        &plan,
        &input,
        &first,
        control("stale"),
        options(),
    )
    .expect("test fixture");
    let next = restart(root, &first);
    assert_ne!(first.process_ref, next.process_ref);
    assert_eq!(
        execute(
            root,
            &repo,
            &executor,
            &plan,
            &stale,
            &VerificationControl::new(Duration::from_secs(30))
        ),
        Err(InferenceError::StaleRuntimeProcess)
    );
    assert_eq!(
        inspect(root, &a).expect("test fixture"),
        Some(result.clone())
    );
    assert_eq!(
        repo.head_revision("bindings-owner").expect("test fixture"),
        before
    );
    println!(
        "INFERENCE_PROOF {}",
        serde_json::json!({"status":"PASS","runtimeRef":runtime,"first":first,"replacement":next,"requestRef":result.request_ref,"resultRef":result.result_ref,"usage":result.usage,"cost":result.cost,"elapsedMs":result.elapsed_ms,"secondRequestRef":second.request_ref,"secondResultRef":second.result_ref,"secondCost":second.cost,"usageCoverage":result.usage_coverage,"semanticBefore":before,"semanticAfter":repo.head_revision("bindings-owner").expect("test fixture"),"autoAdoptions":0})
    );
}

fn submit_with_released_writer(
    root: &Path,
    repo: FsRepository,
    executor: &InferenceExecutor,
    plan: &InferencePlan,
    request: &InferenceRequest,
) -> (FsRepository, InferenceResult) {
    // Merely waiting cannot create an execution, even for a valid prepared request.
    assert_eq!(
        wait(
            root,
            request,
            &VerificationControl::new(Duration::from_secs(1))
        ),
        Err(InferenceError::UnknownRequest)
    );
    submit(
        root,
        &repo,
        executor,
        plan,
        request,
        &VerificationControl::new(Duration::from_secs(30)),
    )
    .expect("submission");
    drop(repo);
    // A different handle can acquire the real exclusive writer lock after
    // submission, before waiting, without changing an adopted semantic head.
    let mut writer = FsRepository::open_existing(root.join("semantic"))
        .expect("inference must not retain the global semantic writer lock");
    let physical = writer
        .put_object(b"unrelated immutable object".to_vec())
        .expect("writer remains available");
    drop(writer);
    let result = wait(
        root,
        request,
        &VerificationControl::new(Duration::from_secs(30)),
    )
    .expect("result");
    let repo = FsRepository::open_existing(root.join("semantic"))
        .expect("fresh owner fence after physical completion");
    assert!(repo.contains_object(&physical));
    (repo, result)
}

fn launch_fixture(runtime: &str) -> (tempfile::TempDir, Owner, ProcessSnapshot) {
    let source = PathBuf::from(std::env::var("ZIXCEL_PROOF_REGISTRY").expect("explicit fixture"));
    let temp = tempfile::tempdir().expect("test fixture");
    let root = temp.path();
    std::fs::copy(source.join("admission.redb"), root.join("admission.redb"))
        .expect("test fixture");
    std::fs::File::create(root.join("admission.lock")).expect("test fixture");
    std::fs::create_dir(root.join("objects")).expect("test fixture");
    for f in std::fs::read_dir(source.join("objects")).expect("test fixture") {
        let f = f.expect("test fixture");
        std::fs::hard_link(f.path(), root.join("objects").join(f.file_name()))
            .expect("test fixture");
    }
    let child = Command::new(
        std::env::var("ZIXCEL_LOCAL_INFERENCE_TEST_BINARY").expect("explicit provider test binary"),
    )
    .args([
        "runtime-serve",
        "--state",
        root.to_str().expect("test fixture"),
    ])
    .env_clear()
    .env("LANG", "C")
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::inherit())
    .spawn()
    .expect("test fixture");
    let owner = Owner {
        child,
        root: root.into(),
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while process::request(root, &ProcessCommand::Inspect {}).is_err() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let start = process::request(
        root,
        &ProcessCommand::Start {
            runtime_ref: runtime.into(),
            request_ref: "d".repeat(64),
        },
    )
    .expect("test fixture");
    assert_eq!(start.rejection, None);
    let first = ready(root);
    (temp, owner, first)
}
fn cost_options() -> InferenceOptions {
    let mut cost_options = options();
    cost_options.cost_rates = Some(zixcel_inference_cost::CostRateCard {
        schema: zixcel_inference_cost::RATE_CARD_SCHEMA_V1.into(),
        unit: "synthetic-test-credit".into(),
        source: "synthetic:test".into(),
        as_of: "2026-09-11".into(),
        input_micro_units_per_token: 1,
        output_micro_units_per_token: 2,
        cached_input_micro_units_per_token: 0,
        cache_write_input_micro_units_per_token: 0,
        reasoning_output_micro_units_per_token: 0,
    });

    cost_options
}
fn assert_result(
    result: &InferenceResult,
    request: &InferenceRequest,
    first: &ProcessSnapshot,
    plan: &InferencePlan,
) {
    assert_eq!(result.request_ref, request.request_ref());
    assert_eq!(result.runtime_ref, first.runtime_ref);
    assert_eq!(result.process_ref, first.process_ref);
    assert_eq!(result.target_ref, *plan.target_ref());
    assert_eq!(result.binding_ref, plan.binding_ref());
    assert_eq!(result.disposition, ResultDisposition::UnadoptedObservation);
    assert!(
        matches!(&result.output.output.value,sem_lang_core::Value::Text(s) if s.len()<=MAX_RESPONSE_BYTES)
    );
    assert!(result.usage.output_tokens <= 8);
    assert_eq!(result.cost, None);
}
fn restart(root: &Path, first: &ProcessSnapshot) -> ProcessSnapshot {
    let restart = process::request(
        root,
        &ProcessCommand::Restart {
            process_ref: first.process_ref.clone(),
            runtime_ref: first.runtime_ref.clone(),
            request_ref: "e".repeat(64),
        },
    )
    .expect("test fixture");
    assert_eq!(restart.rejection, None);
    ready(root)
}
fn mutations(prepared: &InferenceRequest) -> Vec<InferenceRequest> {
    let mut values = Vec::new();
    for field in ["runtime_ref", "process_ref", "request_ref"] {
        let mut wire = serde_json::to_value(prepared).expect("test fixture");
        wire[field] = serde_json::json!("c".repeat(64));
        let mutated = serde_json::from_value(wire).expect("test fixture");
        values.push(mutated);
    }
    for (pointer, value) in [
        ("/target_ref/revision", serde_json::json!("changed")),
        ("/binding_ref/revision", serde_json::json!("changed")),
        ("/control_ref/revision", serde_json::json!("changed")),
        (
            "/input/arguments/value/value/Text",
            serde_json::json!("changed"),
        ),
        ("/options/max_output_tokens", serde_json::json!(7)),
        ("/options/execution_budget_ms", serde_json::json!(29999)),
        (
            "/options/cost_rates",
            serde_json::to_value(cost_options().cost_rates).expect("rates"),
        ),
        (
            "/contract/output/reference/revision",
            serde_json::json!("changed"),
        ),
    ] {
        let mut wire = serde_json::to_value(prepared).expect("wire");
        *wire.pointer_mut(pointer).expect("real field") = value;
        let mutated = serde_json::from_value(wire).expect("well shaped mutation");
        values.push(mutated);
    }

    values
}
