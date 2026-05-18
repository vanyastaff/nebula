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

For **which markdown files are authoritative vs archive**, read
[`docs/README.md`](docs/README.md) before pulling product or ADR context.

Everything else — project map, layered dependency map, command catalog, full
agent rules, and the mechanically enforced discipline (guard hooks) — is in
[`CLAUDE.md`](CLAUDE.md).
