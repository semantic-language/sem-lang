# Public source-owner composition acceptance

A separate, source-only integration executable, outside the foundational workspace.
It reuses public HAT input validation/transport and attaches only the explicitly
provided private language context. Foundation crates have no HAT/service dependency.

Build with `cargo build --locked --manifest-path fixtures/source-owner-composition/Cargo.toml`.
Run the cross-owner tests with
`cargo test --locked --manifest-path fixtures/source-owner-composition/Cargo.toml`.
Both commands, formatting and Clippy run in the public source CI.
Run its binary with an absolute synthetic owner JSON path and a new socket path in
an owner-only temporary directory. Owner JSON is `{plan, context, budget}`; plan is
sealed by the public source producer and contains no language context. Public
`describe` omits source values and private context. `bind` composes the validated
SourceTree into StructuralInput. The execution consumer still verifies subject and current semantic
revision at admission. This fixture installs no provider into a real environment.

## What is exercised

The tests call the actual `hat-source-curator` implementation at public revision
`199cd7b792fdae7a8bdca14d46e9895dece3efba`, selected in this fixture's manifest
and lockfile. They pass its validated SourceTree directly to sem-lang's existing
`source_to_structural` adapter. No producer mock, duplicate source decomposer,
semantic schema or execution engine is introduced.

| Boundary | Required result |
| --- | --- |
| Selected record and declared editable field | Current supplied value reaches structural input; original source remains unchanged |
| Unselected audit/profile data | Never supplies source values |
| Stale source revision, changed contract/target/operation, undeclared or oversized input | Producer rejects before language composition |
| Changed source, private binding or revision without resealing | Contract identity rejects substitution |
| Exact serialized owner reloaded and replayed | Same structural output, unchanged owner state; stale request still rejected |
| Sequence or fractional number accepted by the source producer | Language adapter returns its explicit unsupported-shape error |
| Producer and language budgets | Each owner enforces its own limit |
| Private language context or operational authority added to the producer plan | Producer schema rejects the extra fields |

The records and subject are synthetic. A subject label in CompileContext is not
authentication. These tests exercise source binding and owner composition;
they do not certify semantic resolution, durable-store recovery, live inference,
HAT Grant checks, socket transport or a deployed application. The source
conformance suite separately exercises filesystem restoration and replay, and
the ignored actual-owner inference test still requires an accepted registry.
Do not count these tests as that inference acceptance.
