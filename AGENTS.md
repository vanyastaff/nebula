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

| Task | Read |
|------|------|
| Fix a bug in a crate | `crates/<crate>/AGENTS.md`, `crates/<crate>/README.md`, relevant ADR |
| Add a new feature | `docs/INTEGRATION_MODEL.md` (how it connects) + `crates/<crate>/AGENTS.md` (roadmap/ADRs live in the maintainers' private design vault, not this repo) |
| Understand error handling | `crates/error/AGENTS.md` |
| Understand storage | `crates/storage/AGENTS.md` |
| Understand credentials | `crates/credential/AGENTS.md` |
| Add a cross-crate dep | `deny.toml` wrappers |
| Understand observability | `crates/metrics/AGENTS.md` |
| Create a PR | This file §Git Workflow |

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
| **obsidian** (memory) | Cross-session memory: `/recall <area>` before working a known area, `/remember` after a durable learning | One-shot tasks that don't need persistence |
| **grep** | Searching for string patterns, log messages | Finding symbol definitions (use Serena) |
| **read** | Reading a known file | Exploring unknown code structure (use Serena) |

**Rule of thumb:** If you're about to do 3+ grep/read calls to find something, use Serena instead.

**Cross-session memory (Obsidian vault):** design records (ADRs, roadmap, specs, research) and
accumulated agent learnings live in the maintainers' private Obsidian vault — **not tracked in this
repo** — reachable through the `obsidian` MCP. The vault root is `$OBSIDIAN_VAULT_PATH`; this
project's notes are under `projects/nebula/` (`agent/`, `codebase/`, `decisions/`, `knowledge/`,
`planning/`, `research/`, `specs/`, plus `MEMORY.md`). Run `/recall <area>` **before** working in a
known area so prior decisions and gotchas carry forward; `/remember` **after** a non-obvious
decision, gotcha, or fix. If the `obsidian` MCP is absent, memory recall is simply skipped.

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
| `cargo xtask ci-plan full` | Emit the versioned full CI package plan |
| `cargo xtask ci-plan diff --base <sha> --head <sha> --comparison merge-base` | Emit a metadata-driven diff plan |

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
├── clippy.toml         # lint thresholds (msrv 1.96)
├── crates/             # workspace members
├── tools/xtask/        # repository automation; outside the product layer graph
├── scripts/            # worktree.sh + lefthook helpers
├── .claude/            # Claude Code: guard hooks, slash commands
└── .github/            # CI workflows, CODEOWNERS, templates
```

Per-crate layout: `crates/<name>/` has `Cargo.toml`, `README.md`, `AGENTS.md`;
some carry a sibling derive crate (`<name>/macros`) and/or a `docs/` folder.

---

## Layered Dependency Map

**Mechanically enforced** by `cargo deny check` against `deny.toml` `[bans].deny` wrappers.
Each layer depends only on layers below. Direct imports of lower-layer domain types and ports are
normal; upward dependencies and undeclared lateral coupling are CI failures.

| Layer | Crates |
|-------|--------|
| **API / Surfaces** | `api`, `sdk` |
| **Exec** | `engine`, `orchestrator`, `worker`, `storage`, `storage-loom-probe` |
| **Business** | `resource`, `action`, `plugin`, `plugin-core`, `tenancy` |
| **Core / shared-infra** | `core`, `validator`, `expression`, `workflow`, `execution`, `schema`, `metadata`, `storage-port`, `credential` |
| **Cross-cutting** | `crypto`, `log`, `eventbus`, `metrics`, `resilience`, `error`, `env` |

`nebula-xtask` is repository tooling, not a product crate or architectural
layer. It may depend on general-purpose tooling libraries but never on a
`nebula-*` product package.

**Architecture Invariants** (rust-analyzer convention: each states what holds — or is
*deliberately absent* — everywhere; violating one is an architecture change, not a refactor):

- **Invariant:** durable commands and business facts cross crate boundaries through persisted state
  or explicit outbox/inbox ports. Direct downward dependencies on domain types and ports are normal.
  `nebula-eventbus` carries only ephemeral observations (telemetry, cache/UI invalidation, wake
  hints); consumers must tolerate loss, duplication, and reordering, and it is never a source of
  truth.
- **Invariant:** `nebula-storage-port` (Core) is the object-safe storage seam — it contains no backend code and never will.
- **Invariant:** `nebula-storage` (Exec) is the sole persistence-backend implementation. SQLite and
  Postgres are deployment backends; InMemory is an internal test/reference/conformance adapter,
  not a supported deployment backend. Policy decorators such as `nebula-tenancy` may wrap the
  port, but no other crate implements a persistence backend.
- **Invariant:** `nebula-credential` is shared infra importable from Exec, Business, and API tiers; secrets never appear in error messages (`SecretFreeMessage`) or `Debug` output.
- **Invariant:** durable write authority is aggregate-scoped. Runtime control owns the execution
  aggregate, execution journal and queues, execution outbox/inbox, and operation ledger;
  credential runtime owns credential/refresh/lease state; resource lifecycle owns
  resource/binding/fan-out state. Cross-aggregate commands and facts use durable persisted seams;
  `nebula-eventbus` may only wake or observe their owners.
- **Invariant:** every first-party deployment composition root in this workspace lives under
  `apps/`. `nebula-worker` (Exec) is reusable assembly that wires the engine into the
  `nebula-orchestrator` pull-loop (ADR-0095); `apps/worker` selects concrete adapters,
  configuration, and process lifecycle. A downstream host becomes a supported composition root
  only through the curated `nebula_sdk::embedded::RuntimeBuilder`; until that façade ships,
  downstream embedding is not a supported deployment surface. It cannot replace or bypass
  aggregate ownership, admission, or tenant authority.
- **Invariant:** plugins are statically linked, trusted in-process adapters (ADR-0091); WASM/process
  isolation is a non-goal (canon §12.6). `nebula-plugin-core` (Business) is the first-party `core`
  plugin built on `action`/`plugin`.
- **Invariant:** each `+macros` companion lives at the same layer as its parent and ships derives only — no runtime code.
- **Invariant:** CI package selection comes only from Cargo metadata through
  `cargo xtask ci-plan`; workflow and hook scripts consume its versioned JSON
  and do not maintain package-selection name lists or path-to-crate inference.
  The pre-push names `nebula-resilience`, `nebula-log`, `nebula-expression`,
  `nebula-credential`, `nebula-resource`, and `nebula-storage` form an
  independent no-default-feature gate-policy list applied only after selection;
  they never decide matrix membership.
- **API boundaries:** `sdk` is the sole supported and branded Rust surface, organized by persona:
  workflow/authoring, integration, schema, testing, client, and embedded façades. The curated
  client submits versioned transport requests; the curated embedded façade submits typed runtime
  commands. Neither exposes raw stores, mutation/admission capabilities, claim tokens, or tenant
  proofs. The HTTP API contract and all implementation crates remain technical boundaries, not
  separately supported Rust products. Required internal packages may be published as exact-version,
  lockstep dependencies of `nebula-sdk`, but direct use is unsupported (private ADR-0117).

Per-crate invariants live in each `crates/<crate>/AGENTS.md` (convention: prefer a
dedicated `## Invariants` section; state what the crate deliberately does NOT do).

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
- **Use the right cross-crate seam:** direct downward imports for domain types and ports; persisted
  state or explicit outbox/inbox ports for durable commands and facts; `nebula-eventbus` only for
  lossy observations and wake hints.
