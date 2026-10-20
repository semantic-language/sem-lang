# sem-lang-inference-client

Validate semantic inference requests and interpret provider text as typed, unadopted results.

## What you can do

- Prepare requests for exact adopted semantic targets.
- Preserve repository and revision fences when interpreting output.

## Current scope

The caller retains semantic authority. Physical inference is supplied through the declared Zixcel dependency; protocol changes do not trigger automatic restart or migration.

Package distribution is not activated by this documentation. Use the checked-in source and the declared dependency versions; published availability must be verified separately.

## Getting started

Install Rust 1.97 or newer. Product dependencies resolve inside the sem-lang workspace; Zixcel dependencies come from crates.io. Run from the workspace root:

```sh
cargo test --locked -p sem-lang-inference-client
```

## Documentation and source

[Usage guide](docs/getting-started.md)

[Detailed documentation](docs) · [Implementation and public interfaces](src) · [Verification cases](tests) · [License](LICENSE) · [Attribution notices](NOTICE)

## Source ownership

This adapter belongs to this semantic workspace. It is source-owned (`publish = false`); no private registry is required. LICENSE and NOTICE retain the original attribution.
