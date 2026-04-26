# z8run — Issues Sweep

## Repository stats (as of research date: 2026-04-26)
- Stars: 5
- Forks: 2
- Open issues: 4 (per GitHub API `open_issues_count`)
- Actual issues (non-PR): 0 found via `gh issue list` — all 4 "open issues" appear to be open Dependabot PRs
- Closed issues: 0 (confirmed via `gh issue list --state closed` returns empty)

## Assessment
z8run has **no closed GitHub issues at all** and no architectural/design issues open.

The project is in very early stages (v0.1.0 released 2026-03-06, 51 days before research date). Community engagement is minimal.

The "issues" visible in the repo are all Dependabot automated dependency update PRs (`dependabot[bot]` as user, labels: `dependencies`, `javascript`).

## Threshold check for ≥3 cited issues
Per Worker Brief §1 rule 6: "Cite ≥ 3 GitHub issues for Tier 1/2 projects with >100 closed issues."
z8run has **0 closed issues**, well below the 100-closed threshold. No issue citations are required.

## Architectural signals from PR history
From the git log and CHANGELOG, architectural pain points inferred (no formal issues):
- CodeQL security alerts were raised and fixed in v0.2.0 (HTTPS enforcement, API key header placement, prototype pollution guard, manual SHA-256 replaced with `sha2` crate)
- MSRV mis-specified (stated 1.75, actually needed 1.91 — corrected in v0.2.0)
- SQLite migration V2's `ALTER TABLE ADD COLUMN` is wrapped in silent error continuation (`debug!("Migration statement possibly already applied")`) suggesting idempotency issues were encountered

## Noted roadmap gaps (from README `[ ]` items)
- Undo/redo in editor
- Flow duplication
- Node palette search
- Rate limiting on API (partially done in rate_limit.rs but not wired)
- Integration tests (zero integration tests exist)
- MySQL storage adapter (dependency present but no `mysql.rs`)
- Plugin marketplace
- Kubernetes Helm chart
