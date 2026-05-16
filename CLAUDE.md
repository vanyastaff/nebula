@AGENTS.md

## Claude Code

- Treat `AGENTS.md` as the source of truth for project rules.
- Daily commands go through `task` (see Common Commands in AGENTS.md). Don't
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

Mechanically enforced by `scripts/guard/*.sh` (committed in `.claude/settings.json`),
not advisory. `task hooks:test` proves each guard. **The no-cheat guarantee is
structural (D10): B (edit-guard) + A2 (clean-gate recorder) + C (Stop-gate) +
lefthook/CI.** Hook A is a **fail-open advisory tripwire**, not a security
boundary — it nudges on blatant literals only. Plan 2 makes this file canonical.

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
