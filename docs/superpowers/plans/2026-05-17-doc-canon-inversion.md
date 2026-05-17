# Doc-Canon Inversion (D8) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `CLAUDE.md` the single canonical agent-rules & project-map file; reduce `AGENTS.md` to a thin pointer that names `CLAUDE.md` canonical; repoint `.cursor/rules/*` and `.github/copilot-instructions.md` at `CLAUDE.md`.

**Architecture:** This is a paired doc inversion. Today `CLAUDE.md` is a one-line `@AGENTS.md` shim and `AGENTS.md` carries the canonical body. We flip them in **one atomic task** (they must change together — a half-flipped pair is internally contradictory), then repoint the two cross-tool reference files, then run a scripted acceptance gate that mirrors spec §11 D8. There is no code; the falsifiable oracle is a block of `grep` assertions plus a guard-hook regression run.

**Tech Stack:** Markdown, `grep`, `git`, `lefthook` (typos + convco run for docs), `task hooks:test`.

**Plan series (Plan 2 of 4 — spec `docs/superpowers/specs/2026-05-16-agent-discipline-and-curation-design.md`, decision D8; spec §10 item 2; acceptance §11 D8):** 1=guard-hooks subsystem (done, this branch); **2=this**; 3=skill+subagent curation (G/H); 4=lefthook granularity (F)+`nebula-pitfalls` (E). All four land on the same branch `goofy-cannon-4f7ebe`.

**Owner decision context (durable):** D8 is *non-standard* — `AGENTS.md`-as-canon is the cross-tool norm. The owner explicitly chose `CLAUDE.md`-as-canon with the trade-off shown (non-Claude `AGENTS.md`-only tools see only the pointer). Do **not** "fix" this back. The thin `AGENTS.md` still inlines the irreducible branch/commit safety rules so a committing tool that reads only `AGENTS.md` cannot get them wrong; everything else defers to `CLAUDE.md`. Out of scope: `.ai-factory/*` (AI-Factory-managed, base files overwritten on update per D5/Rule 7) — Plan 3 territory; this plan only *reports* any `.ai-factory` mention, never edits it.

---

## File Structure

| File | Responsibility after this plan |
|------|--------------------------------|
| `CLAUDE.md` | **Canonical.** Self-contained: canonical-declaration header + full project map (Overview, Tech Stack, Common Commands, Workspace Layout, Layered Dependency Map, Key Entry Points, Documentation Index, AI Context Files, Agent Git Workflow, Agent Rules) + `## Claude Code` section + `## Enforced Discipline (guard hooks)` section. No `@AGENTS.md` import. |
| `AGENTS.md` | Thin cross-tool pointer: project one-liner + bold "canonical = CLAUDE.md" + the irreducible Git-workflow summary. Defers everything else to `CLAUDE.md`. |
| `.cursor/rules/agent-workflow.mdc` | Same worktree guidance, but source-of-truth line points at `CLAUDE.md`. |
| `.github/copilot-instructions.md` | Same review guidance, but source-of-truth line points at `CLAUDE.md`. |

Files that change together (`CLAUDE.md` + `AGENTS.md`) live in one task/commit per the writing-plans File-Structure principle; the two independent cross-tool files get their own tasks.

---

### Task 1: Invert the canon pair — `CLAUDE.md` canonical, `AGENTS.md` thin pointer

**Files:**
- Rewrite: `CLAUDE.md` (currently 40 lines: `@AGENTS.md` + `## Claude Code` + `## Enforced Discipline`)
- Rewrite: `AGENTS.md` (currently 218 lines: the canonical body)

- [ ] **Step 1: Write the falling acceptance assertion (must fail before the edit)**

Create `/tmp/d8-check.sh` with this exact content and run it; it MUST fail now (pre-inversion):

