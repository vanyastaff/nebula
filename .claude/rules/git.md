# Git & PR Rules — Nebula

## Conventional Commits

PR titles and commit messages follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>
```

**Types:** `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `chore`
**Scope:** crate name without `nebula-` prefix: `feat(resilience): add adaptive rate limiter`

CI enforces this via `commitlint` and `action-pr-title`.

## Branch Naming

```
<type>/<short-description>
```

Examples: `feat/adaptive-rate-limiter`, `fix/circuit-breaker-race`, `refactor/retry-api`

## Pull Requests

- Title: conventional commit format, under 70 chars
- Body: use the PR template (Description, Changes, Testing, Breaking Changes)
- Labels: auto-applied by `.github/labeler.yml` + manual `type:*` and `area:*` labels
- One logical change per PR — don't bundle unrelated refactors

### Label system

**Type labels** (one required): `type:bug`, `type:feature`, `type:enhancement`, `type:chore`, `type:docs`
**Area labels** (optional): `area:action`, `area:engine`, `area:credential`, `area:runtime`
**Crate labels** (auto): `nebula-*` labels match crate names
**Priority**: `high-priority`, `medium-priority`

## Commit Hygiene

- Each commit should compile and pass tests independently
- Don't commit generated files, `.env`, credentials, or large binaries
- Prefer specific `git add <file>` over `git add .`
- Co-author line: `Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>`

## CI Pipeline

All PRs must pass: `fmt` → `clippy` → `check` → `test` (nextest) → `doc` → `typos` → `MSRV` → `deny`

Before pushing, run locally:
```bash
cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace
```
