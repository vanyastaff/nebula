# AGENTS.md

> **Canonical agent-rules & project map for Nebula.**
> This file is the single source of truth for AI agents working in this repo.
> `CLAUDE.md` is a thin pointer back to this file.
> Detailed product/architecture content lives in `README.md` — do not duplicate it here.

---

## Quick Start for AI Agents

**Read this first.** Then read `crates/<crate>/AGENTS.md` for the crate you're working on.

### Decision Tree

```
You need to...
├── Understand the project → read this file + README.md
├── Work on a specific crate → read crates/<crate>/AGENTS.md + README.md
├── Find a symbol/function → use Serena (find_symbol, symbol_overview)
├── Find where something is called → use Serena (find_references)
├── Rename across files → use Serena (rename_symbol) — NOT grep+replace
├── Understand an error type → read crates/error/AGENTS.md + docs/PRODUCT_CANON.md
├── Add a dependency → check layer rules in "Layered Dependency Map" below
├── Run tests for one crate → `cargo nextest run -p nebula-<name>`
├── Run full pre-PR gate → `task dev:check`
├── Create a branch → `bash scripts/worktree.sh new <slug> feat <crate>`
├── Make a commit → `bash scripts/worktree.sh commit feat <crate> "summary"`
└── Check if code compiles → `cargo check -p nebula-<name>`
```

### What to Read by Task

| Task | Read First | Then |
|------|-----------|------|
| Fix a bug in a crate | `crates/<crate>/AGENTS.md` | `crates/<crate>/README.md`, relevant ADR |
| Add a new feature | `docs/ROADMAP.md` (is it planned?) | `crates/<crate>/AGENTS.md`, `docs/INTEGRATION_MODEL.md` |
| Understand error handling | `.agents/skills/nebula-error-and-validation/SKILL.md` | `crates/error/AGENTS.md` |
| Understand storage | `.agents/skills/nebula-storage-port-adapter/SKILL.md` | `crates/storage/AGENTS.md` |
| Understand credentials | `.agents/skills/nebula-credential-lifecycle/SKILL.md` | `crates/credential/AGENTS.md` |
| Add a cross-crate dep | `.agents/skills/nebula-layer-boundaries/SKILL.md` | `deny.toml` wrappers |
| Understand observability | `.agents/skills/nebula-observability-dod/SKILL.md` | `crates/metrics/AGENTS.md` |
| Create a PR | `.agents/skills/nebula-worktree-pr-workflow/SKILL.md` | This file §Git Workflow |

---

## MCP Servers — When to Use What

| Tool | Use When | Don't Use When |
|------|----------|----------------|
| **Serena find_symbol** | Looking for a struct/fn/trait definition | You already know the exact file:line |
| **Serena find_references** | Finding all callers of a function | You need to search for a string literal (use grep) |
| **Serena rename_symbol** | Renaming across the codebase | Renaming a local variable in one function (use edit) |
| **Serena symbol_overview** | Getting file structure/outline | You need to read the full file (use read) |
| **Serena replace_symbol_body** | Replacing a function/struct body | Editing a few lines inside a function (use edit) |
| **rust-analyzer-mcp** | Hover info, diagnostics, code actions, completion | Symbol search (use Serena) |
| **rust-mcp-server** | cargo check/clippy/deny/machete/hack/fmt/test | Symbol-level code navigation (use Serena) |
| **rust-docs** | Crate documentation, source code, dependency trees | Local crate code (use Serena) |
| **cratesio** | Searching crates.io for packages | Local workspace queries |
| **Memory MCP** | Storing cross-session knowledge | One-shot tasks that don't need persistence |
| **grep** | Searching for string patterns, log messages | Finding symbol definitions (use Serena) |
| **read** | Reading a known file | Exploring unknown code structure (use Serena) |

**Rule of thumb:** If you're about to do 3+ grep/read calls to find something, use Serena instead.

---

## Preferred CLI Tools

Use these instead of standard Unix equivalents — they're installed and better.

