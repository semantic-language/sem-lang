# Private owner composition fixture

A separate, source-only integration executable, outside the foundational workspace.
It reuses public HAT input validation/transport and attaches only the explicitly
provided private language context. Foundation crates have no HAT/service dependency.

Build with `cargo build --locked --manifest-path fixtures/source-owner-composition/Cargo.toml`.
Run its binary with an absolute synthetic owner JSON path and a new socket path in
an owner-only temporary directory. Owner JSON is `{plan, context, budget}`; plan is
sealed by the public source producer and contains no language context. Public
`describe` omits source values and private context. `bind` composes the validated
SourceTree into StructuralInput. The execution consumer still verifies subject and current semantic
revision at admission. This fixture installs no provider into a real environment.
