# AGENTS.md

> Map of the Nebula workspace for AI agents and new contributors. Keep factual.
> Update when crates / top-level layout / required files change. Detailed
> product / architecture content lives in `README.md` — do not duplicate it
> here.

## Project Overview

Nebula is a modular, type-safe **workflow automation engine** in Rust. See
`README.md` for the product overview and `.ai-factory/DESCRIPTION.md` for the
agent-facing summary.

## Tech Stack

- **Language:** Rust 1.95+ (edition 2024, resolver 3)
- **Async:** Tokio
- **Errors:** `thiserror` (libs) / `anyhow` (bins)
- **Storage backends:** PostgreSQL, SQLite (`crates/storage/migrations/`)
- **Testing:** `cargo nextest` + doctests
- **Build orchestration:** `Taskfile.yml` (`task --list`)
- **Local hooks:** `lefthook.yml` (mirrors CI required jobs)

## Common Commands

Run via `task <name>`. See `task --list` for the full catalog.

**Workspace-wide:**

| Command                  | Purpose |
|--------------------------|---------|
| `task dev:check`         | Pre-PR gate: fmt + clippy + nextest + doctests + deny |
| `task check`             | Type-check all crates and targets (no codegen) |
| `task build`             | Debug build (`task build:release` for release) |
| `task fmt`               | Format (`cargo fmt --all` on the pinned stable toolchain) |
| `task clippy`            | Workspace clippy with `-D warnings` |
| `task quality`           | Quick mechanical gate (fmt:check + clippy) |
| `task deny`              | `cargo-deny`: layer wrappers + advisories + licenses |
| `task test`              | All workspace tests (`cargo test`; nextest is in `dev:check`) |
| `task doc` / `doc:open`  | Build (and open) workspace docs |
| `task ci`                | Full CI pipeline locally |

**Single crate:**

| Command                                | Purpose |
|----------------------------------------|---------|
| `cargo check -p <crate>`               | Fastest feedback for one crate |
| `cargo build -p <crate>`               | Build one crate |
| `task test:crate CRATE=<name>`         | Tests for one crate via `cargo test -p` (e.g. `CRATE=nebula-action`) |
| `task test:crate:features CRATE=<n>`   | Same, with `--all-features` |
| `cargo nextest run -p <crate>`         | Per-crate run with the workspace's nextest config |
| `cargo nextest run -p <crate> <test>`  | Single test by name |
| `cargo test -p <crate> --doc`          | Doctests for one crate |
| `cargo doc -p <crate> --open`          | Build/open one crate's rustdoc |
| `cargo tree -p <crate>`                | Inspect that crate's dep tree |
| `task bench:crate CRATE=<name>`        | Benchmarks for one crate |

**Infra:**

| Command                         | Purpose |
|---------------------------------|---------|
| `task db:up && task db:migrate` | Local Postgres + sqlx migrations |
| `task db:reset`                 | Drop + recreate DB (prompts) |
| `task obs:up` / `obs:down`      | Jaeger + OTEL collector |
| `task infra:up` / `infra:down`  | Self-hosted stack (Postgres + Redis + API) |

## Workspace Layout

Top-level structure (run `ls` for the full listing; crate inventory is in
the Layered Dependency Map below):

```
nebula/
├── Cargo.toml          # workspace members + pinned deps + [workspace.lints]
├── Taskfile.yml        # task runner — see Common Commands above
├── deny.toml           # cargo-deny: layer wrappers (CI gate)
├── lefthook.yml        # local pre-commit / pre-push (mirrors CI)
├── rustfmt.toml        # rustfmt config (stable-only)
├── clippy.toml         # lint thresholds (msrv 1.95)
├── crates/             # 33 workspace members (incl. 8 derive companions)
├── scripts/            # worktree.sh + lefthook helpers
├── .ai-factory/        # agent context (DESCRIPTION, ARCHITECTURE, rules/, plans/)
├── .claude/            # Claude Code skills + subagents
├── .cursor/rules/      # Cursor rules (point back to AGENTS.md)
└── .github/            # CI workflows, CODEOWNERS, PR/issue templates
```

Per-crate layout: each `crates/<name>/` has `Cargo.toml` and `README.md`;
some carry a sibling derive crate (`<name>/macros`) and/or a `docs/` folder.

## Layered Dependency Map

Mechanically enforced by `cargo deny check` against `deny.toml` `[wrappers]`.
Each layer depends only on layers below; cross-cutting crates are importable at
any level.

| Layer        | Crates |
|--------------|--------|
| API / Public | `api`, `sdk` |
| Exec         | `engine`, `storage`, `storage-loom-probe`, `sandbox`, `plugin-sdk` |
| Business     | `credential`, `credential-builtin`, `resource`, `action`, `plugin` |
| Core         | `core`, `validator`, `expression`, `workflow`, `execution`, `schema`, `metadata` |
| Cross-cutting| `log`, `system`, `eventbus`, `metrics`, `resilience`, `error` |

Each `+macros` companion (`action/macros`, `credential/macros`, `error/macros`,
`plugin/macros`, `resource/macros`, `schema/macros`, `validator/macros`,
`sdk/macros-support`) lives at the same layer as its parent and ships derives
only — omitted from the table for brevity.

