# Structure Summary — duroxide

## Crate Count and Layout

**2 workspace members:**
1. `duroxide` (root) — the entire framework: runtime, providers, client, testing infrastructure
2. `sqlite-stress` — standalone benchmark/stress tool

Single-crate design philosophy: all runtime, storage, and client code ships in one crate. Extension via feature flags.

## Feature Flags

| Flag | Purpose |
|------|---------|
| `sqlite` | Bundled SQLite provider (optional) |
| `provider-test` | Generic provider conformance test suite |
| `test` | `provider-test` + `test-hooks` |
| `test-hooks` | Fault injection for integration testing |
| `replay-version-test` | V2 event types for replay extensibility tests |

## Key Source Modules

| Module | Role |
|--------|------|
| `src/lib.rs` | Core types: `Event`, `EventKind`, `Action`, `ErrorDetails`, `OrchestrationContext`, `DurableFuture`, `RetryPolicy` |
| `src/runtime/mod.rs` | `Runtime`, `OrchestrationHandler`, `ActivityHandler`, `RuntimeOptions`, dispatcher startup |
| `src/runtime/replay_engine.rs` | Deterministic orchestration replay per turn |
| `src/runtime/registry.rs` | Versioned `Registry<H>` for orchestrations and activities |
| `src/runtime/dispatchers/` | `OrchestrationDispatcher` + `WorkDispatcher` |
| `src/runtime/observability.rs` | `metrics` facade integration + `tracing-subscriber` setup |
| `src/providers/mod.rs` | `Provider` trait, `WorkItem`, `OrchestrationItem`, `TagFilter`, `SemverRange` |
| `src/providers/sqlite.rs` | SQLite provider implementation (sqlx, 12 migrations) |
| `src/provider_validation/` | 20 modules of generic conformance tests |
| `src/client/mod.rs` | `Client` public API |

## Top Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | 1.x, full | Async runtime |
| `async-trait` | 0.1.x | `dyn Handler` trait compatibility |
| `serde` + `serde_json` | 1.x | Event/WorkItem serialization |
| `tracing` + `tracing-subscriber` | 0.1.x / 0.3.x | Structured logging |
| `metrics` | 0.24 | Zero-cost metrics facade |
| `semver` | 1.x | Versioned orchestration registry |
| `sqlx` | 0.8 (optional, `sqlite` feature) | SQL queries + migrations |
| `futures` | 0.3 | `join_all`, `select_biased!` |
| `tokio-util` | 0.7 | `CancellationToken` |

## Lines of Code

`tokei` not installed; estimated from `wc -l src/**/*.rs tests/**/*.rs` total ≈ 44,924 lines across all Rust source files. Docs in `docs/` total approximately 15 large markdown files plus 29 proposals — extremely well-documented for a project of this size and age.

## Test Coverage

Provider validation suite: 20 modules covering atomicity, cancellation, KV store, deletion, locking, sessions, tag filtering, poison messages, and more. E2E samples in `tests/e2e_samples.rs`. Unit tests in `tests/unit_tests.rs`. Dedicated replay engine test scenarios in `tests/replay_engine/`.

## Migration Count

12 SQL migrations from initial schema (instances/executions/history/queues) to current (KV delta table, sessions, activity tags, custom status, attempt_count, pinned_version).
