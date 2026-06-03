---
name: nebula-worktree-pr-workflow
description: Use when starting a branch, committing, opening a PR, or hitting lefthook / convco / cargo-deny / fmt / clippy gate failures in this workspace.
---

# Nebula worktree + PR workflow

Procedure for branching, committing, gating, and finishing work in this Rust
workspace. Authoritative sources: `CLAUDE.md` (Agent Git Workflow / Agent Rules
/ Common Commands / Enforced Discipline), `scripts/worktree.sh`, `lefthook.yml`,
`.github/workflows/ci.yml`, `Taskfile.yml`.

## When to use

Creating a task branch, writing a commit message, running the pre-PR gate,
finishing/merging a branch, or diagnosing a lefthook / CI required-job failure
(`fmt`, `clippy`, `nextest`, `convco`, `deny`).

## 1. Start a branch (from `origin/main`)

```bash
bash scripts/worktree.sh new <slug> <type> <scope>
```

- Creates `.worktrees/<slug>` and branch `<type>/<scope>-<slug>`, based on
  `origin/main` (`DEFAULT_BASE` in `scripts/worktree.sh`; override with a 4th
  `[base]` arg or `NEBULA_WORKTREE_BASE`).
- The script `slugify`s name/type/scope and rejects empty values, `.`/`..`, and
  path separators. It fails fast if the path or local branch already exists.
- Equivalent task wrapper: `task wt:new` with `WT_NAME` / `WT_TYPE` / `WT_SCOPE`
  env vars. List with `bash scripts/worktree.sh list` (`task wt:list`).
- **Branch from `main`, squash-merge back, never force-push shared history**
  (CLAUDE.md Agent Rules).
- Managed cloud/IDE worktrees that can't live under `.worktrees/` still follow
  these branch/commit rules — CI and lefthook are the gate.

## 2. Commit (Conventional Commits, convco-validated)

```bash
git add <paths>
bash scripts/worktree.sh commit <type> <scope> <summary...>
```

- Emits `<type>(<scope>): <summary>` and validates it with `convco check
  --from-stdin` before `git commit`. Fails if there are no staged changes.
- The `commit-msg` lefthook (`cat {1} | convco check --from-stdin`) re-validates
  on every commit, regardless of harness.
- **Allowed types** (from `VALID_TYPES` in `scripts/worktree.sh`): `build`,
  `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`, `revert`, `style`,
  `test`.
- **Scope** = crate name without the `nebula-` prefix (`resilience`, `engine`,
  `api`) or a top-level area (`docs`, `ci`, `scripts`, `github`).
- Task wrapper: `task git:commit` with `WT_TYPE` / `WT_SCOPE` / `WT_MSG`.

**Decompose chained git commands.** Run each git op as its own step so pass/fail
is clear; do not chain unrelated git operations.
- Wrong: `git checkout main && git pull`
- Right: `git checkout main`, then `git pull origin main`.

## 3. Pre-PR gate

Authoritative local gate before opening a PR:

```bash
task dev:check
```

Runs (see `Taskfile.yml` `dev:check`): `fmt:check` → `clippy` (`-D warnings`) →
`cargo nextest run --workspace` → `cargo test --workspace --doc` → `deny`.

