//! Provider-neutral inference execution. The semantic caller fences submission
//! and separately revalidates before admission; physical waiting needs no store
//! guard. Zixcel owns physical requests, not meaning or admission.
//! physical deadline ownership regression coverage.
use sem_lang_core::Value;
use sem_lang_execution::semantic::{
    ExactRef, InferenceExecutor, InferencePlan, ResultDisposition, SemanticOperationContract,
    TypedRequest, TypedResult, ValueKind,
};
use sem_lang_repository::Repository;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use zixcel_inference_cost::{CostEstimate, TokenUsage};
use zixcel_local_inference::{VerificationControl, inference as physical};

pub const MAX_REQUEST_BYTES: usize = 65_536;
pub const MAX_RESPONSE_BYTES: usize = 65_536;
pub const MAX_REQUESTS: usize = 128;
pub const MAX_EXECUTION_BUDGET_MS: u32 = 30_000;

pub use physical::{
    InferenceError, InferenceOptions, ReceiptStatus, RequestReference, UsageCoverage,
};
type Result<T> = std::result::Result<T, InferenceError>;
/// Not a public raw-model invocation constructor. Fields are sealed at exact
/// adopted semantic preparation. The private IPC is inside the same OS-owner
/// trust domain and is not a remote semantic authorization boundary.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceRequest {
    pub(crate) request_ref: String,
    pub(crate) target_ref: ExactRef,
    pub(crate) binding_ref: ExactRef,
    pub(crate) control_ref: ExactRef,
    pub(crate) runtime_ref: String,
    pub(crate) process_ref: String,
    pub(crate) contract: SemanticOperationContract,
    pub(crate) input: TypedRequest,
    pub(crate) options: InferenceOptions,
}

/// Execution metadata is not an Assertion, Claim or a memory write receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceResult {
    pub result_ref: String,
    pub request_ref: String,
    pub target_ref: ExactRef,
    pub binding_ref: ExactRef,
    pub runtime_ref: String,
    pub process_ref: String,
    pub output: TypedResult,
    pub disposition: ResultDisposition,
    pub usage: TokenUsage,
    /// The adapter reports total input and output only. Other `TokenUsage` fields
    /// are zero placeholders, not independently measured cache/reasoning usage.
    pub usage_coverage: UsageCoverage,
    /// Absent without an explicit rate card. Never invent free/billable prices.
    pub cost: Option<CostEstimate>,
    pub elapsed_ms: u64,
    /// Owner wall-clock observation only; absent if UTC cannot be represented.
    /// Never used for identity admission or elapsed-time enforcement.
    pub accepted_at_epoch_ms: Option<u64>,
}

