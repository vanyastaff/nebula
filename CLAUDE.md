@AGENTS.md

## Claude Code

- Treat `AGENTS.md` as the source of truth for project rules.
- For persistent Nebula task branches, create worktrees with
  `bash scripts/worktree.sh new <slug> <type> <scope>`.
- Do not rely on Claude's default `--worktree` location for persistent repo
  work unless the user explicitly asks for a disposable Claude-managed
  worktree.
