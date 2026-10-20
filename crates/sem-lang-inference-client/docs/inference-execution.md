# Bounded inference execution (0.10.0)

<!-- Execution deadline belongs to the physical owner. -->

`sem-lang-execution::semantic::InferenceExecutor` validates the canonical adopted
binding, exact operation/target/contract and C2 typed input. The caller fences
preparation/submission and revalidates separately after physical completion.
Zixcel does not own a semantic store or perform adoption.

`InferenceRequest::prepare` freezes that plan's exact target and binding, caller
control reference, admitted runtime implementation, current `ProcessSnapshot`
references, typed input/contract and finite execution budget. `execute` revalidates the
semantic plan and contacts the already running inference owner. It cannot launch a
process. `inspect` retrieves the same retained observation after a lost reply;
`cancel` requests cancellation of that exact request, not the runtime process.

For a filesystem semantic repository with a lifetime-exclusive lock, use the
split path: `submit` under the existing exact semantic fence, release the
repository handle, then `wait` without a repository. Acquire a fresh semantic
fence and revalidate the original role, binding, target and expected semantic
revision before interpretation/admission. `wait` only inspects the exact receipt;
it cannot start an inference. Neither waiting nor successful submission grants
semantic adoption. The blocking `execute` helper retains its original signature
and final semantic validation; do not use it while holding a product-wide writer
lock. A submission transport failure is ambiguous: inspect the original request
instead of automatically calling `submit` again.

The accepted inference-owner references are validated 64-character lowercase hexadecimal
`String` fields (`runtime_ref`, `process_ref`), not Rust newtypes. They are reused
unchanged. Only the inference owner mints a process incarnation. The implementation
descriptor is `ExactRef { owner: "zixcel-local-inference", id: "runtime",
revision: <RuntimeRef> }`; this is an exact subordinate reference, not a new
runtime identity. External Worker/WorkerGeneration/Grant/Invocation remain separate.

The private same-OS-owner socket is a physical control channel, not a remote
semantic authorization service. Only the semantic-gated Rust API is public for
inference. Do not expose raw IPC to untrusted clients; it does not authenticate a
remote caller or independently reopen a semantic repository. Descriptor labels,
model names, raw URLs and paths never select an execution target.

## Bounds and lifecycle

- One active inference worker, no hidden queue; another new request rejects with
  `CapacityExceeded`. Completed requests do not reserve the execution slot.
- At most 128 retained terminal/running request entries per live owner. No silent
  eviction or automatic rerun. At capacity, explicit owner lifecycle management
  is necessary; durable result retention/reclamation is outside this slice.
- Request: 65,536 encoded bytes, existing C2 bounds (text 4,096 UTF-8 bytes,
  map 64, sequence 256, depth 16, values 2,048). This adapter accepts one declared
  Text argument and Text output, not arbitrary backend JSON.
- Backend response: 65,536 bytes including headers; header maximum 8,192 bytes.
  The fixed adapter accepts bounded Content-Length responses, not streaming.
- Output limit 1–256 tokens; `options.execution_budget_ms` is 1–30,000 ms,
  fixed in the request identity. The physical owner binds one monotonic origin
  immediately before worker creation; startup, conversion and exchange share
  the original deadline. No creator UTC participates in admission or execution.
  Internal `timeout_ms`/`deadline_ms` are removed with no compatibility alias.
- The execution host owns queue policy, independently (current Work has no queue TTL).
  Crowsi owns bounded framing; its supplied `Instant` is in the physical owner's
  clock domain. `wait` has a caller-owned observation timeout, not execution
  authority: expiry does not cancel the physical request. Explicit cancellation
  still does. Exact lookup, resupply and replay cannot renew the original budget.
- Crowsi checks cancellation/generation before connect and every at-most-50 ms
  I/O wait. Cancellation closes only the request socket. No reconnect, resend,
  automatic process restart, model/provider fallback or deadline extension.
- Read/stop/cancel control remains available while inference runs. Workers are
  joined/reaped; owner destruction cancels requests before terminating its model.
- Stale G1 cannot dispatch against G2. A retained G1 result remains a G1
  observation; reading it never rebinds it to G2.

## Results, usage and failure

Request and result hashes cover their respective exact contents and domains.
Results retain binding/target/runtime/process/request references and the C2 typed
result. `UnadoptedObservation` is an execution disposition, not a duplicate
semantic Evidence/Claim type. Explicit observation compilation/admission remains
in sem-lang. No semantic write or provider world-effect Grant is performed here.

The current adapter reports input/output tokens only. `UsageCoverage` makes that
restriction explicit; other `TokenUsage` fields are unmeasured zero placeholders.
Costs use `zixcel-inference-cost` and an optional explicit bounded rate card.
Only measured input/output rates are accepted. No configured rates means no cost
estimate, not zero cost. Estimates are never authoritative billing or semantics.

Errors are provider-neutral typed outcomes. Backend HTTP details, model files,
loader layout, private port and authentication key are never public fields.
Requests are not logged. Tests use synthetic text and never assess model quality.

`InvalidExecutionBudget`, `ExecutionTimedOut`, and `TransportTimeout` distinguish
invalid duration, physical elapsed expiry, and bounded observation/framing.
An optional `accepted_at_epoch_ms` records owner UTC for provenance only; missing
or skewed UTC cannot invalidate an otherwise valid request. `elapsed_ms` measures
the original owner's monotonic elapsed time. Monotonic raw instants are private.
The same semantic/control/process/input/budget tuple has the same request ID;
an explicitly different invocation/control identity is needed for a new attempt.

Results in this adapter are not durable. Owner death loses receipts/results and retires its
live process incarnation. Inspection then fails explicitly; it does not infer
that a request failed to execute and does not repeat probabilistic computation.
Application orchestration/routing and native module activation remain the caller's responsibility.