/// Exact subordinate descriptor for an already admitted runtime, not a new ID.
/// # Errors
/// Rejects malformed runtime references; existence is checked by the owner.
pub fn runtime_implementation(runtime_ref: &str) -> Result<ExactRef> {
    if !digest(runtime_ref) {
        return Err(InferenceError::RuntimeNotFound);
    }
    Ok(ExactRef {
        owner: "zixcel-local-inference".into(),
        id: "runtime".into(),
        revision: runtime_ref.into(),
    })
}
fn identity<T: Serialize>(domain: &str, value: &T) -> Result<String> {
    {
        let bytes =
            serde_json::to_vec(&(domain, value)).map_err(|_| InferenceError::InvalidRequest)?;
        Ok(hex::encode(Sha256::digest(bytes)))
    }
}
fn bounded_reference(r: &ExactRef) -> Result<()> {
    if [&r.owner, &r.id, &r.revision]
        .iter()
        .any(|s| s.is_empty() || s.len() > 512 || !s.bytes().all(|b| b.is_ascii_graphic()))
    {
        return Err(InferenceError::InvalidRequest);
    }
    Ok(())
}
impl InferenceRequest {
    /// No role, model-label, provider selection, Grant or semantic adoption.
    /// # Errors
    /// Rejects non-adopted/mismatched targets, non-Ready processes and invalid bounds/types.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare<R: Repository>(
        repo: &R,
        executor: &InferenceExecutor,
        plan: &InferencePlan,
        input: &TypedRequest,
        process: &zixcel_local_inference::process::ProcessSnapshot,
        control_ref: ExactRef,
        options: InferenceOptions,
    ) -> Result<Self> {
        options.validate()?;
        bounded_reference(&control_ref)?;
        executor
            .validate(repo, plan, input)
            .map_err(|_| InferenceError::InvalidTypedInput)?;
        if executor.target().implementation != runtime_implementation(&process.runtime_ref)? {
            return Err(InferenceError::TargetMismatch);
        }
        if process.state != zixcel_local_inference::process::ProcessState::Ready {
            return Err(InferenceError::RuntimeNotReady);
        }
        let mut request = Self {
            request_ref: String::new(),
            target_ref: plan.target_ref().clone(),
            binding_ref: plan.binding_ref(),
            control_ref,
            runtime_ref: process.runtime_ref.clone(),
            process_ref: process.process_ref.clone(),
            contract: executor.contract().clone(),
            input: input.clone(),
            options,
        };
        request.request_ref = request.identity()?;
        request.validate()?;
        Ok(request)
    }
    #[must_use]
    pub fn request_ref(&self) -> &str {
        &self.request_ref
    }

    #[must_use]
    pub fn reference(&self) -> RequestReference {
        RequestReference {
            request_ref: self.request_ref.clone(),
            process_ref: self.process_ref.clone(),
        }
    }
    /// Verify that a serialized prepared request belongs to the caller's exact
    /// semantic/control snapshot. This does not select a process, refresh a
    /// deadline, authorize execution, or turn an inspected receipt into truth.
    /// # Errors
    /// Rejects tampering and substitution of any accepted input/context.
    pub fn validate_context(
        &self,
        control: &ExactRef,
        input: &TypedRequest,
        runtime_ref: &str,
        process_ref: &str,
        options: &InferenceOptions,
    ) -> Result<()> {
        self.validate()?;
        if &self.control_ref != control
            || &self.input != input
            || &self.options != options
            || self.runtime_ref != runtime_ref
            || self.process_ref != process_ref
        {
            return Err(InferenceError::InvalidRequest);
        }
        Ok(())
    }
    fn identity(&self) -> Result<String> {
        Ok(self.physical()?.request_ref().into())
    }
    fn physical(&self) -> Result<physical::InferenceRequest> {
        let context = identity(
            "caller/inference/context/2",
            &(
                &self.target_ref,
                &self.binding_ref,
                &self.control_ref,
                &self.runtime_ref,
                &self.process_ref,
                &self.contract,
                &self.input,
                &self.options,
            ),
        )?;
        let Value::Text(text) = &self
            .input
            .arguments
            .values()
            .next()
            .ok_or(InferenceError::InvalidTypedInput)?
            .value
        else {
            return Err(InferenceError::InvalidTypedInput);
        };
        let process = zixcel_local_inference::process::ProcessSnapshot {
            runtime_ref: self.runtime_ref.clone(),
            process_ref: self.process_ref.clone(),
            state: zixcel_local_inference::process::ProcessState::Ready,
            reason: None,
            termination: None,
            rss_bytes: 0,
            peak_rss_bytes: 0,
            threads: 0,
            loaded_artifact_refs: vec![],
            observed_host_libraries: vec![],
        };
        physical::InferenceRequest::prepare(&process, context, text.clone(), self.options.clone())
    }
    /// Verify the complete immutable request, including its content identity.
    /// This is pure validation: it never reads storage, contacts an owner,
    /// selects a runtime or authorizes a new execution. Historical receipt
    /// consumers may validate a supplied body after the runtime has stopped.
    /// # Errors
    /// Rejects invalid bounds, types, configuration or a substituted body.
    pub fn validate(&self) -> Result<()> {
        self.options.validate()?;
        if !digest(&self.runtime_ref) || !digest(&self.process_ref) {
            return Err(InferenceError::InvalidRequest);
        }
        for r in [&self.target_ref, &self.binding_ref, &self.control_ref] {
            bounded_reference(r)?;
        }
        // This adapter supports one explicitly typed text input and text output;
        // the argument's name is declared by the exact contract, never guessed.
        let Some(spec) = self.contract.inputs.values().next() else {
            return Err(InferenceError::InvalidTypedInput);
        };
        if self.contract.inputs.len() != 1
            || spec.kind != ValueKind::Text
            || self.contract.output.kind != ValueKind::Text
            || self.input.arguments.len() != 1
            || self.input.contract_ref
                != self
                    .contract
                    .exact_ref()
                    .map_err(|_| InferenceError::TargetMismatch)?
        {
            return Err(InferenceError::InvalidTypedInput);
        }
        let (name, value) = self
            .input
            .arguments
            .iter()
            .next()
            .ok_or(InferenceError::InvalidTypedInput)?;
        if self.contract.inputs.get(name) != Some(spec)
            || value.type_ref != spec.reference
            || !matches!(&value.value, sem_lang_core::Value::Text(_))
        {
            return Err(InferenceError::InvalidTypedInput);
        }
        let encoded =
            serde_json::to_vec(&self.input).map_err(|_| InferenceError::InvalidTypedInput)?;
        TypedRequest::decode(&encoded).map_err(|_| InferenceError::InvalidTypedInput)?;
        if serde_json::to_vec(self)
            .map_err(|_| InferenceError::InvalidRequest)?
            .len()
            > MAX_REQUEST_BYTES
        {
            return Err(InferenceError::InvalidRequest);
        }
        if self.identity()? != self.request_ref {
            return Err(InferenceError::InvalidRequest);
        }
        Ok(())
    }
}
impl InferenceResult {
    fn validate(&self, request: &InferenceRequest) -> Result<()> {
        let bytes = serde_json::to_vec(self).map_err(|_| InferenceError::BackendOutputInvalid)?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(InferenceError::OutputLimitExceeded);
        }
        let output =
            serde_json::to_vec(&self.output).map_err(|_| InferenceError::BackendOutputInvalid)?;
        TypedResult::decode(&output).map_err(|_| InferenceError::BackendOutputInvalid)?;
        if self.output.contract_ref != request.input.contract_ref
            || self.output.output.type_ref != request.contract.output.reference
            || !matches!(self.output.output.value, sem_lang_core::Value::Text(_))
            || self.usage.output_tokens > u64::from(request.options.max_output_tokens)
            || self.usage.cached_input_tokens != 0
            || self.usage.cache_write_input_tokens != 0
            || self.usage.reasoning_output_tokens != 0
        {
            return Err(InferenceError::BackendOutputInvalid);
        }
        let expected = request
            .options
            .cost_rates
            .as_ref()
            .map(|rates| zixcel_inference_cost::estimate(&self.usage, rates))
            .transpose()
            .map_err(|_| InferenceError::CostAccountingError)?;
        if self.cost != expected {
            return Err(InferenceError::CostAccountingError);
        }
        let physical_request = request.physical()?;
        let Value::Text(text) = &self.output.output.value else {
            return Err(InferenceError::BackendOutputInvalid);
        };
        let receipt = physical::InferenceResult {
            result_ref: self.result_ref.clone(),
            request_ref: self.request_ref.clone(),
            context_digest: physical_request.context_digest().into(),
            runtime_ref: self.runtime_ref.clone(),
            process_ref: self.process_ref.clone(),
            output: text.clone(),
            usage: self.usage,
            usage_coverage: self.usage_coverage,
            cost: self.cost.clone(),
            elapsed_ms: self.elapsed_ms,
            accepted_at_epoch_ms: self.accepted_at_epoch_ms,
        };
        receipt.validate(&physical_request)?;
        if self.request_ref != request.request_ref
            || self.runtime_ref != request.runtime_ref
            || self.process_ref != request.process_ref
            || self.target_ref != request.target_ref
            || self.binding_ref != request.binding_ref
            || self.disposition != ResultDisposition::UnadoptedObservation
        {
            return Err(InferenceError::BackendOutputInvalid);
        }
        Ok(())
    }
}