- **Ship observability with every new state/error/hot path** — typed error variant + tracing span + invariant check.
- **Use Serena's symbolic tools** (find_symbol, rename_symbol, replace_symbol_body) instead of grep/read for code navigation.
- **Run `cargo check -p nebula-<name>`** after editing a crate for fast feedback.
- **Read `crates/<crate>/AGENTS.md`** before working on a crate — it has crate-specific rules.
- **Suppress lints with `#[expect(lint, reason = "...")]`**, never bare `#[allow]` — the `allow_attributes` lint enforces this. If the lint fires only in some feature/cfg config, gate the expectation: `#[cfg_attr(not(feature = "x"), expect(...))]`.
- **Verify the feature matrix for crates with features** (`log`, `credential`, `resource`, `storage`): run clippy on default AND `--no-default-features`, not just `--all-features` — `cfg_attr(not(feature = ...))` code is invisible to an all-features pass.
- **Pin lockfile changes**: `cargo update -p <crate> --precise <ver>` — never a wholesale `cargo update`.

## Rules — DON'T

- **Don't `unwrap()`/`expect()`/`panic!()` in library code.** Tests, `const`, and binaries are exempt.
- **Don't add TODO/FIXME/HACK in committed code.** The `edit-guard.sh` hook blocks it.
- **Don't weaken tests while changing implementation** in the same turn.
- **Don't use bare `#[allow(...)]`** — use `#[expect(..., reason = "...")]`. The only sanctioned `#[allow]` is inside exported `macro_rules!` bodies (expansions land in downstream crates where an expectation can't be fulfilled): there, use the self-suppressing form `#[allow(<lint>, clippy::allow_attributes, reason = "...")]` plus `// guard-justified: <reason>` for the edit hook.
- **Don't use `git commit --no-verify`** or `git push --force` without explicit user confirmation.
- **Don't add dependencies that cross layer boundaries** without checking `deny.toml` wrappers first.
- **Don't put runnable examples in per-crate dirs** — they go in the root `examples/` workspace member.
- **Don't read `target/`, `.worktrees/`, or `.claude/worktrees/`** — they're denied in settings.

