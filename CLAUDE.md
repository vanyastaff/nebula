@AGENTS.md

## Claude Code

- Treat `AGENTS.md` as the source of truth for project rules.
- Daily commands go through `task` (see Common Commands in AGENTS.md). Don't
  call raw `cargo` for fmt/test/lint — `task fmt` uses nightly rustfmt and
  `task test:crate CRATE=<name>` matches the project's nextest convention.
- For persistent Nebula task branches, create worktrees with
  `bash scripts/worktree.sh new <slug> <type> <scope>`.
- After the task PR is merged, clean up with
  `bash scripts/worktree.sh finish <slug>`.
- Do not rely on Claude's default `--worktree` location for persistent repo
  work unless the user explicitly asks for a disposable Claude-managed
  worktree.