Cross-crate communication goes through `nebula-eventbus`, **not** direct imports
between siblings at the same layer.

## Key Entry Points

| File                          | Purpose |
|-------------------------------|---------|
| `Cargo.toml`                  | Workspace members, pinned deps, `[workspace.lints]` |
| `deny.toml`                   | Layer wrappers, licenses, advisories — CI gate |
| `clippy.toml`                 | Lint thresholds (msrv 1.95) |
| `rustfmt.toml`                | rustfmt config (stable-only, runs on pinned toolchain) |
| `rust-toolchain.toml`         | Pinned toolchain |
| `lefthook.yml`                | Local pre-commit / pre-push (mirrors CI) |
| `Taskfile.yml`                | `task dev:check` = full pre-PR gate; `task --list` for catalog |
| `scripts/worktree.sh`         | Canonical helper behind `task wt:*` and `task git:commit` |
| `.github/workflows/ci.yml`    | CI required jobs: fmt, clippy, nextest, doctests, MSRV, deny |
| `.github/CODEOWNERS`          | Auto-reviewer + security-sensitive path gates |
| `crates/<crate>/README.md`    | Per-crate human entry point |
| `crates/<crate>/Cargo.toml`   | Per-crate features, deps, lints |

## Documentation Index

| Document               | Path                          | Description |
|------------------------|-------------------------------|-------------|
| Product overview       | `README.md`                   | What Nebula is, design principles, architecture |
| Contribution guide     | `CONTRIBUTING.md`             | Quick start, workflow, branch / commit / PR rules |
| Security policy        | `.github/SECURITY.md`         | Reporting vulnerabilities |
| Per-crate READMEs      | `crates/<crate>/README.md`    | Crate-level usage and design notes |
| Per-crate design docs  | `crates/<crate>/docs/`        | Where present |
| GitHub project setup   | `.github/PROJECT_SETUP.md`    | Repo / project board configuration |

## AI Context Files

| File                          | Purpose |
|-------------------------------|---------|
| `AGENTS.md`                   | This file — project map for any AI agent |
| `CLAUDE.md`                   | Claude Code shim that imports `AGENTS.md` |
| `.ai-factory/config.yaml`     | AI Factory settings (language, paths, git, rules) |
| `.ai-factory/DESCRIPTION.md`  | Agent-facing project summary |
| `.ai-factory/ARCHITECTURE.md` | Agent-actionable architecture subset |
| `.ai-factory/rules/base.md`   | Distilled coding rules for agents |
| `.ai-factory.json`            | AI Factory install manifest (managed by tooling) |
| `.github/copilot-instructions.md` | GitHub Copilot guidance |
| `.cursor/rules/*.mdc`         | Cursor project rules that point back to `AGENTS.md` |
| `.claude/skills/`             | Claude Code `/aif-*` skill definitions |
| `.claude/agents/`             | Subagent definitions (sidecars, workers, loop roles) |

## Agent Git Workflow

All persistent task branches go through `scripts/worktree.sh` (or the
`task wt:*` wrappers — set `WT_NAME` / `WT_TYPE` / `WT_SCOPE`). Branch from
`origin/main`; squash-merge back; never force-push shared history.

| Step       | Command |
|------------|---------|
| New branch | `bash scripts/worktree.sh new <slug> <type> <scope>` — creates `.worktrees/<slug>` and branch `<type>/<scope>-<slug>` |
| List       | `bash scripts/worktree.sh list` |
| Commit     | `bash scripts/worktree.sh commit <type> <scope> <summary>` — emits `<type>(<scope>): <summary>`, validated by `convco` |
| Remove     | `bash scripts/worktree.sh remove <slug>` |
| Finish     | `bash scripts/worktree.sh finish <slug>` — sync `main`, drop worktree, delete merged branch |

**Allowed types:** `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`,
`refactor`, `revert`, `style`, `test`.

**Scope:** crate name without `nebula-` prefix (`resilience`, `engine`, `api`)
or top-level area (`docs`, `ci`, `scripts`, `github`).

Managed cloud/IDE worktrees that can't live under `.worktrees/` must still
follow these branch/commit rules — CI and lefthook are the gate.

## Agent Rules

- **Decompose chained shell commands.** Run them as separate steps so each step
  has a clear pass/fail. Do not chain unrelated git operations.
  - Wrong: `git checkout main && git pull`
  - Right: first `git checkout main`, then `git pull origin main`
- **Branch from `main`, squash-merge to `main`.** Never force-push or rewrite
  shared history without explicit confirmation.
- **Conventional Commits, validated by `convco`.** Scope = crate name without
  `nebula-` prefix, or top-level area (`docs`, `ci`).
- **No `unwrap()` / `expect()` / `panic!()` in library code.** Use typed
  `thiserror` errors. Tests, `const`, and binaries are exempt per `clippy.toml`.
- **Cross-crate communication goes through `nebula-eventbus`** — never reach
  across layer boundaries with direct imports.
- **Observability is part of Definition of Done.** New state / error / hot path
  must ship with a typed error variant + tracing span + invariant check.
- **`lefthook pre-push` mirrors CI required jobs.** Keep them in sync; if you
  change one, update the other.
- **Runnable examples** live in a root-level `examples/` workspace member, not
  per-crate `examples/`.
