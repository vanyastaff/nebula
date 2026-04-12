# nebula-sdk

Single entry point for plugin / action authors. Thin facade that re-exports the
public surface of `nebula-action`, `-core`, `-credential`, `-parameter`,
`-plugin`, `-validator`, `-workflow`, plus the in-process `TestRuntime` harness.

## Invariants

- `use nebula_sdk::prelude::*` gives everything needed for plugin authoring: all DX trait families (Stateful, Paginated, Batch, Poll, Webhook), adapters, `impl_paginated_action!` / `impl_batch_action!` macros, testing utilities (`TestContextBuilder`, `Spy*`), and `TestRuntime`.
- `runtime::TestRuntime` mirrors `ActionRegistry::register_*` — one method per action kind (`run_stateless`, `run_stateful`, `run_poll`, `run_webhook`). No single `.run()` — avoids overlapping blanket impls.
- `TestRuntime::new(ctx)` consumes a `TestContextBuilder`, exposes `.with_stateful_cap(u32)` and `.with_trigger_window(Duration)`. Each `run_*` is terminal (consumes self).
- `RunReport { kind, output, iterations, duration, emitted, note }` is the single shape returned by all `run_*` methods.
- `testing` feature adds a `tokio` re-export.

## Traps

- `ActionContext` is re-exported as an alias to avoid conflict with `anyhow::Context`.
- `TestRuntime::run_poll` uses `tokio::select!` inside `PollTriggerAdapter`, which only checks cancellation between sleep / fetch — in-flight HTTP requests are **not** interrupted, so real run duration can exceed the configured trigger window by one fetch latency.
- Poll-trigger stop grace is 5 s (`TRIGGER_STOP_GRACE` in `runtime.rs`). If the start task hasn't exited by then, `note` captures it but the report is still returned.

## Relations

Re-exports: `nebula-action`, `nebula-core`, `nebula-credential`, `nebula-parameter`, `nebula-plugin`, `nebula-validator`, `nebula-workflow`, `anyhow`, `async-trait`, `serde`, `serde_json`, `thiserror`.
<!-- reviewed: 2026-04-12 -->