---

## Error Triage

When you hit a build/test error:

1. **Layer violation (cargo-deny)** → check `deny.toml` `[bans].deny` wrappers. The crate you're
   importing from is in a higher layer or is an undeclared lateral dependency. Depend on a
   lower-layer domain/port crate, move the shared contract down, or communicate a durable command
   through its owning persisted port. Do not bypass the layer map with `nebula-eventbus`.
2. **`unwrap()` in lib code** → replace with `?` operator + typed `thiserror` variant.
3. **Missing trait bound** → check if the type needs `Send + Sync` (all async paths require it).
4. **Clippy warning** → run `task clippy` to see workspace-wide. Fix the warning, don't suppress it.
5. **Test failure after refactor** → check if you weakened a test assertion. The `edit-guard.sh` hook blocks this.
6. **`convco` commit rejection** → your commit message doesn't follow Conventional Commits. Format: `type(scope): summary`.
7. **`unfulfilled_lint_expectations` warning** → the `#[expect]`ed lint no longer fires. Either the suppression is stale (delete it) or it's config-dependent (gate with `cfg_attr` to the configs where it fires; remember lib and lib-test compile the same source twice — `cfg_attr(not(test), ...)` for test-only-used items).
8. **`clippy::allow_attributes` warning** → convert `#[allow]` to `#[expect(..., reason)]`; see Rules — DON'T for the macro_rules exception.

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

## Slash Commands

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
| Pitfalls | `docs/pitfalls.md` | Before touching hot paths |
| Engineering playbook 2026 | `docs/ENGINEERING_2026.md` | Adopted practices from iroh/reth/omicron/rust-analyzer/uv + adoption roadmap |
| Design records (ADRs, roadmap, specs, research) | maintainers' private Obsidian vault (`obsidian` MCP → `projects/nebula/`) | Not tracked here — reach via `/recall` |

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
| `tools/xtask/` | Metadata-driven repository automation and its contract tests |
| `.github/workflows/ci.yml` | CI required jobs |
