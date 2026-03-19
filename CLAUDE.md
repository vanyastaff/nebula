# CLAUDE.md — Working Memory for AI Sessions

Last updated: **2026-03-18**

---

## Project Overview

**Nebula** is a modular, type-safe workflow automation engine (like n8n/Zapier) built in Rust 1.93+. Workflows are DAGs of composable actions with built-in retries/error handling, extensible via plugins. Alpha: core crates stable, execution engine + credential system in active development. 26-crate workspace with strict one-way layer dependencies.

---

## Active Constraints

- **No upward dependencies.** Use slim DTOs if needed; enforce via `deny.toml`.
- **`nebula-core` stays small.** It's imported everywhere.
- **Credentials always encrypted at rest.** `CredentialAccessor` injected via `Context`; no global lookups.
- **EventBus for all cross-crate signals.** Never add direct imports to "fix" missing signals.
- **Actions use DI via `Context`.** Never construct runtime-managed types inside actions.
- **Parameter provider API is error-based.** `register_*()` returns `Result`; no panics.
- **`InProcessSandbox` only in Phase 2.** No OS-process/WASM isolation until Phase 3.

---

## Do Not Touch

- **`nebula-core`** without approval — cascades everywhere. Adding new ID types is safe; changing traits is not.
- **`crates/resilience/benches/compose.rs`** — it's a contract documenting the compose API.
- **`docs/crates/parameter/*.md`** — stale but kept for migration history; scheduled for update, not deletion.

---

## Key Commands

```bash
cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace
cargo bench --no-run -p nebula-resilience   # verify compose API compiles
bash .claude/validate.sh                    # check context file token budgets
```

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