| Task | Use | Instead of |
|------|-----|-----------|
| Search code/text | `rg` (ripgrep) | `grep` |
| Find files | `fd` | `find` |
| View file with highlighting | `bat` | `cat` |
| List directory | `eza --icons` | `ls` |
| Disk usage | `dust` | `du` |
| Process list | `procs` | `ps` |
| Find & replace in files | `sd` | `sed` |
| JSON query | `jq` | — |
| YAML/TOML query | `yq` | — |
| Git diff viewer | `delta` | `diff` |
| Markdown preview | `glow` | — |
| Quick docs lookup | `tldr` | `man` |
| Sort Cargo.toml deps | `cargo-sort` | manual |
| Smart directory jump | `zoxide` | `cd` |

**Install new cargo tools with `cargo binstall`** (pre-built binaries) instead of `cargo install` (compiles from source).

---

## Tech Stack

- **Language:** Rust 1.96+ (edition 2024, resolver 3)
- **Async:** Tokio
- **Errors:** `thiserror` (libs) / `anyhow` (bins)
- **Storage:** PostgreSQL, SQLite (`crates/storage/migrations/`)
- **Testing:** `cargo nextest` + doctests
- **Build:** `Taskfile.yml` (`task --list`)
- **Hooks:** `lefthook.yml` (mirrors CI required jobs)

---

## Common Commands

Run via `task <name>`. See `task --list` for the full catalog.

### Workspace-wide

| Command | Purpose |
|---------|---------|
| `task dev:check` | **Pre-PR gate:** fmt + clippy + nextest + doctests + deny |
| `task check` | Type-check all crates (no codegen) |
| `task build` | Debug build (`task build:release` for release) |
| `task fmt` | Format (`cargo fmt --all` on pinned stable toolchain) |
| `task clippy` | Workspace clippy with `-D warnings` |
| `task quality` | Quick gate: fmt:check + clippy |
| `task deny` | `cargo-deny`: layer wrappers + advisories + licenses |
| `task test` | All workspace tests |
| `task ci` | Full CI pipeline locally |

### Single Crate

| Command | Purpose |
|---------|---------|
| `cargo check -p nebula-<name>` | **Fastest feedback** for one crate |
| `cargo nextest run -p nebula-<name>` | Tests for one crate |
| `cargo nextest run -p nebula-<name> <test>` | Single test by name |
| `cargo test -p nebula-<name> --doc` | Doctests for one crate |
| `cargo doc -p nebula-<name> --open` | Build/open crate docs |
| `cargo tree -p nebula-<name>` | Inspect dependency tree |
| `task bench:crate CRATE=<name>` | Benchmarks |

### Infra

| Command | Purpose |
|---------|---------|
| `task db:up && task db:migrate` | Local Postgres + sqlx migrations |
| `task db:reset` | Drop + recreate DB (prompts) |
| `task obs:up` / `obs:down` | Jaeger + OTEL collector |

---

## Workspace Layout

```text
nebula/
├── Cargo.toml          # workspace members + pinned deps + [workspace.lints]
├── Taskfile.yml        # task runner
├── deny.toml           # cargo-deny: layer wrappers (CI gate)
├── lefthook.yml        # local pre-commit / pre-push (mirrors CI)
├── rustfmt.toml        # rustfmt config (stable-only)
├── clippy.toml         # lint thresholds (msrv 1.95)
├── crates/             # workspace members
├── scripts/            # worktree.sh + lefthook helpers
├── .agents/skills/     # Agent skills
├── .claude/            # Claude Code: guard hooks, slash commands
├── .cursor/rules/      # Cursor rules (defer to AGENTS.md)
└── .github/            # CI workflows, CODEOWNERS, templates
```

Per-crate layout: `crates/<name>/` has `Cargo.toml`, `README.md`, `AGENTS.md`;
some carry a sibling derive crate (`<name>/macros`) and/or a `docs/` folder.

---

## Layered Dependency Map

**Mechanically enforced** by `cargo deny check` against `deny.toml` `[bans].deny` wrappers.
Each layer depends only on layers below. Upward dependency = CI failure.