```bash
#!/usr/bin/env bash
# D8 acceptance gate — mirrors spec §11 D8. Exit 0 = all hold.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
f=0; ok(){ printf 'ok   - %s\n' "$1"; }; no(){ printf 'FAIL - %s\n' "$1"; f=1; }
grep -q 'Canonical agent-rules & project map' CLAUDE.md && ok "CLAUDE.md declares canonical" || no "CLAUDE.md declares canonical"
grep -q '^## Enforced Discipline (guard hooks)$' CLAUDE.md && ok "CLAUDE.md keeps Enforced Discipline" || no "CLAUDE.md keeps Enforced Discipline"
grep -q '^## Layered Dependency Map$' CLAUDE.md && ok "CLAUDE.md absorbed project map" || no "CLAUDE.md absorbed project map"
! grep -qx '@AGENTS.md' CLAUDE.md && ok "CLAUDE.md has no @AGENTS.md import" || no "CLAUDE.md still imports AGENTS.md"
grep -q 'CLAUDE.md' AGENTS.md && ok "AGENTS.md points at CLAUDE.md" || no "AGENTS.md points at CLAUDE.md"
[ "$(wc -l < AGENTS.md)" -lt 40 ] && ok "AGENTS.md is thin (<40 lines)" || no "AGENTS.md not thin"
if grep -rInE 'Treat `?AGENTS\.md`? as the source|Use `?AGENTS\.md`? as the source|AGENTS\.md`? (is|as) the source of truth' CLAUDE.md AGENTS.md .cursor/rules/ .github/copilot-instructions.md ; then no "stale 'AGENTS.md is source of truth' wording remains"; else ok "no 'AGENTS.md source of truth' wording"; fi
grep -q 'CLAUDE.md' .cursor/rules/agent-workflow.mdc && ok ".cursor resolves to CLAUDE.md" || no ".cursor resolves to CLAUDE.md"
grep -q 'CLAUDE.md' .github/copilot-instructions.md && ok ".github resolves to CLAUDE.md" || no ".github resolves to CLAUDE.md"
[ "$f" -eq 0 ] && echo "D8 ACCEPTANCE PASSED" || echo "D8 ACCEPTANCE FAILED"
exit "$f"
```

Run: `bash /tmp/d8-check.sh`
Expected: **FAIL** — `CLAUDE.md declares canonical`, `absorbed project map`, `no @AGENTS.md import`, `AGENTS.md is thin`, `.cursor/.github resolve to CLAUDE.md`, and `no 'AGENTS.md source of truth'` all fail (pre-inversion `CLAUDE.md` is `@AGENTS.md` + says "Treat `AGENTS.md` as the source of truth"; `.cursor`/`.github` say "Use `AGENTS.md` as the source of truth").

- [ ] **Step 2: Rewrite `CLAUDE.md` with this exact content**

```markdown
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
| Business     | `credential-builtin`, `resource`, `action`, `plugin` |
| Plugin-Proto | `plugin-sdk`, `sandbox` |
| Core         | `core`, `validator`, `expression`, `workflow`, `execution`, `schema`, `metadata` |
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
| Agent rules + map      | `CLAUDE.md`                   | **Canonical** — this file — branch / commit / PR rules + project map + enforced discipline |
| Product overview       | `README.md`                   | What Nebula is, design principles, architecture |
| Cross-tool pointer     | `AGENTS.md`                   | Thin pointer naming `CLAUDE.md` canonical |
| Security policy        | `.github/SECURITY.md`         | Reporting vulnerabilities |
| Architecture decisions | `docs/adr/`                   | M6 / M11 cascade ADRs (0042+) accepted in this worktree (ADR-0047 covers OpenAPI 3.1 generation + Stub Endpoint Policy) |
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
  call raw `cargo` for fmt/lint — `task fmt` requires nightly rustfmt and
  `task clippy` runs with `-D warnings`. Note: `task test` / `task test:crate`
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

| Rule | Guard |
|------|-------|
| Nudge: blatant `git commit --no-verify` / `cargo fmt --all` / `git push --force` | `bash-deny.sh` (advisory, fail-open) |
| Lint-suppressed clippy never counts as a passing gate | `record.sh` (A2) |
| No `unwrap()/expect()/panic!()` in lib code | `edit-guard.sh` |
| `#[allow]/todo!/unimplemented!/unreachable!` need `// guard-justified:` | `edit-guard.sh` |
| No TODO/FIXME/HACK/plan-id in committed code | `edit-guard.sh` |
| No test-weakening while impl changed same turn | `edit-guard.sh` |
| Cannot end a turn with impl changed but no green clippy+nextest | `stop-gate.sh` |

Escape hatch for discretionary edit rules: a `// guard-justified: <reason>` line
directly above the construct. No escape for lefthook-bypass, lint-suppression,
or no-unwrap.
```

- [ ] **Step 3: Rewrite `AGENTS.md` with this exact content**

```markdown
# AGENTS.md

