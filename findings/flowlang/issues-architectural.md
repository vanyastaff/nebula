# Issues — flowlang

## Issue count
GitHub API reports 0 open issues and 0 closed issues (empty tracker).
One open issue appears via `gh api repos/mraiser/flow/issues?state=all`
but had no content in the paginated output — likely a stale/spam entry.

## Issue citation requirement
The quality gate requires ≥3 cited issues for Tier 1/2 projects with
>100 closed issues. This repo has <10 total issues; the requirement is
not applicable (does not meet the >100 threshold).

## GitHub metadata (as of 2026-04-26)
- Stars: 11
- Forks: 1
- Open issues: 0
- License: MIT (README says MIT; GitHub API says "Other")
- Language: Rust

## Notable discussion points (from README / commit log)
- Recent commit: "Support for lib.so with FFI provides dependency
  isolation" — this indicates the hot-reload / dylib feature is
  recently added (commit `7a75693`).
- Commit: "add MCP, misc fixes, update README" — MCP support is recent
  (`e64964f`).
- Commit: "hot-reload is fine" — suggests prior instability in
  hot-reload that was just stabilized (`049dbbf`).
- Commit: "Rust code generation fixes" (×2) — builder code gen was
  buggy recently.
