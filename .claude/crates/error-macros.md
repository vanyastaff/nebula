# nebula-error-macros

Proc-macros for the `nebula-error` crate.

## Layer

Foundation — companion proc-macro crate for nebula-error.

## Key Design Decisions

- **Separate crate** — Rust requires proc-macros in their own crate. Re-exported from nebula-error via `derive` feature flag.
- **Stub only** — `#[derive(Classify)]` currently generates an empty impl. Full implementation in Task 11.

## Invariants

- Must not depend on nebula-error (circular dependency). Only uses syn/quote/proc-macro2.

<!-- reviewed: 2026-03-26 -->
