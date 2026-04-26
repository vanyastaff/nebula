# Treadle — Structure Summary

## Crate count

1 crate (`treadle = "0.2.0"`, single package, no workspace).

## Layout

```
treadle/
├── src/
│   ├── lib.rs           (269 lines — public re-exports)
│   ├── workflow.rs      (2418 lines — Workflow, WorkflowBuilder, executor)
│   ├── stage.rs         (833 lines — Stage trait, StageOutcome, StageState, etc.)
│   ├── state_store/
│   │   ├── mod.rs       (226 lines — StateStore trait)
│   │   ├── memory.rs    (461 lines — MemoryStateStore)
│   │   └── sqlite.rs    (611 lines — SqliteStateStore, feature-gated)
│   ├── status.rs        (419 lines — PipelineStatus, StageStatusEntry)
│   ├── work_item.rs     (174 lines — WorkItem trait)
│   ├── event.rs         (269 lines — WorkflowEvent enum)
│   └── error.rs         (144 lines — TreadleError, Result)
├── tests/
│   └── integration.rs   (448 lines — 8 integration tests)
├── examples/
│   └── basic_pipeline.rs (239 lines)
├── docs/
│   ├── dev/             (0001–0011 phase implementation plans, ~17.9K lines total)
│   ├── design/          (v2 design document under review)
│   └── related-projects.md
├── Cargo.toml
├── CHANGELOG.md
└── README.md
```

## Total LOC

- Source: ~5,824 lines (Rust)
- Tests: ~448 lines (integration) + ~800 lines (inline unit tests estimated from 149 unit test count)
- tokei: not available on this system

## Top-10 dependencies

1. `petgraph = "0.8"` — DAG data structure and algorithms
2. `tokio = { version = "1", features = ["full"] }` — async runtime
3. `async-trait = "0.1"` — async fn in trait (proc macro)
4. `serde = { version = "1", features = ["derive"] }` — serialization
5. `serde_json = "1"` — JSON state storage
6. `thiserror = "2"` — derive macro for error types
7. `chrono = { version = "0.4", features = ["serde"] }` — timestamps in state
8. `tracing = "0.1"` — structured logging / spans
9. `rusqlite = { version = "0.38", features = ["bundled"] }` — SQLite (optional, default feature)
10. `tokio = { version = "1", features = ["full", "test-util"] }` — (dev-dep, test utilities)

## Test count

- 149 unit tests (inline)
- 8 integration tests
- 9 doc tests
- Total: 166 (as reported in CHANGELOG.md)
