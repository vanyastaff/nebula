# acts — Structure Summary

## Crate Count and Layout

The workspace contains **10 Cargo.toml files** across 7 members (excluding 3 excluded example plugins):

| Crate | Purpose |
|-------|---------|
| `acts` | Core engine library — all workflow logic |
| `store/sqlite` (`acts-store-sqlite`) | SQLite persistence plugin |
| `store/postgres` (`acts-store-postgres`) | PostgreSQL persistence plugin |
| `plugins/state` | Redis state read/write package plugin |
| `plugins/http` (`acts-http`) | HTTP request package plugin |
| `plugins/shell` | Shell/nushell/bash/powershell command plugin |
| `examples/plugins/*` | Example code for each plugin (excluded from build) |

This is a **monolith core** design: the entire workflow engine, scheduler, package registry, expression engine, event system, and in-memory store all live in the `acts` crate (`acts/src/`). Plugins extend the engine but do not split the core.

## Layer Separation

No formal layering by crate (unlike Nebula's 26-crate stack). Logical modules within `acts/src/`:

- `engine.rs` + `builder.rs` — public API entry points
- `scheduler/` — runtime process/task dispatch, context, queue, NodeTree
- `model/` — YAML-parseable data structures (Workflow, Step, Branch, Act)
- `package/` — built-in package registry + ActPackageFn/ActPackage traits
- `plugin/` — ActPlugin trait (system-level integrations)
- `env/` — JavaScript runtime (rquickjs), user var extensions
- `store/` — DbCollection trait + in-memory default store
- `event/` — Emitter, Channel, Message, EventAction
- `cache/` — Moka LRU cache for in-flight processes
- `export/` — public API surface: Executor, Channel, Extender
- `utils/` — helpers, constants

## Top Dependencies

| Crate | Version | Role |
|-------|---------|------|
| `tokio` | 1.44 | Async runtime |
| `rquickjs` | 0.9 | JavaScript expression engine (QuickJS embedding) |
| `serde_yaml` | 0.9 | Workflow model parsing |
| `serde_json` | 1.0 | JSON I/O, params, Vars storage |
| `thiserror` | 2 | Error type generation |
| `moka` | 0.12 | Async LRU cache for in-flight procs |
| `inventory` | 0.3 | Compile-time package registration |
| `jsonschema` | 0.30 | Package parameter validation |
| `tracing` | 0.1 | Structured logging |
| `async-trait` | 0.1 | Async trait objects |
| `globset` | 0.4 | Channel message pattern matching |
| `sea-query` | 0.32 | SQL query builder (store plugins) |
| `sqlx` / `rusqlite` | various | Store plugin DB drivers |

## LOC Estimate

`acts/src/` contains 174 `.rs` files with approximately **31,796 total lines** (wc -l count). Of these, 55 files are in `tests/` subdirectories.

## Test Count

55 test files. Tests are inline modules or files under `scheduler/tests/`, `model/tests/`, `store/tests/`, `cache/tests/`, `env/tests/`, `export/tests/`. No separate testing crate.

## Notable Observations

1. Single-crate core means all internal types are visible to each other — no intra-workspace API boundary enforcement.
2. Package plugins compile as separate workspace members but are Rust crates, not WASM or dynamic libraries.
3. The `inventory` crate enables zero-config compile-time package registration — packages register themselves via `inventory::submit!()`.
4. No derive macros of the project's own — uses `serde` derive + `strum` directly.
5. `rquickjs` embeds QuickJS JavaScript engine — expressions are JavaScript, not a custom DSL.