/// Exact replay is only available in this live owner. Owner death loses results;
/// its process incarnation is retired. This function never retries execution.
/// # Errors
/// Exact semantic, process, cancellation, deadline and backend errors are propagated.
pub fn execute<R: Repository>(
    root: &Path,
    repo: &R,
    executor: &InferenceExecutor,
    plan: &InferencePlan,
    request: &InferenceRequest,
    control: &VerificationControl,
) -> Result<InferenceResult> {
    submit(root, repo, executor, plan, request, control)?;
    let result = wait(root, request, control)?;
    executor
        .observe(repo, plan, &request.input, &result.output)
        .map_err(|_| InferenceError::BackendOutputInvalid)?;
    Ok(result)
}

fn check_control(control: &VerificationControl) -> Result<()> {
    control.check().map_err(|e| {
        if e.code() == "cancelled" {
            InferenceError::Cancelled
        } else {
            InferenceError::TransportTimeout
        }
    })
}

/// Validate exact adopted semantic input and submit once while the caller's
/// existing semantic fence is held. Returns after owner acceptance, not model
/// completion. The caller may release its repository guard before `wait`.
/// An interrupted acceptance reply must be recovered by `inspect`, never by
/// silently resubmitting. The owner retains the original volatile receipt.
/// # Errors
/// Semantic, target, control, process and physical owner failures reject.
pub fn submit<R: Repository>(
    root: &Path,
    repo: &R,
    executor: &InferenceExecutor,
    plan: &InferencePlan,
    request: &InferenceRequest,
    control: &VerificationControl,
) -> Result<()> {
    check_control(control)?;
    executor
        .validate(repo, plan, &request.input)
        .map_err(|_| InferenceError::InvalidTypedInput)?;
    if request.target_ref != *plan.target_ref()
        || request.binding_ref != plan.binding_ref()
        || executor.target().implementation != runtime_implementation(&request.runtime_ref)?
    {
        return Err(InferenceError::TargetMismatch);
    }
    request.validate()?;
    physical::submit(root, &request.physical()?, control)
}