| Layer | Crates |
|-------|--------|
| **API / Public** | `api`, `sdk` |
| **Exec** | `engine`, `storage`, `storage-loom-probe` |
| **Business** | `resource`, `action`, `plugin`, `tenancy` |
| **Core / shared-infra** | `core`, `validator`, `expression`, `workflow`, `execution`, `schema`, `metadata`, `storage-port`, `credential` |
| **Cross-cutting** | `crypto`, `log`, `eventbus`, `metrics`, `resilience`, `error`, `env` |

**Key architectural facts:**
- Cross-crate communication goes through `nebula-eventbus`, **not** direct imports between siblings.
- `nebula-storage-port` (Core) is the object-safe storage seam — no backend code.
- `nebula-storage` (Exec) is the sole adapter implementation (InMemory + SQLite + Postgres).
- `nebula-credential` is shared infra importable from Exec, Business, and API tiers.
- Plugins register in-process (ADR-0091). WASM/process isolation is a non-goal (canon §12.6).
- Each `+macros` companion lives at the same layer as its parent and ships derives only.

---

## Agent Git Workflow

All persistent branches go through `scripts/worktree.sh` (or `task wt:*` wrappers).

| Step | Command |
|------|---------|
| New branch | `bash scripts/worktree.sh new <slug> <type> <scope>` |
| List | `bash scripts/worktree.sh list` |
| Commit | `bash scripts/worktree.sh commit <type> <scope> <summary>` |
| Finish | `bash scripts/worktree.sh finish <slug>` |

**Allowed types:** `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`, `revert`, `style`, `test`.
**Scope:** crate name without `nebula-` prefix (`resilience`, `engine`, `api`) or top-level area (`docs`, `ci`).

---

## Rules — DO

- **Decompose chained shell commands.** Run each step separately for clear pass/fail.
- **Branch from `main`, squash-merge to `main`.** Never force-push shared history.
- **Use Conventional Commits**, validated by `convco`. Scope = crate name without `nebula-` prefix.
- **Use `thiserror` in libs, `anyhow` in bins.** No `unwrap()`/`expect()`/`panic!()` in library code.
- **Route cross-crate calls through `nebula-eventbus`** — never direct sibling imports.
- **Ship observability with every new state/error/hot path** — typed error variant + tracing span + invariant check.
- **Use Serena's symbolic tools** (find_symbol, rename_symbol, replace_symbol_body) instead of grep/read for code navigation.
- **Run `cargo check -p nebula-<name>`** after editing a crate for fast feedback.
- **Read `crates/<crate>/AGENTS.md`** before working on a crate — it has crate-specific rules.

## Rules — DON'T

- **Don't `unwrap()`/`expect()`/`panic!()` in library code.** Tests, `const`, and binaries are exempt.
- **Don't add TODO/FIXME/HACK in committed code.** The `edit-guard.sh` hook blocks it.
- **Don't weaken tests while changing implementation** in the same turn.
- **Don't add `#[allow(clippy::...)]` without `// guard-justified: <reason>`** on the line above.
- **Don't use `git commit --no-verify`** or `git push --force` without explicit user confirmation.
- **Don't add dependencies that cross layer boundaries** without checking `deny.toml` wrappers first.
- **Don't put runnable examples in per-crate dirs** — they go in the root `examples/` workspace member.
- **Don't read `target/`, `.worktrees/`, or `.claude/worktrees/`** — they're denied in settings.

---

## Error Triage

When you hit a build/test error:

1. **Layer violation (cargo-deny)** → check `deny.toml` `[bans].deny` wrappers. The crate you're importing from is in a higher layer. Use `nebula-eventbus` for cross-crate communication, or move the code down a layer.
2. **`unwrap()` in lib code** → replace with `?` operator + typed `thiserror` variant. See `.agents/skills/nebula-error-and-validation/SKILL.md`.
3. **Missing trait bound** → check if the type needs `Send + Sync` (all async paths require it).
4. **Clippy warning** → run `task clippy` to see workspace-wide. Fix the warning, don't suppress it.
5. **Test failure after refactor** → check if you weakened a test assertion. The `edit-guard.sh` hook blocks this.
6. **`convco` commit rejection** → your commit message doesn't follow Conventional Commits. Format: `type(scope): summary`.

---

## Enforced Discipline

