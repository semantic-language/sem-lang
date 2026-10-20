# Source binding conformance

This isolated source test consumer uses product crates within this repository and external dependencies from crates.io. It proves source integration, not independent package publication. No private registry configuration is required.

```sh
cargo test --locked --manifest-path conformance/source/Cargo.toml
cargo clippy --locked --manifest-path conformance/source/Cargo.toml --all-targets -- -D warnings
```

The scenario starts distinct processes against an isolated filesystem repository.
It explicitly promotes for a supplied subject and decision, adopts a definition
and presentation binding, and checkpoints exact memory/runtime references. After
changing both heads it exits. A second process uses `Engine::restore`, checks
complete memory equality and the original binding, replays the decision without
mutation and checks the separate provider object. Corruption rejects restoration.
No UI, provider runtime, HAT or network is required.

Promotion requests pin the foundation commit revision as well as their semantic
input digest. The build must contain `zixcel-graph` with an empty feature set,
without `graph`, `redb`, an execution host or an async runtime. Its receipt store is the sole
promotion commit authority. This does not assert completion of the still-pending
PacketReducer and Repository-head migration.

This test does not prove execution-grant enforcement, extension uninstall or renderer
availability. Consumers must test those integration gates separately. The scenario
only establishes exact semantic/reference adoption, restart, replay and corruption
behavior; see the [public introduction](../../docs/getting-started.md) for the
source API boundary.