/// Wait only for an exact already-submitted physical result. This never sends
/// Infer, acquires a semantic repository guard or interprets/adopts the output.
/// After waiting the semantic owner must revalidate its expected state before
/// interpreting/admitting. A lost volatile receipt remains explicitly lost.
/// # Errors
/// Cancellation, deadline, missing receipt and physical/result failures reject.
pub fn wait(
    root: &Path,
    request: &InferenceRequest,
    control: &VerificationControl,
) -> Result<InferenceResult> {
    request.validate()?;
    convert(
        request,
        physical::wait(root, &request.physical()?, control)?,
    )
}

/// Read-only exact retained physical observation. It is never an adoption and
/// never sends an execution command. Useful after loss of an acceptance reply.
/// # Errors
/// Unknown/lost requests and unavailable owners or invalid result identities reject.
pub fn inspect(root: &Path, request: &InferenceRequest) -> Result<Option<InferenceResult>> {
    request.validate()?;
    physical::inspect(root, &request.physical()?)?
        .map(|result| convert(request, result))
        .transpose()
}

/// Exact request cancellation, not a runtime stop. A final result remains final.
/// # Errors
/// Unknown requests and unavailable owners reject; no other request is cancelled.
pub fn cancel(root: &Path, request: &InferenceRequest) -> Result<()> {
    request.validate()?;
    physical::cancel(root, &request.physical()?)
}

pub use physical::{cancel_reference, inspect_reference};
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn convert(
    request: &InferenceRequest,
    result: physical::InferenceResult,
) -> Result<InferenceResult> {
    result.validate(&request.physical()?)?;
    let value = InferenceResult {
        result_ref: result.result_ref,
        request_ref: result.request_ref,
        target_ref: request.target_ref.clone(),
        binding_ref: request.binding_ref.clone(),
        runtime_ref: result.runtime_ref,
        process_ref: result.process_ref,
        output: TypedResult {
            contract_ref: request.input.contract_ref.clone(),
            output: sem_lang_execution::semantic::TypedValue {
                type_ref: request.contract.output.reference.clone(),
                value: Value::Text(result.output),
            },
        },
        disposition: ResultDisposition::UnadoptedObservation,
        usage: result.usage,
        usage_coverage: result.usage_coverage,
        cost: result.cost,
        elapsed_ms: result.elapsed_ms,
        accepted_at_epoch_ms: result.accepted_at_epoch_ms,
    };
    value.validate(request)?;
    Ok(value)
}