Fast single-crate inner loop (don't run the full workspace every edit):

| Need | Command |
|------|---------|
| Type-check one crate | `cargo check -p <crate>` |
| Run one crate's tests | `cargo nextest run -p <crate>` |
| One test by name | `cargo nextest run -p <crate> <test>` |
| Doctests for one crate | `cargo test -p <crate> --doc` |
| Supply-chain audit | `task deny` (`cargo deny check`) |

> On Windows deep worktree paths `task dev:check` (and `task fmt`) trip the
> `cargo fmt --all` failure — see §6. Verify formatting per-crate; CI Linux
> `fmt` is the authoritative formatting gate.

## 4. Three hook layers — what each owns

| Layer | Trigger | Owns |
|-------|---------|------|
| `.claude/hooks/*.sh` | Claude Code per-turn (edit-guard / stop-gate) | No-unwrap/expect/panic in lib code, no TODO/FIXME/plan-ids, no test-weakening, and the Stop-gate: can't end a turn with impl changed but no green clippy + nextest |
| `lefthook.yml` | `git commit` / `git push` (any harness) | Pre-commit + pre-push gates (§5) |
| `.github/workflows/ci.yml` | PR / merge_group | Required jobs: `fmt`, `clippy`, `check`, `doctests`, `msrv`, `doc`, `deny` (aggregated into the single required `CI` check) |

**`lefthook pre-push` MUST mirror CI required jobs.** If you change a CI required
job, change pre-push to match, and vice versa (CLAUDE.md Agent Rules;
`feedback_lefthook_mirrors_ci.md`). Note CI also runs `check` (incl.
`--all-features` and several `--no-default-features` crate checks), `doc`
(`RUSTDOCFLAGS=-D warnings cargo doc --no-deps`), and `msrv` (1.95) — these are
CI-owned and not in pre-push, so a green pre-push does not guarantee green CI.

## 5. Commit granularity and the lefthook gates

From `lefthook.yml`:

- **pre-commit** (parallel, changed-crate scoped → cheap):
  - `fmt-check` — `scripts/pre-commit-fmt-check.sh` on staged `.rs` (per-crate,
    avoids the Windows `cargo fmt --all` cmdline-length break).
  - `clippy` — `scripts/pre-commit-clippy-changed.sh` on staged `.rs` (only the
    crates owning staged files; full-workspace clippy moved to pre-push).
  - `typos` (whole tree), `taplo fmt --check` (staged `.toml`), `cargo deny`
    (staged `Cargo.toml` / `Cargo.lock` / `deny.toml`).
  - Docs-only / toml-only commits skip the clippy/fmt crate steps → cheap.
- **pre-push** (serial, ~100s):
  - `clippy-full` — `cargo clippy --workspace --all-targets -- -D warnings`
    (CI `clippy` parity).
  - `crate-diff-gate` — `scripts/pre-push-crate-diff.sh`: `cargo nextest run`
    for changed crates; runs `--features postgres` integration tests only when
    `DATABASE_URL` is set, otherwise emits a single WARN line and skips them.

**Commit big refactors at per-touched-crate-green points** so each atomic commit
keeps the changed crates green; an as-yet-untouched crate won't go red because
pre-commit clippy is changed-crate scoped.

## 6. Windows worktree gotchas

- `cargo fmt --all` trips **OS error 206** (command-line too long) in deep
  `.worktrees/` paths. Verify formatting per-crate (`cargo fmt -p <crate> --
  --check`); do **not** report `task dev:check` / `task fmt:check` green from a
  deep worktree. CI Linux `fmt` (`cargo fmt --all -- --check`) is the real gate.
- A backgrounded `git push` looks hung but is just the slow pre-push hook
  (full-workspace clippy + crate-diff nextest, ~100s). **Don't re-push** on the
  first "not pushed yet" reading — wait it out.

## 7. Cargo.lock discipline

- On **any dependency add/change**, stage the **root `Cargo.lock`** too — not
  just `crates/<name>`. Too-narrow staging breaks per-commit `--locked` builds.
- Resolve lockfile **rebase conflicts** with `git checkout --theirs Cargo.lock`
  (then let cargo reconcile). Do **not** `cargo update -p <pkg>` to fix a
  conflict.

## 8. Never bypass gates

- No `git commit --no-verify`, no `git push --no-verify`, no `git push --force`
  on shared history (`bash-deny.sh` nudges; the permission system denies the
  blatant forms).
- No `#[allow(...)]` / lint-suppression to fake a green clippy — a
  lint-suppressed clippy never counts as a passing gate (`record.sh` A2).
- No `unwrap()` / `expect()` / `panic!()` in library code (`edit-guard.sh`);
  tests, `const`, and binaries are exempt per `clippy.toml`.

## 9. Finish (after the PR is merged)

```bash
bash scripts/worktree.sh finish <slug>
```

- Requires a clean checkout in both the target worktree and the primary; it
  switches the primary to `main`, `fetch` + `pull --ff-only`s from the tracked
  remote, removes `.worktrees/<slug>`, prunes, and deletes the merged local
  branch (`git branch -d`, so it refuses if the branch isn't merged).
- Task wrapper: `task wt:finish` (`WT_NAME`). Drop a worktree without the
  main-sync via `bash scripts/worktree.sh remove <slug>` (`task wt:remove`).

## Failure → fix quick map

| Symptom | Cause / fix |
|---------|-------------|
| `convco check` fails | Message isn't `<type>(<scope>): <summary>` or `<type>` not in the allowed list (§2). Re-run via `scripts/worktree.sh commit`. |
| pre-commit `clippy` red | Changed-crate clippy on staged `.rs`. Fix the lint — do not `#[allow]` it (§8). |
| pre-push `clippy-full` red | Full-workspace `-D warnings`. Run `task clippy` locally first. |
| `cargo deny check` fails | Layer-wrapper / advisory / license violation. Inspect `deny.toml`; if a dep changed, also stage root `Cargo.lock` (§7). |
| `fmt` red in CI but local fmt "passed" | Likely the Windows OS-error-206 false pass (§6). Verify per-crate. |
| pre-push seems hung | Slow pre-push hook, not a hang — wait, don't re-push (§6). |
