# CLAUDE.md

> **Canonical agent-rules & project map for Nebula.** This file is the single
> source of truth for AI agents and new contributors in this repo: layout,
> architecture, commands, Git workflow, coding rules, and enforced discipline.
> `AGENTS.md` is an intentionally thin cross-tool pointer back to this file;
> `.cursor/rules/*` and `.github/copilot-instructions.md` defer here. Keep
> factual; update when crates / top-level layout / required files change.
> Detailed product / architecture content lives in `README.md` — do not
> duplicate it here.

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

```text
nebula/
├── Cargo.toml          # workspace members + pinned deps + [workspace.lints]
├── Taskfile.yml        # task runner — see Common Commands above
├── deny.toml           # cargo-deny: layer wrappers (CI gate)
├── lefthook.yml        # local pre-commit / pre-push (mirrors CI)
├── rustfmt.toml        # rustfmt config (stable-only)
├── clippy.toml         # lint thresholds (msrv 1.95)
├── crates/             # 36 workspace members (incl. 8 derive companions)
├── scripts/            # worktree.sh + lefthook helpers
├── .ai-factory/        # agent context (DESCRIPTION, ARCHITECTURE, rules/, plans/)
├── .claude/            # Claude Code: canonical CLAUDE.md, guard hooks, skills, subagents
├── .cursor/rules/      # Cursor rules (defer to CLAUDE.md)
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
| Exec         | `engine`, `storage`, `storage-loom-probe` |
| Business     | `credential-builtin`, `resource`, `action`, `plugin`, `tenancy` |
| Plugin-Proto | `plugin-sdk`, `sandbox` |
| Core         | `core`, `validator`, `expression`, `workflow`, `execution`, `schema`, `metadata`, `storage-port` |
| Cross-cutting| `log`, `eventbus`, `metrics`, `resilience`, `error` |

**Plugin-Proto** is a leaf tier between Core and Business: the
out-of-process plugin protocol (`plugin-sdk`) and the duplex transport
(`sandbox`). It depends only on Core (+ tokio/serde); Business (`plugin`)
and Exec (`engine`) depend on it *downward*. The discovery path and the
`SandboxError` → `ActionError` seam live in `plugin`; the `SandboxRunner`
runner abstraction lives in `engine`.

`nebula-credential` is **shared infra**, not a single-tier Business crate:
the Exec tier (`engine`, `storage`) and the API tier consume the
credential contract directly alongside Business (`action`, `plugin`,
`resource`) and the first-party backends (`credential-builtin`,
`credential-vault`). Like the cross-cutting crates it is importable from
those tiers; the `deny.toml` `[wrappers]` allowlist locks the exact
consumer set.

`nebula-storage-port` (Core) is the object-safe storage seam: the spec-16
row-model contract (execution / workflow-row+version / control-queue /
node-result / journal ports) every storage consumer depends on, with no
backend code. `nebula-storage` (Exec) is the sole adapter implementation
(InMemory + SQLite + Postgres); `nebula-tenancy` (Business) is the
scope-enforcing decorator that wraps a raw adapter so a tenant scope is
substituted on every call before it reaches a handler. `engine`, `api`,
the knife harness, and `storage-loom-probe` run on the port — the legacy
`ExecutionRepo` / `WorkflowRepo` surface and the never-implemented spec-16
placeholders were deleted (ADR-0072).

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
| **Agent doc map**      | `docs/README.md`              | **Start here for docs** — Tier 0–1 only; do not bulk-read `docs/adr/0*` |
| Removed doc trees      | `docs/ARCHIVE.md`             | superpowers / audits / brainstorms moved out of repo |
| Agent rules + map      | `CLAUDE.md`                   | **Canonical** — this file — branch / commit / PR rules + project map + enforced discipline |
| Product strategy       | `STRATEGY.md`                 | Direction, 2026 standard bar, flagship, tracks (complements canon) |
| 1.0 roadmap            | `docs/ROADMAP.md`             | Production-ready 1.0 milestone checklist (M0–M14), capability/dependency-ordered |
| Product overview       | `README.md`                   | What Nebula is, design principles, architecture |
| Cross-tool pointer     | `AGENTS.md`                   | Thin pointer naming `CLAUDE.md` canonical |
| Product canon          | `docs/PRODUCT_CANON.md`       | Binding invariants (durability, credentials, operational honesty) |
| Integration model      | `docs/INTEGRATION_MODEL.md`   | Authoritative integration mechanics (canon §3.5 points here) |
| Maturity model         | `docs/MATURITY.md`            | L0–L4 definitions per area |
| Observability canon    | `docs/OBSERVABILITY.md`       | Metrics / tracing / logging boundaries |
| Security policy        | `.github/SECURITY.md`         | Reporting vulnerabilities |
| Architecture decisions | `docs/adr/` | ADRs 0042+ active; 0001–0041 index at `docs/adr/HISTORICAL.md` |
| Per-crate READMEs      | `crates/<crate>/README.md`    | Crate-level usage and design notes |
| Per-crate design docs  | `crates/<crate>/docs/`        | Where present |
| Recurring pitfalls     | `docs/pitfalls.md`            | Source of truth for trap classes |
| GitHub project setup   | `.github/PROJECT_SETUP.md`    | Repo / project board configuration |

## AI Context Files

| File                          | Purpose |
|-------------------------------|---------|
| `CLAUDE.md`                   | **Canonical** agent rules + project map — every AI tool should read this |
| `AGENTS.md`                   | Thin pointer naming `CLAUDE.md` canonical (cross-tool stub) |
| `.ai-factory/config.yaml`     | AI Factory settings (language, paths, git, rules) |
| `.ai-factory/DESCRIPTION.md`  | Agent-facing project summary |
| `.ai-factory/ARCHITECTURE.md` | Agent-actionable architecture subset |
| `.ai-factory/rules/base.md`   | Distilled coding rules for agents |
| `.ai-factory.json`            | AI Factory install manifest (managed by tooling) |
| `.github/copilot-instructions.md` | GitHub Copilot guidance (defers to `CLAUDE.md`) |
| `.cursor/rules/*.mdc`         | Cursor project rules that defer to `CLAUDE.md` |
| `.claude/hooks/`              | Committed guard hooks (enforced discipline) |
| `.claude/skills/`             | Claude Code `/aif-*` skill definitions |
| `.claude/skills/rust-intel/`  | Vendored LLM-Rust-failure-mode skill — v0.2.2, MIT, advisory (not hook-enforced); `/rust-cc-{audit,fix,plan}` (see its `UPSTREAM.md`) |
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

## Claude Code

- This file (`CLAUDE.md`) is the canonical source of truth for project rules;
  `AGENTS.md` only points here.
- Daily commands go through `task` (see Common Commands above). Don't
  call raw `cargo` for fmt/lint — `task fmt` runs `cargo fmt --all` on the
  pinned stable toolchain (per `rustfmt.toml`) and `task clippy` runs with
  `-D warnings`. Note: `task test` / `task test:crate`
  use plain `cargo test`; the workspace pre-PR gate (`task dev:check`) runs
  nextest. For nextest on a single crate: `cargo nextest run -p <crate>`.
- For persistent Nebula task branches, create worktrees with
  `bash scripts/worktree.sh new <slug> <type> <scope>`.
- After the task PR is merged, clean up with
  `bash scripts/worktree.sh finish <slug>`.
- Do not rely on Claude's default `--worktree` location for persistent repo
  work unless the user explicitly asks for a disposable Claude-managed
  worktree.

## Enforced Discipline (guard hooks)

Mechanically enforced by `.claude/hooks/*.sh` (committed in `.claude/settings.json`),
not advisory. `task hooks:test` proves each guard. **The no-cheat guarantee is
structural (D10): B (edit-guard) + A2 (clean-gate recorder) + C (Stop-gate) +
lefthook/CI.** Hook A is a **fail-open advisory tripwire**, not a security
boundary — it nudges on blatant literals only.
`intent-gate.sh` (Layer-2, ADR-0083) is a deterministic structural-budget
**addition above** D10 — it does not alter the D10 core; `stop-gate.sh` still
runs first and remains the guarantee.

| Rule | Guard |
|------|-------|
| Nudge: blatant `git commit --no-verify` / `cargo fmt --all` / `git push --force` | `bash-deny.sh` (advisory, fail-open) |
| Lint-suppressed clippy never counts as a passing gate | `record.sh` (A2) |
| No `unwrap()/expect()/panic!()` in lib code | `edit-guard.sh` |
| `#[allow]/todo!/unimplemented!/unreachable!` need `// guard-justified:` | `edit-guard.sh` |
| No TODO/FIXME/HACK/plan-id in committed code | `edit-guard.sh` |
| No test-weakening while impl changed same turn | `edit-guard.sh` |
| Cannot end a turn with impl changed but no green clippy+nextest | `stop-gate.sh` |
| Layer-2: turn diff over structural budget (net-LoC / new-files / blob / dup symbol) | `intent-gate.sh` (deterministic, ADR-0083; bench / migration / golden-snapshot paths + files with `@generated` are auto-exempt; `// budget-justified: <reason>` or `# budget-justified: <reason>` escape capped at 2 markers/turn with a ≥30-char + keyword quality bar on the blob check) |

Escape hatch for discretionary edit rules: a `// guard-justified: <reason>` line
directly above the construct. No escape for lefthook-bypass, lint-suppression,
or no-unwrap.
