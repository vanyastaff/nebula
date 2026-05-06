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
