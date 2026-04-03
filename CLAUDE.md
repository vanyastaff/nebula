# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

---

## Project Overview

**Nebula** is a modular, type-safe workflow automation engine (like n8n/Zapier) built in Rust 1.93+. Workflows are DAGs of composable actions with built-in retries/error handling, extensible via plugins. Alpha: core crates stable, execution engine + credential system in active development. 26-crate workspace with strict one-way layer dependencies.

---

## Architecture

### Layer system (enforced by `cargo deny`)

```
API layer          api · webhook · (auth — RFC, not yet in workspace)
  ↑
Exec layer         engine · runtime · storage · sdk
  ↑
Business layer     credential · resource · action · plugin
  ↑
Core layer         core · validator · parameter · expression · memory · workflow · execution

Cross-cutting      log · system · eventbus · telemetry · metrics · config · resilience · error
(importable at any layer)
```

Arrows mean "depends on". No upward dependencies — enforce via `deny.toml`.

Each business/exec crate may have a companion proc-macro sub-crate (e.g., `crates/action/macros`, `crates/error/macros`, `crates/credential/macros`). These are part of their parent crate's public API surface.

### Data flow

```
Trigger (webhook/cron/event)
  → Engine resolves workflow DAG
    → Runtime schedules nodes (topological order)
      → Each node: Action::execute(Context) → serde_json::Value
        → Context provides: credentials (encrypted), resources, parameters, logger
          → Cross-crate signals via EventBus (e.g., CredentialRotatedEvent)
```

### Key patterns
- **`serde_json::Value`** is the universal data type — workflow data, action I/O, config, expressions
- **`Context` trait** for DI into actions — credentials, resources, logger injected, never constructed
- **`EventBus<E>`** for cross-crate signals — prevents circular deps (especially credential↔resource)
- **`NebulaError<E>`** with `Classify` trait — typed errors in libs (`thiserror`), `anyhow` in binaries
- **`NodeId`** = graph position; **`ActionKey`** = action type identity — multiple nodes can share an action

### Desktop app
Tauri-based desktop surface in `apps/desktop/` — currently in development.

---

## MCP Servers

### Serena (LSP-backed code intelligence)

Project config: `.serena/project.yml`. **Must activate at session start:**
```
activate_project("nebula")
```
Provides: `find_symbol`, `find_referencing_symbols`, `get_symbols_overview`, `rename_symbol`, `replace_symbol_body`, `insert_before/after_symbol`. Macro-generated symbols (e.g. `NodeId` from `define_id!`) may not be indexed.

### crates.io MCP

Search crates, check dependencies, audit vulnerabilities, compare alternatives — no setup needed.

---

## Key Commands

```bash
# Fast local check (use this by default)
cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace

# Single crate (fastest iteration)
cargo check -p nebula-<crate> && cargo nextest run -p nebula-<crate>

# Full validation (before PR)
cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace && cargo test --workspace --doc && cargo deny check

# Compose API contract
cargo bench --no-run -p nebula-resilience

# Context file budgets
bash .claude/validate.sh
```

`cargo nextest run` replaces `cargo test` — parallel execution, better output. Doctests run separately (`cargo test --doc`) because nextest doesn't support them.

### Taskfile (task runner)

`Taskfile.yml` wraps common workflows — run `task --list` to see all tasks. Key groups:

```bash
task db:up          # Start local Postgres via Docker Compose
task db:migrate     # Run pending sqlx migrations (requires DATABASE_URL)
task db:prepare     # Regenerate sqlx offline query data after schema changes
task db:reset       # Drop + recreate + re-migrate (destructive)

task desktop:dev    # Start Tauri dev mode (from apps/desktop/)
task desktop:build  # Build Tauri app for distribution

task obs:up         # Start Jaeger + OTEL collector for local tracing
```

Database URL for local dev is in `deploy/.env` (copy from `deploy/.env.example`). sqlx runs in offline mode in CI — run `task db:prepare` after any query changes.

---

## Context System Protocol

Workspace context lives in `.claude/` — do not re-derive what is already documented there.

| File | Token budget | Contents |
|------|-------------|----------|
| `.claude/ROOT.md` | 300 | Workspace index, crate layers, conventions |
| `.claude/decisions.md` | 500 | Cross-cutting architectural decisions (why, not what) |
| `.claude/pitfalls.md` | 300 | Global traps and non-obvious constraints |
| `.claude/active-work.md` | 200 | Current work, blocked areas, migration state |
| `.claude/crates/{name}.md` | 500 each | Per-crate invariants, traps, non-obvious decisions |
| `.claude/plans/{date}-{slug}.md` | — | Implementation plans for multi-step work (date-prefixed) |
| `.claude/research/{slug}.md` | — | Background research, competitive analysis, findings |

**Include:** invariants, non-obvious design decisions, active constraints, traps that burned someone.

**Exclude:** pub types, function signatures, Cargo.toml deps (read the code), git history.

### Update Rules

- **After modifying a crate**: update `.claude/crates/{name}.md` before the session ends.
  - Changed invariants, decisions, or traps → update content.
  - Only implementation details changed → add `<!-- reviewed: YYYY-MM-DD -->` at bottom.
- **After global decisions change**: update `decisions.md` or `pitfalls.md`.
- **After work state changes**: update `active-work.md`.
- A Stop hook enforces this: completion is blocked if a crate was modified but its context file was not touched.
