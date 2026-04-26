# Structure Summary — deltaflow

## Crates

| Crate | Role |
|-------|------|
| `deltaflow` (root) | Core library: Step trait, Pipeline builder, retry, recorder, runner (sqlite-gated), scheduler (sqlite-gated) |
| `deltaflow-harness` | Dev companion: web visualizer over axum, serves pipeline graph JSON |

## Source files

| File | Size | Purpose |
|------|------|---------|
| `src/pipeline.rs` | ~44 KB | Pipeline builder, typestate chain, SpawnRule, builders, BuiltPipeline |
| `src/runner/sqlite_store.rs` | ~12 KB | SQLite implementation of TaskStore |
| `src/runner/executor.rs` | ~7 KB | Runner + RunnerBuilder, semaphore-based concurrency |
| `src/retry.rs` | ~4.5 KB | RetryPolicy enum (None/Fixed/Exponential) |
| `src/sqlite.rs` | ~4 KB | SqliteRecorder, inline DDL schema |
| `src/scheduler/builder.rs` | ~3.7 KB | SchedulerBuilder |
| `src/runner/erased.rs` | ~2.5 KB | ErasedPipeline trait, SpawnedTask |
| `src/recorder.rs` | ~2.4 KB | Recorder trait, NoopRecorder, RunId/StepId/RunStatus/StepStatus |
| `src/runner/store.rs` | ~2.2 KB | TaskStore trait, StoredTask, TaskError, TaskId |
| `src/step.rs` | ~1.4 KB | Step trait, StepError |
| `src/lib.rs` | ~2.8 KB | Re-exports, feature gates |

## LOC

tokei not available in environment. Rough estimate from file sizes: approximately 1,200–1,500 lines of Rust across the main crate.

## Key dependencies

- `async-trait = "0.1"` — required for `async fn` in traits (pre-RPITIT workaround)
- `thiserror = "2"` — error enums
- `tokio = "1"` — async runtime, semaphores, sleep
- `anyhow = "1"` — error boxing in `StepError`
- `serde = "1"` / `serde_json = "1"` — serialization for task queue boundary
- `sqlx = "0.8"` (optional, sqlite feature) — SQLite via runtime-tokio
- `chrono = "0.4"` — timestamps for task scheduling
- `tracing = "0.1"` — minimal logging in scheduler

## Feature flags

- `default = []` — in-memory only, no persistence
- `sqlite` — enables `runner`, `scheduler`, `SqliteRecorder`, `SqliteTaskStore`

## Test count

Unit tests present in `src/retry.rs` (7 tests). Integration tests in `tests/`. No count without tokei.
