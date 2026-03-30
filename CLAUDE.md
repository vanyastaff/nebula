# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

---

## Project Overview

**Nebula** is a modular, type-safe workflow automation engine (like n8n/Zapier) built in Rust 1.93+. Workflows are DAGs of composable actions with built-in retries/error handling, extensible via plugins. Alpha: core crates stable, execution engine + credential system in active development. 26-crate workspace with strict one-way layer dependencies.

---

## Architecture

### Layer system (enforced by `cargo deny`)

```
API layer          api · webhook · auth
  ↑
Exec layer         engine · runtime · storage · macros · sdk
  ↑
Business layer     credential · resource · action · plugin
  ↑
Core layer         core · validator · parameter · expression · memory · workflow · execution

Cross-cutting      log · system · eventbus · telemetry · metrics · config · resilience · error
(importable at any layer)
```

Arrows mean "depends on". No upward dependencies — enforce via `deny.toml`.

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

## Active Constraints

- **No upward dependencies.** Use slim DTOs if needed; enforce via `deny.toml`.
- **`nebula-core` stays small.** It's imported everywhere — new ID types safe, trait changes cascade to 25+ crates.
- **Credentials always encrypted at rest.** AES-256-GCM; `SecretString` zeroizes on drop. `CredentialAccessor` injected via `Context`; no global lookups.
- **EventBus for all cross-crate signals.** In-memory, best-effort, no persistence. Never add direct imports to "fix" missing signals.
- **Actions use DI via `Context`.** Never construct runtime-managed types inside actions.
- **Parameter provider API is error-based.** `register_*()` returns `Result`; no panics.
- **`InProcessSandbox` only in Phase 2.** No OS-process/WASM isolation until Phase 3.

---

## Do Not Touch

- **`nebula-core`** without approval — cascades everywhere. Adding new ID types is safe; changing traits is not.
- **`crates/resilience/benches/compose.rs`** — it's a contract documenting the compose API.
- **`docs/crates/parameter/*.md`** — stale but kept for migration history; scheduled for update, not deletion.

---

## Code Navigation

Use code-review-graph MCP tools for code exploration.
- Use `get_review_context_tool` before reviewing changes
- Use `get_impact_radius_tool` to understand blast radius
- Prefer graph queries over reading full files

---

## Key Commands

```bash
# Fast local check (use this by default)
rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run --workspace

# Single crate (fastest iteration)
rtk cargo check -p nebula-<crate> && rtk cargo nextest run -p nebula-<crate>

# Full validation (before PR)
rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run --workspace && rtk cargo test --workspace --doc && rtk cargo deny check

# Compose API contract
rtk cargo bench --no-run -p nebula-resilience

# Context file budgets
bash .claude/validate.sh
```

`cargo nextest run` replaces `cargo test` — parallel execution, better output. Doctests run separately (`cargo test --doc`) because nextest doesn't support them.

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

**Include:** invariants, non-obvious design decisions, active constraints, traps that burned someone.

**Exclude:** pub types, function signatures, Cargo.toml deps (read the code), git history.

### Update Rules

- **After modifying a crate**: update `.claude/crates/{name}.md` before the session ends.
  - Changed invariants, decisions, or traps → update content.
  - Only implementation details changed → add `<!-- reviewed: YYYY-MM-DD -->` at bottom.
- **After global decisions change**: update `decisions.md` or `pitfalls.md`.
- **After work state changes**: update `active-work.md`.
- A Stop hook enforces this: completion is blocked if a crate was modified but its context file was not touched.
