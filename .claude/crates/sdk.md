# nebula-sdk
Re-exports of core crates — single entry point for plugin/action authors.

## Invariants
- Thin facade. Primarily re-exports nebula-action, nebula-core, nebula-credential, nebula-parameter, nebula-plugin, nebula-validator, nebula-workflow.

## Key Decisions
- `use nebula_sdk::prelude::*` gives everything for plugin authoring (StatelessAction, ActionContext, action_key!, Plugin, descriptors, Value, json!, Serialize/Deserialize).
- `testing` feature adds `tokio` re-export.

## Traps
- `ActionContext` is re-exported as alias (avoids conflict with `anyhow::Context`).
- `simple_action!` macro uses `ProcessAction` — verify trait name is current.
- `params!` references `nebula_parameter::values::ParameterValues` — may change.

## Relations
- Re-exports: nebula-action, nebula-core, nebula-credential, nebula-parameter, nebula-plugin, nebula-validator, nebula-workflow, anyhow, async-trait, serde, serde_json, thiserror.

<!-- reviewed: 2026-04-08 — expanded prelude with StatelessAction, ActionDependencies, ActionContext, action_key!, descriptors -->
