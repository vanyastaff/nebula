# nebula-sdk
Re-exports of core crates for plugin/action authors — the single entry point for external integrations.

## Invariants
- Thin facade only. Contains minimal own logic; primarily re-exports nebula-action, nebula-core, nebula-credential, nebula-macros, nebula-parameter, nebula-plugin, nebula-validator, nebula-workflow.

## Key Decisions
- Action authors import from `nebula_sdk::prelude::*` instead of depending on individual crates.
- `testing` feature adds `tokio` re-export and a `testing` module with test utilities.
- Helper macros: `params!` (builds `FieldValues`), `workflow!` (DAG builder), `simple_action!` (closure-based action).

## Traps
- `params!` macro references `nebula_parameter::values::ParameterValues` — this type may change during parameter migration (RFC 0005). Verify it compiles after parameter changes.
- `simple_action!` macro uses `ProcessAction` trait — verify this trait name is current before referencing it in docs.
- Re-exports can shadow each other. If something is missing from `prelude::*`, check the individual re-exported crate directly.

## Relations
- Re-exports: nebula-action, nebula-core, nebula-credential, nebula-macros, nebula-parameter, nebula-plugin, nebula-validator, nebula-workflow, anyhow, async-trait, serde, serde_json, thiserror.

<\!-- reviewed: 2026-03-25 -->