> **Canonical agent rules and the full Nebula project map live in
> [`CLAUDE.md`](CLAUDE.md).** Read that file — it is the single source of
> truth for layout, architecture, commands, Git workflow, coding rules, and
> enforced discipline. This file is an intentionally thin cross-tool pointer:
> the owner chose `CLAUDE.md` (not `AGENTS.md`) as canonical, accepting that
> `AGENTS.md`-only tools see just this stub.

Nebula is a modular, type-safe **workflow automation engine** in Rust
(edition 2024, MSRV 1.95). Product overview: `README.md`.

## Git workflow (summary — `CLAUDE.md` is authoritative)

- Branch from `origin/main`; squash-merge back; never force-push shared
  history without explicit confirmation.
- Persistent task branches go through
  `bash scripts/worktree.sh new <slug> <type> <scope>` (creates
  `.worktrees/<slug>` and branch `<type>/<scope>-<slug>`); finish with
  `bash scripts/worktree.sh finish <slug>`.
- Conventional Commits, validated by `convco`. **Allowed types:** `build`,
  `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`, `revert`,
  `style`, `test`. **Scope:** crate name without the `nebula-` prefix, or a
  top-level area (`docs`, `ci`, `scripts`, `github`).
- Managed cloud/IDE worktrees that cannot live under `.worktrees/` still
  follow these branch/commit rules — CI and `lefthook` are the gate.

Everything else — project map, layered dependency map, command catalog, full
agent rules, and the mechanically enforced discipline (guard hooks) — is in
[`CLAUDE.md`](CLAUDE.md).
```

- [ ] **Step 4: Run the acceptance gate**

Run: `bash /tmp/d8-check.sh`
Expected: every line `ok`, ending `D8 ACCEPTANCE PASSED`, exit 0. (At this point `.cursor`/`.github` lines still pass only if they already contain the string `CLAUDE.md` — they do not yet, so expect those two specific lines to still FAIL; Tasks 2–3 fix them. All `CLAUDE.md`/`AGENTS.md` lines MUST pass now.)

To isolate: `bash /tmp/d8-check.sh | grep -E 'CLAUDE.md|AGENTS.md is thin|no .AGENTS'` must be all `ok`.

- [ ] **Step 5: Commit**

```bash
git add CLAUDE.md AGENTS.md
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -F- <<'EOF'
docs: invert doc canon — CLAUDE.md canonical, AGENTS.md thin pointer (D8)

Spec D8 (owner, non-standard by choice): CLAUDE.md is now the single
source of truth (project map + layer map + commands + Git workflow +
agent rules + Enforced Discipline). AGENTS.md drops from 218 lines to a
thin cross-tool pointer that names CLAUDE.md canonical and inlines only
the irreducible branch/commit safety rules. CLAUDE.md no longer
@-imports AGENTS.md; the internal Documentation Index / AI Context
Files / Workspace tree now describe the inverted topology.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
```

---

### Task 2: Repoint `.cursor/rules/agent-workflow.mdc` at `CLAUDE.md`

**Files:**
- Modify: `.cursor/rules/agent-workflow.mdc:7-8` (the source-of-truth sentence)

- [ ] **Step 1: Verify the stale wording is present (failing precondition)**

Run: `grep -n 'AGENTS.md' .cursor/rules/agent-workflow.mdc`
Expected: line 7 prints ``Use `AGENTS.md` as the source of truth for project layout, architecture, Git`` (stale — points at the now-non-canonical file).

- [ ] **Step 2: Replace the source-of-truth sentence**

Replace exactly this block:

```
Use `AGENTS.md` as the source of truth for project layout, architecture, Git
workflow, and coding rules.
```

with:

```
Use [`CLAUDE.md`](../../CLAUDE.md) as the canonical source of truth for project
layout, architecture, Git workflow, and coding rules. (`AGENTS.md` is only a
thin pointer to it.)
```

Leave the rest of the file (the `bash scripts/worktree.sh …` worktree guidance and the "same branch and commit rules as Claude, Codex, and Copilot" line) unchanged.

- [ ] **Step 3: Verify**

Run: `grep -nE 'CLAUDE.md|AGENTS.md' .cursor/rules/agent-workflow.mdc`
Expected: the source-of-truth line now names `CLAUDE.md`; the only `AGENTS.md` mention is the parenthetical "is only a thin pointer to it". No ``Use `AGENTS.md` as the source`` remains.

- [ ] **Step 4: Commit**

```bash
git add .cursor/rules/agent-workflow.mdc
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "docs: .cursor rule defers to CLAUDE.md canonical (D8)"
```

---

### Task 3: Repoint `.github/copilot-instructions.md` at `CLAUDE.md`

**Files:**
- Modify: `.github/copilot-instructions.md:11` (the source-of-truth sentence in `## Agent Git Workflow`)

