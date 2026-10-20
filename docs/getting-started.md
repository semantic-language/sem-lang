# Getting started with sem-lang

Compile controlled language into deterministic semantic outcomes while keeping meaning separate from external execution.

## Before you start

This is an experimental Rust language workspace. A complete semantic result does not authorize an external operation.

## First steps

Run from the repository root:

```sh
node scripts/verify.mts
```

## How to assess the result

- Use the existing Rust 1.97 and Node toolchain; no provider credential is needed for synthetic tests.
- The real-inference test requires a separately accepted inference registry and remains unexecuted without it.
- Inspect typed semantic results and diagnostics.
- Develop controlled-language resources and explicit repository context.

A passing source-level check establishes only what that check observes. Keep missing configuration, unavailable services and unverified deployment paths visible.

## Continue reading

[Repository overview](../README.md)

Source tooling uses Node 24.15+ and pnpm 10.29.3. Install its pinned development
dependencies with `pnpm install --frozen-lockfile --ignore-scripts`. Rust library
consumers do not require these tooling dependencies.
