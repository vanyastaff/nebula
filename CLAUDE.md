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
# Fast local check (use this by default)
cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace

# Single crate (fastest iteration)
cargo nextest run -p nebula-<crate>

# Full validation (before PR)
cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace && cargo test --workspace --doc && cargo deny check

# Compose API contract
cargo bench --no-run -p nebula-resilience

# Context file budgets
bash .claude/validate.sh
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

<!-- rtk-instructions v2 -->
# RTK (Rust Token Killer) - Token-Optimized Commands

## Golden Rule

**Always prefix commands with `rtk`**. If RTK has a dedicated filter, it uses it. If not, it passes through unchanged. This means RTK is always safe to use.

**Important**: Even in command chains with `&&`, use `rtk`:
```bash
# ❌ Wrong
git add . && git commit -m "msg" && git push

# ✅ Correct
rtk git add . && rtk git commit -m "msg" && rtk git push
```

## RTK Commands by Workflow

### Build & Compile (80-90% savings)
```bash
rtk cargo build         # Cargo build output
rtk cargo check         # Cargo check output
rtk cargo clippy        # Clippy warnings grouped by file (80%)
rtk tsc                 # TypeScript errors grouped by file/code (83%)
rtk lint                # ESLint/Biome violations grouped (84%)
rtk prettier --check    # Files needing format only (70%)
rtk next build          # Next.js build with route metrics (87%)
```

### Test (90-99% savings)
```bash
rtk cargo test          # Cargo test failures only (90%)
rtk vitest run          # Vitest failures only (99.5%)
rtk playwright test     # Playwright failures only (94%)
rtk test <cmd>          # Generic test wrapper - failures only
```

### Git (59-80% savings)
```bash
rtk git status          # Compact status
rtk git log             # Compact log (works with all git flags)
rtk git diff            # Compact diff (80%)
rtk git show            # Compact show (80%)
rtk git add             # Ultra-compact confirmations (59%)
rtk git commit          # Ultra-compact confirmations (59%)
rtk git push            # Ultra-compact confirmations
rtk git pull            # Ultra-compact confirmations
rtk git branch          # Compact branch list
rtk git fetch           # Compact fetch
rtk git stash           # Compact stash
rtk git worktree        # Compact worktree
```

Note: Git passthrough works for ALL subcommands, even those not explicitly listed.

### GitHub (26-87% savings)
```bash
rtk gh pr view <num>    # Compact PR view (87%)
rtk gh pr checks        # Compact PR checks (79%)
rtk gh run list         # Compact workflow runs (82%)
rtk gh issue list       # Compact issue list (80%)
rtk gh api              # Compact API responses (26%)
```

### JavaScript/TypeScript Tooling (70-90% savings)
```bash
rtk pnpm list           # Compact dependency tree (70%)
rtk pnpm outdated       # Compact outdated packages (80%)
rtk pnpm install        # Compact install output (90%)
rtk npm run <script>    # Compact npm script output
rtk npx <cmd>           # Compact npx command output
rtk prisma              # Prisma without ASCII art (88%)
```

### Files & Search (60-75% savings)
```bash
rtk ls <path>           # Tree format, compact (65%)
rtk read <file>         # Code reading with filtering (60%)
rtk grep <pattern>      # Search grouped by file (75%)
rtk find <pattern>      # Find grouped by directory (70%)
```

### Analysis & Debug (70-90% savings)
```bash
rtk err <cmd>           # Filter errors only from any command
rtk log <file>          # Deduplicated logs with counts
rtk json <file>         # JSON structure without values
rtk deps                # Dependency overview
rtk env                 # Environment variables compact
rtk summary <cmd>       # Smart summary of command output
rtk diff                # Ultra-compact diffs
```

### Infrastructure (85% savings)
```bash
rtk docker ps           # Compact container list
rtk docker images       # Compact image list
rtk docker logs <c>     # Deduplicated logs
rtk kubectl get         # Compact resource list
rtk kubectl logs        # Deduplicated pod logs
```

### Network (65-70% savings)
```bash
rtk curl <url>          # Compact HTTP responses (70%)
rtk wget <url>          # Compact download output (65%)
```

### Meta Commands
```bash
rtk gain                # View token savings statistics
rtk gain --history      # View command history with savings
rtk discover            # Analyze Claude Code sessions for missed RTK usage
rtk proxy <cmd>         # Run command without filtering (for debugging)
rtk init                # Add RTK instructions to CLAUDE.md
rtk init --global       # Add RTK to ~/.claude/CLAUDE.md
```

## Token Savings Overview

| Category | Commands | Typical Savings |
|----------|----------|-----------------|
| Tests | vitest, playwright, cargo test | 90-99% |
| Build | next, tsc, lint, prettier | 70-87% |
| Git | status, log, diff, add, commit | 59-80% |
| GitHub | gh pr, gh run, gh issue | 26-87% |
| Package Managers | pnpm, npm, npx | 70-90% |
| Files | ls, read, grep, find | 60-75% |
| Infrastructure | docker, kubectl | 85% |
| Network | curl, wget | 65-70% |

Overall average: **60-90% token reduction** on common development operations.
<!-- /rtk-instructions -->