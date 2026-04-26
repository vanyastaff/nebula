# runtara-core — Architectural Issues

Total issues found: 4 (3 closed, 1 open). Repo has fewer than 100 total issues, so the ≥3 citation requirement does not apply strictly, but all issues are cited below.

## Issue #2 — [OPEN] Time-travel debugger for Runtara SDK
**URL:** https://github.com/runtarahq/runtara/issues/2  
**Author:** volodymyrrudyi  
**Labels:** enhancement, good first issue  
**Reactions:** 0  

Proposes a `TimeTravel` API in `runtara-management-sdk` for post-mortem debugging of durable workflows: inspect state at any checkpoint, step forward/back through execution history, set breakpoints on checkpoints/signals/errors, and diff state between two execution points.

Architectural significance: requires richer event storage in runtara-core (a new `instance_events` table with state snapshots), protocol extensions for debug-attach/step/inspect, and a new debugger module in the SDK. The issue notes that runtara-core already stores checkpoints; the feature exposes them usefully.

Comparison note: issue explicitly benchmarks against Temporal (event history, state inspection, step-forward) and Restate, claiming step-backward and breakpoints as differentiators Runtara would uniquely offer.

## Issue #3 — [CLOSED] WASM runner support in runtara-environment
**URL:** https://github.com/runtarahq/runtara/issues/3  
**Author:** volodymyrrudyi  
**Labels:** enhancement  
**Reactions:** 0  

Proposed adding WASM runner to `runtara-environment`. Comment: "PoC worked flawlessly." Now implemented in `crates/runtara-environment/src/runner/wasm.rs` — wasmtime CLI runner with WASI support, default production runner.

## Issue #21 — [CLOSED] Non-durable scenarios/workflows
**URL:** https://github.com/runtarahq/runtara/issues/21  
**Author:** volodymyrrudyi  
**Labels:** enhancement  
**Reactions:** 0  

Detailed design proposal for compile-out durability: scenario-level `durable: false` and per-step `durable: false` flags. Renames `#[durable]` macro to `#[resilient]` with a `durable` attribute. When `durable = false`, no checkpoint read/write, no SDK sleep calls, but retry logic and error classification are preserved. Now fully implemented in the codebase (`crates/runtara-sdk-macros/src/lib.rs`).

## Issue #4 — [CLOSED] Runtara UI
**URL:** https://github.com/runtarahq/runtara/issues/4  
**Author:** volodymyrrudyi  
**Labels:** enhancement  
**Reactions:** 0  

Visual workflow editor now implemented in `crates/runtara-server/frontend/`. Embedded via `embed-ui` feature flag in `runtara-server/Cargo.toml`.
