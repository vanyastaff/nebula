# RunnerQ — Structure Summary

## Crate count
2 crates: `runner_q` (root) + `runner_q_redis` (separate workspace member)

## Rust source files
31 `.rs` files total (27 in `runner_q`, 4 in `runner_q_redis`)

## Module layout (runner_q)
- `src/lib.rs` — public re-exports
- `src/config.rs` — WorkerConfig
- `src/activity/` — Activity, ActivityHandler, ActivityContext, ActivityError
- `src/queue/` — internal ActivityQueueTrait (not public)
- `src/runner/` — WorkerEngine, WorkerEngineBuilder, ActivityExecutor, MetricsSink
- `src/storage/` — QueueStorage, InspectionStorage, Storage traits + PostgresBackend
- `src/observability/` — QueueInspector, models, Axum UI

## Key dependencies (runner_q)
- `tokio` (full) — async runtime
- `sqlx` (postgres, chrono, uuid, json) — PostgreSQL (optional feature)
- `async-trait` — dyn async traits
- `serde` / `serde_json` — JSON payload serialization
- `uuid` — activity IDs
- `chrono` — timestamps
- `thiserror` — error types
- `tracing` + `tracing-subscriber` — structured logging
- `tokio-util` — CancellationToken
- `axum` (optional) — HTTP observability UI
- `tower-http` (optional) — CORS for axum

## Key dependencies (runner_q_redis)
- `runner_q` (path dep) — re-uses all traits
- `redis` (tokio-comp, connection-manager)
- `bb8-redis` — connection pool

## LOC estimate
Tokei unavailable in environment. Rough manual count from file sizes:
- `src/runner/runner.rs`: ~1600 lines
- `src/storage/postgres/mod.rs`: ~1663 lines
- `src/activity/activity.rs`: ~465 lines
- All other files: ~500 lines combined
- `runner_q_redis`: ~600 lines
Total: ~4800 lines

## Test count
No `#[test]` blocks found in src/. Tests would be in a separate `tests/` directory (not included in --depth 50 clone). 5 example files in `examples/`.

## Version history (tags)
- runner_q-v0.6.4 (current)
- v0.6.3, v0.6.2, v0.6.1, v0.6.0 — active iteration
- runner_q_redis-v0.1.1, runner_q_redis-v0.1.0 — Redis backend recently extracted