Rules enforced by **lefthook** (pre-commit + pre-push) and **CI**. Not by Claude Code hooks.

**Pre-commit** (on `git commit`):
- `fmt-check` — per-crate rustfmt
- `clippy` — per-crate clippy
- `typos` — typo detection
- `taplo` — TOML formatting
- `cargo-deny` — layer wrappers + advisories

**Pre-push** (on `git push`):
- `clippy-full` — workspace clippy `-D warnings` (skips if no `.rs` in push range)
- `crate-diff-gate` — nextest for changed crates

**Commit message**: `convco` validates Conventional Commits format.

**Rules to follow manually** (no hook blocks you — but CI will fail):
- No `unwrap()`/`expect()`/`panic!()` in library code
- No TODO/FIXME/HACK in committed code
- Don't weaken tests while changing implementation
- Use `// guard-justified: <reason>` if you need `#[allow]` or `todo!` temporarily

---

## Skills

Skills live in `.agents/skills/`, discoverable on demand:

| Skill | When to Load |
|-------|-------------|
| `nebula-layer-boundaries` | Adding a cross-crate dependency |
| `nebula-credential-lifecycle` | Working with credentials, OAuth, secret rotation |
| `nebula-storage-port-adapter` | Changing storage, repository traits, CAS/leases |
| `nebula-error-and-validation` | Adding error types, handling validation |
| `nebula-observability-dod` | Adding metrics, tracing, logging |
| `nebula-worktree-pr-workflow` | Creating branches, commits, PRs |

Slash commands: `.claude/commands/` (project-specific, load on demand).

---

## Documentation Index

| Document | Path | When to Read |
|----------|------|-------------|
| **Doc map** | `docs/README.md` | **Start here for docs** — Tier 0–1 only |
| Agent rules | `AGENTS.md` | This file — always relevant |
| Per-crate map | `crates/<crate>/AGENTS.md` | Before working on a crate |
| Product overview | `README.md` | Understanding what Nebula is |
| Product canon | `docs/PRODUCT_CANON.md` | Binding invariants (durability, credentials) |
| Integration model | `docs/INTEGRATION_MODEL.md` | How crates connect (Resource, Credential, Action, Schema, Plugin) |
| 1.0 roadmap | `docs/ROADMAP.md` | Checking if a feature is planned |
| Pitfalls | `docs/pitfalls.md` | Before touching hot paths |
| ADRs | `docs/adr/` | Understanding architectural decisions |
| Onboarding | `HANDOFF.md` | New collaborator orientation |

---

## Key Entry Points

| File | Purpose |
|------|---------|
| `Cargo.toml` | Workspace members, pinned deps, `[workspace.lints]` |
| `deny.toml` | Layer wrappers, licenses, advisories — CI gate |
| `clippy.toml` | Lint thresholds (msrv 1.96) |
| `rustfmt.toml` | rustfmt config (stable-only, pinned toolchain) |
| `Taskfile.yml` | `task dev:check` = full pre-PR gate |
| `.mcp.json` | MCP server config (Serena, rust-analyzer, cratesio, etc.) |
| `scripts/worktree.sh` | Branch lifecycle helper |
| `.github/workflows/ci.yml` | CI required jobs |

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, invoke the `skill` tool with `skill: "graphify"` before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).

### Research artifacts → `raw/`

When an agent gathers external knowledge — articles, studies, research findings, deepwiki/web dumps, or a standalone analysis it produced — **save it as a file under `raw/` in the project root**. The owner ingests `raw/` into graphify manually (`graphify .`) on their own schedule.

- `raw/` is git-ignored (local-only, point-in-time, not durable docs). Do **not** commit it.
- One artifact per file. Markdown preferred (graphify ingests it cleanly); keep the original format if conversion would lose signal.
- Name by source so provenance is obvious: `deepwiki_<repo>/…`, `github_com_<owner>_<repo>.md`, `web_<domain>_<slug>.md`, or `analysis_<topic>.md`. Multi-page sources get a subfolder.
- Lead each file with a short header: source URL/identifier, what it is, date collected.
- This is for *gathered/derived* knowledge, not durable project docs — real docs still go to `docs/`.
