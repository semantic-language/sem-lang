# sem-lang

A Rust workspace for semantic representation, controlled language, validation,
local memory and explicit execution boundaries. Meaning does not grant authority
to perform an external operation. UI and provider integrations remain consumers.

## Explore

- [Getting started](docs/getting-started.md)
- [Language facade](crates/sem-lang)
- [Core representation](crates/sem-lang-core)
- [Frontend](crates/sem-lang-frontend)
- [Local filesystem store](crates/sem-lang-store-fs)
- [Inference boundary](crates/sem-lang-inference-client)
- [Independent source consumer](fixtures/source-consumer)
- [Actual producer composition acceptance](fixtures/source-owner-composition)
- [Independent Coela producer acceptance](docs/coela-producer-acceptance.md)
- [Dependency policy](docs/dependency-updates.md)

## Verify

With the existing Rust 1.97 and Node toolchain, run
`node scripts/document-links.mts` and `node scripts/verify.mts`.
It checks source boundaries, feature configurations, Clippy, conformance and an
independent consumer by executing its snapshot/error assertions. Public CI also
executes the pinned source producer's binding and language-composition tests.
The actual-owner inference test needs an accepted inference
registry; an ignored case is not successful integration acceptance. No model is
downloaded by the default source verification.

## License and support

HAT Inc.'s sem-lang source is offered under [MPL 2.0](LICENSE).
[License scope](docs/license-scope.md) explains preserved earlier grants and
third-party notices. Optional paid support may be provided by HAT Inc. under a
separate agreement; a support contract is not required to use the MPL source.
No support service, response time or warranty is promised by this repository.

Crates remain source-owned (`publish = false`); source availability does not
imply a registry release or service deployment.
Product-independent examples use synthetic state.

Source tooling uses Node 24.15+ and pnpm 10.29.3. Install its pinned development
dependencies with `pnpm install --frozen-lockfile --ignore-scripts`. Rust library
consumers do not require these tooling dependencies.
