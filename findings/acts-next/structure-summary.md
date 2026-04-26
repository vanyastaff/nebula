# acts-next (luminvent/acts) — Structure Summary

## Repository Identity

- **URL:** https://github.com/luminvent/acts
- **Fork of:** https://github.com/yaojianpin/acts (forked 2025-03-31)
- **Stars:** 2 (fork), upstream has 61
- **License:** Apache-2.0
- **Organization:** Luminvent (luminvent GitHub org)
- **Issues:** Disabled on fork; upstream yaojianpin/acts has 9 total, 3 open

## Crate Count and Structure

7 workspace members:

| Member | Crate name | Role |
|--------|-----------|------|
| `acts/` | `acts` v0.17.2 | Core engine (only published crate) |
| `store/sqlite/` | `acts-store-sqlite` | SQLite persistence plugin |
| `store/postgres/` | `acts-store-postgres` | PostgreSQL persistence plugin |
| `plugins/state/` | `acts-state` | Redis state package plugin |
| `plugins/http/` | `acts-http` | HTTP request package plugin |
| `plugins/shell/` | `acts-shell` | Shell command plugin (bash/nushell/powershell) |
| `examples/plugins/*` | example crates | Excluded from default build |

No umbrella re-export crate. Consumers add `acts` directly. Plugin crates are opt-in.

## Layer Model

No formal layer boundaries. All engine logic is in one crate (`acts/`) with internal modules:

```
acts/src/
├── model/       — Workflow, Branch, Step, Act data types
├── scheduler/   — Runtime, Process, Task, Scheduler, NodeTree
├── package/     — ActPackageFn, ActPackage, built-in packages
├── store/       — DbCollection trait, MemStore, Store
├── cache/       — Moka LRU process cache
├── env/         — JavaScript environment, ActUserVar, modules
├── event/       — EventAction, Action, Message, Emitter
├── export/      — Executor, Channel, Extender (public API)
├── plugin/      — ActPlugin trait
└── utils/       — constants, ids, time, JSON helpers
```

## Line of Code

- Total Rust source files: 228
- Total lines (excluding tests): approximately 18,890
- Test files: ~65

## Key Dependencies

From `acts/Cargo.toml`:
- `rquickjs` 0.9 — JavaScript engine (QuickJS binding)
- `tokio` 1.44 — async runtime
- `serde` / `serde_json` / `serde_yaml` — serialization
- `inventory` 0.3.20 — compile-time package registration
- `moka` 0.12 — LRU process cache
- `jsonschema` 0.30.0 — JSON Schema validation for package params
- `thiserror` 2 — error types
- `tracing` 0.1 — structured logging
- `toml` 0.8.22 — config file parsing (acts.toml)
- `chrono`, `nanoid`, `regex`, `globset` — utilities

## What Luminvent Changed vs Upstream

Luminvent's fork adds 5+ meaningful commits on top of yaojianpin/acts v0.17.x:

1. **`SetVars` / `SetProcessVars` EventAction** (Marc-Antoine ARNAUD, 2025-08-29 and earlier): Two new `EventAction` variants to mutate task/process variables without completing a task. Exposed as `executor.act().set_task_vars()` and `executor.act().set_process_vars()`.
2. **`expose_do_action` method** (2025-04-02): Made `do_action` public so callers can dispatch arbitrary `EventAction` directly.
3. **Bug fix — process state propagation** (2025-04-23): Fix for Issue #12 (`keep_processes` config, process root task completion state).
4. **API naming cleanup** (2025-03-30–04): Renamed `sch` → `scheduler`, `proc` → `process`, used `strum` for EventAction string conversion, renamed `EventAction::Update` → `SetProcessVars`.
5. **`keep_processes` config option** (2025-04-23): Allows retaining process/task records after completion for inspection.
6. **`PageData` export** (2025-04-10): Exposed `PageData<T>` from the library public API.
7. **`Process` getter** (2025-04-14): Added `getter` method to retrieve `Process` instance.

These are incremental ergonomic and bug-fix improvements, not a redesign. The core architecture is identical to upstream yaojianpin/acts.