- [ ] **Step 1: Verify the stale wording is present (failing precondition)**

Run: `grep -n 'AGENTS.md' .github/copilot-instructions.md`
Expected: line 11 prints ``Use `AGENTS.md` as the source of truth for repository rules. For local`` (stale).

- [ ] **Step 2: Replace the source-of-truth sentence**

Replace exactly this block:

```
Use `AGENTS.md` as the source of truth for repository rules. For local
persistent task branches, create worktrees with:
```

with:

```
Use [`CLAUDE.md`](../CLAUDE.md) as the canonical source of truth for repository
rules (`AGENTS.md` is only a thin pointer to it). For local persistent task
branches, create worktrees with:
```

Leave everything else (Project Context, the worktree commands, "What to Flag in Reviews", stop-list, project-specific patterns) unchanged.

- [ ] **Step 3: Verify**

Run: `grep -nE 'CLAUDE.md|AGENTS.md' .github/copilot-instructions.md`
Expected: line 11 names `CLAUDE.md`; the only `AGENTS.md` mention is the "only a thin pointer" parenthetical. No ``Use `AGENTS.md` as the source`` remains.

- [ ] **Step 4: Commit**

```bash
git add .github/copilot-instructions.md
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -m "docs: copilot-instructions defers to CLAUDE.md canonical (D8)"
```

---

### Task 4: Full acceptance gate + residual sweep + regression

**Files:**
- Read-only verification across the repo; commit any residual-sweep fixes found.

- [ ] **Step 1: Run the full D8 acceptance gate (now must fully pass)**

Run: `bash /tmp/d8-check.sh`
Expected: every line `ok`, ending `D8 ACCEPTANCE PASSED`, exit 0 — including the `.cursor`/`.github` lines (fixed by Tasks 2–3).

- [ ] **Step 2: Repo-wide residual sweep for stale canon wording**

Step 1's gate is the **authoritative** acceptance check (its negative assertion is AGENTS.md-anchored: it fails only if some file still claims *AGENTS.md* is the source of truth). This step is a secondary belt for stale `@AGENTS.md` imports / "shim" wording / AGENTS-as-canon pointers anywhere else in the repo. The regex MUST stay anchored on `AGENTS.md` — a generic `source of truth for (project|repository)` match is wrong: post-inversion that phrase legitimately describes **CLAUDE.md** (`.cursor`, `.github`, `CLAUDE.md` all correctly say "Use `CLAUDE.md` as the canonical source of truth for project/repository …").

Run:

```bash
git grep -nE '@AGENTS\.md|shim that imports|(Treat|Use) `?AGENTS\.md`? as (the )?(canonical )?source|`?AGENTS\.md`? (is|as) the (canonical )?source of truth|points? (back )?to `?AGENTS\.md`? (as|for) (the )?(source|canon|truth|rules|map)' -- ':!docs/superpowers' ':!.ai-factory'
```

Expected: **no output**. Any hit outside `docs/superpowers` (this plan/spec, which legitimately discuss the inversion) and `.ai-factory` (AI-Factory-managed, out of scope — Plan 3) is a genuine residual: fix it to the post-inversion wording using the same patterns as Tasks 1–3 (name `CLAUDE.md` canonical; `AGENTS.md` = thin pointer), then re-run Step 1.

- [ ] **Step 3: Report (do NOT edit) any `.ai-factory` canon mention**

Run:

```bash
grep -rInE 'AGENTS\.md|source of truth' .ai-factory/ 2>/dev/null || echo "none"
```

This is informational only. `.ai-factory/*` base files are overwritten on AI-Factory update (D5 / Rule 7); they are explicitly **out of scope** for Plan 2 and handled in Plan 3 via `.ai-factory/skill-context/`. Record the output in the commit body so Plan 3 has the inventory; do not modify `.ai-factory/`.

- [ ] **Step 4: Guard-hook regression (CLAUDE.md is named in the enforced-discipline map)**

