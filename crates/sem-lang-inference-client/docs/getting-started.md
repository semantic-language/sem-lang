# Using sem-lang-inference-client

Validate semantic inference requests and interpret provider text as typed, unadopted results.

## Before you start

The caller retains semantic authority. Physical inference is supplied through the declared Zixcel dependency; protocol changes do not trigger automatic restart or migration.

## First steps

Run from the repository root:

```sh
cargo test --locked
```

## How to assess the result

- Prepare requests for exact adopted semantic targets.
- Preserve repository and revision fences when interpreting output.

A passing source-level check establishes only what that check observes. Keep missing configuration, unavailable services and unverified deployment paths visible.

## Continue reading

[Repository overview](../README.md)
