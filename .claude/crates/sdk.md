# nebula-sdk
Re-exports of core crates — single entry point for plugin/action authors.
Also owns the in-process `TestRuntime` harness for running single actions end-to-end.

## Invariants
- Thin facade. Primarily re-exports nebula-action, nebula-core, nebula-credential, nebula-parameter, nebula-plugin, nebula-validator, nebula-workflow.
- `runtime::TestRuntime` mirrors `ActionRegistry::register_*` API shape: one method per action kind (`run_stateless`, `run_stateful`, `run_poll`, `run_webhook`) — not a single `.run()` to avoid overlapping blanket impls in stable Rust.

## Key Decisions
- `use nebula_sdk::prelude::*` gives everything for plugin authoring: all DX trait families (Stateful, Paginated, Batch, Poll, Webhook), adapters, macros (`impl_paginated_action!`, `impl_batch_action!`), testing utilities (TestContextBuilder, Spy*), and the `TestRuntime`. `TransactionalAction` + `impl_transactional_action!` were removed 2026-04-10 (M1 — see action.md).
- `testing` feature adds `tokio` re-export.
- `TestRuntime::new(ctx)` consumes a `TestContextBuilder` (from nebula-action::testing), exposes `.with_stateful_cap(u32)` and `.with_trigger_window(Duration)` knobs, and each `run_*` is terminal (consumes self).
- `RunReport { kind, output, iterations, duration, emitted, note }` is the single shape returned by all `run_*` methods — uniform consumption in examples/tests.

## Traps
- `ActionContext` is re-exported as alias (avoids conflict with `anyhow::Context`).
- `simple_action!` macro uses `ProcessAction` — verify trait name is current.
- `params!` references `nebula_parameter::values::ParameterValues` — may change.
- `TestRuntime::run_poll` uses `tokio::select!` inside `PollTriggerAdapter`, which only checks cancellation between sleep/fetch — in-flight HTTP requests are NOT interrupted, so actual run duration can exceed the configured trigger window by one fetch latency.
- Grace period for poll trigger stop is 5s (`TRIGGER_STOP_GRACE` in `runtime.rs`). If start task hasn't exited by then, `note` captures it but the report is still returned.

## Relations
- Re-exports: nebula-action, nebula-core, nebula-credential, nebula-parameter, nebula-plugin, nebula-validator, nebula-workflow, anyhow, async-trait, serde, serde_json, thiserror.

<!-- reviewed: 2026-04-08 — expanded prelude with StatelessAction, ActionDependencies, ActionContext, action_key!, descriptors -->
<!-- reviewed: 2026-04-10 — added src/runtime.rs (TestRuntime + RunReport), expanded prelude with all DX traits + adapters + macros + Spy* + TestContextBuilder; examples/ now depends on nebula-sdk only -->
<!-- reviewed: 2026-04-10 — M1: removed TransactionalAction + impl_transactional_action! re-exports from prelude (trait deleted in nebula-action). -->
<!-- reviewed: 2026-04-11 — `src/runtime.rs` imports consolidated to a single `use nebula_action::{ ... }` block off the crate root — upstream deleted the `nebula_action::handler::X` alias surface after the post-audit re-export purge. No prelude changes, no API surface changes. -->