Run: `bash .claude/hooks/test/run.sh`
Expected: `ALL GUARD TESTS PASSED`, exit 0. (Sanity: the doc inversion must not perturb the Plan-1 subsystem; CLAUDE.md is referenced by the Enforced Discipline section but the harness does not parse CLAUDE.md.)

Run: `task hooks:test`
Expected: `[hooks:test] ALL GUARD TESTS PASSED`.

- [ ] **Step 5: Lefthook docs gate dry sanity (typos + convco run for `.md`)**

The Task 1–3 commits already passed `lefthook pre-commit` (typos) + `commit-msg` (convco). Confirm tree clean and the four commits present:

```bash
git status --porcelain
git log --oneline -5
```

Expected: clean working tree; the last commits include the Task 1 `docs: invert doc canon …`, Task 2 `.cursor`, Task 3 `copilot-instructions`, and (if Step 2 found residuals) the sweep fix.

- [ ] **Step 6: Commit any residual-sweep fixes**

Two cases:

- **Step 2 found residuals** (you edited files to fix stale canon wording): stage and commit them, embedding the Step 3 `.ai-factory` inventory in the body so Plan 3 has it:

```bash
git add -A
git -c user.name="vanyastaff" -c user.email="ivan.kondrashkin@gmail.com" commit -F- <<'EOF'
docs: residual doc-canon sweep — no AGENTS.md-as-canon wording outside scope (D8)

D8 acceptance gate green. .ai-factory/ canon mentions left intact
(AI-Factory-managed, Plan 3 scope) — inventory recorded for Plan 3:
<paste Step 3 output here>

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
```

- **Step 2 found nothing** (clean — the expected case if Tasks 1–3 were complete): there is nothing to commit. Do not create an empty commit. Carry the Step 3 `.ai-factory` inventory forward in the final execution report instead, prefixed `Plan 3 .ai-factory canon inventory:`.

- [ ] **Step 7: Clean up the temp gate script**

Run: `rm -f /tmp/d8-check.sh`
(The acceptance gate is a one-shot verification artifact, not a committed test — D8 is a docs invariant, re-checkable any time by re-deriving the grep block from spec §11 D8. Do not commit `/tmp/d8-check.sh`.)

---

## Self-Review

**1. Spec coverage (spec §11 D8 → task):**
- "CLAUDE.md opens by declaring itself canonical" → Task 1 Step 2 header + gate line 1 ✓
- "carries the Enforced Discipline section" → Task 1 Step 2 keeps `## Enforced Discipline` + gate line 2 ✓
- "AGENTS.md is a pointer naming CLAUDE.md" → Task 1 Step 3 + gate lines 5–6 ✓
- "`.cursor/rules/*` and `.github/copilot-instructions.md` resolve to CLAUDE.md" → Tasks 2, 3 + gate lines 8–9 ✓
- "no remaining 'treat AGENTS.md as source of truth' wording" → gate line 7 + Task 4 Step 2 sweep ✓
- Spec §10 item 2 ("CLAUDE.md absorbs canonical content + gains Enforced Discipline; AGENTS.md → thin pointer; update .cursor + .github; drop the old line") → Tasks 1–4 ✓

**2. Placeholder scan:** No "TBD"/"similar to"/"handle appropriately". The full `CLAUDE.md` and `AGENTS.md` bodies are reproduced verbatim; the `.cursor`/`.github` edits give exact old/new blocks; the acceptance gate is complete runnable bash. Task 4 Step 6 has a conditional ("if residuals") — this is genuine branch logic, not a placeholder: the exact commands for both branches are given.

**3. Consistency:** The canonical-declaration string `Canonical agent-rules & project map` is identical in Task 1 Step 2 and the gate (Step 1). `## Enforced Discipline (guard hooks)` heading matches the gate's anchored `^## Enforced Discipline (guard hooks)$`. `.claude/hooks/*.sh` (post-Plan-1 relocation) is used, not the obsolete `scripts/guard/`. The new `AGENTS.md` is < 40 lines (gate asserts `< 40`; the reproduced body is ~30). No type/name drift (docs only).

**4. Scope:** Single coherent deliverable (the inverted canon). `.ai-factory/` correctly excluded with rationale. Same branch as Plan 1 (program lands together).

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-17-doc-canon-inversion.md`. This continues the same `goofy-cannon-4f7ebe` branch as Plan 1.
