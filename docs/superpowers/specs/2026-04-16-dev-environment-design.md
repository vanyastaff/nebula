# Developer Environment & CI Hardening — Nebula

**Date:** 2026-04-16
**Author:** Claude (Opus 4.7) via brainstorming session
**Status:** DRAFT — awaiting user review before transition to `writing-plans`
**Scope:** Lefthook, Cargo config, CI workflows, CodeRabbit, Copilot review, Cursor Bugbot, Dependabot, modern Rust 2026 tooling additions

---

## TL;DR

Three concrete pains:

1. **Local hooks miss what CI catches.** `lefthook.yml` skips `taplo`, `cargo test --doc`, `cargo check --all-features`, `--no-default-features`, MSRV, and commit-msg lint. Result: pushes break CI, you fix, push again — the loop you wanted to avoid.
2. **CI duplicates work.** `ci.yml#test` (workspace nextest) and `test-matrix.yml#test-crates` (per-crate nextest) run the same tests twice. `Validate PR metadata` runs twice on `edited` events. `cross-platform-sandbox-smoke` runs on every PR even when sandbox/runtime/plugin-sdk are untouched. CodeQL scans `javascript-typescript` on a Rust workspace. PR wall-time is ~12-15 min.
3. **Bot noise.** Four reviewers per PR (CodeRabbit + Copilot + Cursor Bugbot + CodSpeed). CodeRabbit is set to `assertive` profile with `sequence_diagrams: true`, `incremental_reviews: true`, `auto_review.drafts: true`, `chat.auto_reply: true`, and `request_changes_workflow: true` — generating walls of nit-comments, mermaid diagrams, and merge blocks. Copilot review has no stop-list, so it adds generic "consider adding tests/docs/error handling" to every PR.

**Single biggest win:** mirror CI's required jobs in `pre-push` so nothing slips through, and dim CodeRabbit (`profile: chill`, kill drafts/incremental/diagrams/auto-reply/linear, add `path_instructions`). These two changes alone close ~80% of the pain.

**Approach (B from brainstorm):** quick alignment fixes + local toolchain speedups (`sccache`, `rust-lld`) + selected modern 2026 stack additions (`release-plz`, `cargo-shear`, `mise`, `cargo-semver-checks`).

**Numerical target:**

| | Now | After |
|---|---|---|
| PR wall-time (typical) | ~12-15 min | ~5-7 min |
| Pre-commit | ~10-15 sec | ~10-15 sec (taplo added, glob-filtered) |
| Pre-push | ~30 sec sequential | ~60-90 sec parallel, full CI mirror |
| Required reviewers | 3 (CodeRabbit blocks via request_changes) | 0 blocking, 2 advisory (CodeRabbit + Copilot), 1 advisory (Cursor) |
| Status checks per PR | 34 | ~20 |
| Cargo build/check (local) | baseline | -30 to -50% via sccache + rust-lld |

---

## 1. Problem inventory (full)

### 1.1 Lefthook ↔ CI divergence (root cause: pushes fail CI)

| Check | In `lefthook.yml`? | In CI? |
|---|---|---|
| `cargo +nightly fmt --check` | ✅ pre-commit | ✅ `ci.yml#fmt` |
| `cargo clippy --workspace -- -D warnings` | ✅ pre-commit | ✅ `ci.yml#clippy` |
| `cargo check` workspace | ✅ pre-commit (no features) | ✅ `ci.yml#check` (`--all-features`) |
| `cargo deny check` | ✅ pre-commit | ✅ `ci.yml#deny` |
| `typos` | ✅ pre-commit | ✅ `hygiene.yml` |
| `taplo fmt --check` | ❌ **missing** | ✅ `hygiene.yml` |
| `cargo nextest run --workspace` | ✅ pre-push | ✅ `ci.yml#test` + `test-matrix.yml` |
| `cargo test --workspace --doc` | ❌ **missing** | ✅ `ci.yml#test` |
| `cargo check --all-features` | ❌ **missing** | ✅ `ci.yml#check` |
| `cargo check --no-default-features` (resilience/log/expression) | ❌ **missing** | ✅ `ci.yml#check` |
| `cargo doc -D warnings` | ✅ pre-push | ✅ `ci.yml#doc` |
| MSRV (`+1.94 check`) | ❌ **missing** | ✅ `ci.yml#msrv` |
| Commitlint / conventional title | ❌ **missing** | ✅ `pr-validation.yml` |
| Cross-platform smoke (3 OS) | ❌ **missing** (correctly) | ✅ `test-matrix.yml` |
| `cargo audit` | ❌ (correctly, weekly) | ✅ `security-audit.yml` |
| `cargo udeps` | ❌ (correctly, weekly) | ✅ `udeps.yml` |

**Six gaps** are sources of "passed locally, failed in CI": taplo, doctests, all-features, no-default-features, MSRV, commitlint.

### 1.2 CI duplication and waste

| Issue | Impact |
|---|---|
| `ci.yml#test` (workspace nextest) **and** `test-matrix.yml#test-crates` (per-crate nextest) run the same tests | Double CPU; `Tests` job ~5-8 min on critical path |
| `pr-validation.yml#lint-metadata` runs twice on PRs (triggered by `synchronize` and `edited`) | Two duplicate `Validate PR metadata` checks |
| `cross-platform-sandbox-smoke` (ubuntu+macos+windows) runs on every PR | ~12 runner-min wasted on PRs touching unrelated crates |
| CodeQL `javascript-typescript` analysis on a Rust workspace | ~3-5 min per PR, no value (no JS/TS shipped) |
| `warm-cache` job blocks `test-crates` matrix | Adds ~3 min on cold cache; on warm cache neutral — **keep as-is** |
| Manual `release.yml` via `workflow_dispatch` | No release PR review, no auto-changelog on merge — replace with release-plz |

### 1.3 Bot noise (CodeRabbit `.coderabbit.yaml`)

| Setting | Current | Effect |
|---|---|---|
| `profile` | `assertive` | Maximum nit/style suggestions |
| `request_changes_workflow` | `true` | Blocks merge via required review |
| `sequence_diagrams` | `true` | Mermaid diagram in every PR walkthrough |
| `collapse_walkthrough` | `false` | Walkthrough always expanded |
| `auto_review.drafts` | `true` | Reviews drafts (immature code) |
| `auto_review.incremental_reviews` | `true` | New review on every push (rebase = N reviews) |
| `chat.auto_reply` | `true` | Bot replies to every thread message |
| `knowledge_base.linear` | `true` | Pulls Linear context — but Linear is **not used** by this project |

### 1.4 Copilot review without stop-list

`.github/copilot-instructions.md` describes the project but does not constrain Copilot's commenting behavior. Without a stop-list, Copilot appends generic "consider adding tests / docs / error handling" comments to every PR — duplicating what CodeRabbit catches with depth.

### 1.5 Cursor Bugbot

Status `NEUTRAL` on PRs #410 and #412 (no findings). Currently a required check. User works through Claude Code, not Cursor IDE, so the native value-add is limited.

### 1.6 Repository hygiene

- **Duplicate file:** `commitlint.config.cjs` and `commitlint.config.mjs` both present at repo root. One is dead.
- **No mise.toml / .tool-versions:** tool versions (taplo, typos, sccache, convco, etc.) are not pinned per repo.
- **Manual release flow:** `release.yml` requires `workflow_dispatch` with level input; modern Rust workspaces use `release-plz` with auto release-PR.

---

## 2. Goals & Non-goals

### Goals

1. **Zero "passed locally, failed CI" cases** for the six gaps in §1.1.
2. **PR wall-time ≤ 7 min** on typical PRs (touching 1-3 crates, no sandbox/runtime).
3. **CodeRabbit retains depth** (catches secret leaks, lock ordering, sandbox escapes) but stops emitting diagrams, draft reviews, incremental reviews, auto-replies, and merge blocks.
4. **Copilot review constrained by stop-list** — no generic "consider adding X" comments; instead targeted on layer violations, panic safety, silent error suppression.
5. **Local cargo iteration 30-50% faster** through `sccache` + `rust-lld`.
6. **Tool versions pinned per repo** via `mise.toml` so contributors (and you on a new machine) get identical environment.
7. **Replace manual release with release-plz** — auto release-PR on merge to main, auto-changelog, semver-aware bumps.

### Non-goals

- Replace Taskfile with `just` or `cargo-make`.
- Replace lefthook with `husky` or `pre-commit` (Python).
- Introduce Nix flake / shell.nix.
- Set up sccache distributed cache backend (S3).
- Add devcontainer.json / Codespaces.
- Add MIRI workflow (project minimizes unsafe).
- Add cargo-fuzz (no public parsers).
- Replace Dependabot with Renovate.
- Add codecov bot (would add a 4th bot; user pain = less noise, not more).

---

## 3. Approach: B (CI alignment + local speed) + selected modern stack

Selected from brainstorm options A/B/C — **B** confirmed by user. Augmented with five modern-stack additions:

- `release-plz` (replace manual cargo-release)
- `cargo-shear` (replace cargo-machete; user preference — auto-fix, fewer false positives)
- `mise.toml` (tool version manifest)
- `cargo-semver-checks` (advisory workflow toward 1.0)
- `convco` (Rust-native commitlint replacement for commit-msg hook)

---

## 4. Design — Section 1: Lefthook split

Goal: pre-commit ≤ 10 sec on changed files; pre-push = exact mirror of CI required jobs.

### New `lefthook.yml`

```yaml
skip_output: [meta, summary, success, skips, execution_info]

pre-commit:
  parallel: true
  commands:
    fmt-check:
      glob: "*.rs"
      run: cargo +nightly fmt --all -- --check
    clippy:
      glob: "*.rs"
      run: cargo clippy --workspace --all-targets -q -- -D warnings
    typos:
      run: typos
    taplo:
      glob: "**/*.toml"
      run: taplo fmt --check
    cargo-deny:
      glob: "{Cargo.toml,Cargo.lock,deny.toml}"
      run: cargo deny check

commit-msg:
  commands:
    convco:
      run: convco check --from {1}~1 --to {1}

pre-push:
  parallel: true
  commands:
    nextest:
      run: cargo nextest run --workspace --status-level fail --final-status-level fail
    doctests:
      run: cargo test --workspace --doc
    check-all-features:
      run: cargo check --workspace --all-features --all-targets
    check-no-default:
      run: |
        cargo check -p nebula-resilience --no-default-features
        cargo check -p nebula-log --no-default-features
        cargo check -p nebula-expression --no-default-features
    docs:
      env: { RUSTDOCFLAGS: "-D warnings" }
      run: cargo doc --workspace --no-deps --all-features -q
    msrv:
      # graceful skip if 1.94 toolchain not installed locally
      run: rustup toolchain list | grep -q 1.94 && cargo +1.94 check --workspace --all-targets || echo "skipped (install: rustup install 1.94)"
    shear:
      run: cargo shear
```

### Deltas vs current `lefthook.yml`

- ➕ `taplo fmt --check` (was CI-only)
- ➕ `commit-msg` hook via `convco` (was CI-only)
- ➕ `cargo test --doc` in pre-push (was CI-only)
- ➕ `cargo check --all-features` in pre-push (was CI-only)
- ➕ `cargo check --no-default-features` for 3 crates (was CI-only)
- ➕ MSRV check via `cargo +1.94 check` (graceful skip if toolchain absent)
- ➕ `cargo shear` (replaces missing local unused-deps check)
- ➕ `pre-push: parallel: true` (was sequential)
- ➕ `glob:` filters on pre-commit (only run when relevant files staged)

Pre-push will be 60-90 sec vs current 30 sec — but anything that would fail CI fails locally first.

---

## 5. Design — Section 2: Local cargo speed

Goal: agent-driven workflow (`cargo check`/`nextest` 5-10× per session) gets faster turnaround.

### 5.1 `sccache` (personal config — `~/.cargo/config.toml`)

```toml
[build]
rustc-wrapper = "sccache"
```

With env (set via `mise.toml` — see §11):

```
SCCACHE_DIR=C:\Users\vanya\.cache\sccache
SCCACHE_CACHE_SIZE=20G
```

Install: `cargo install sccache --locked` (or via mise). Documented in `docs/dev-setup.md`.

### 5.2 `rust-lld` linker — repo `.cargo/config.toml` (committed)

```toml
[target.x86_64-pc-windows-msvc]
linker = "rust-lld.exe"

[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]

[target.aarch64-apple-darwin]
rustflags = ["-C", "link-arg=-fuse-ld=lld"]
```

Windows: `rust-lld` ships with rustup since 1.81 — no install needed. Linux/Mac contributors install `mold`/`lld` (one command, documented).

### 5.3 Nextest `agent` profile — `.config/nextest.toml`

```toml
[profile.agent]
status-level = "fail"
final-status-level = "fail"
failure-output = "immediate"
success-output = "never"
fail-fast = true
slow-timeout = { period = "30s", terminate-after = 2 }
```

LLM agents invoke `cargo nextest run --profile agent -p X` for tight, fail-fast output (less context budget on parsing).

### 5.4 Skipped (vs initial plan B)

- `bacon` — user does not use editor-driven continuous feedback (vibe coding via LLM agents).
- `cargo-watch` — same reason.

---

## 6. Design — Section 3: CI dedup and paths-filtering

### 6.1 Remove duplicate `Tests` workspace job

Edit [`.github/workflows/ci.yml`](.github/workflows/ci.yml):

- **Delete** `test` job (workspace nextest + doctests).
- **Add** `doctests` job (lighter, parallel, off critical path):

```yaml
doctests:
  name: Doctests
  needs: [check]
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v6
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2
      with:
        shared-key: ci-doctests
        save-if: ${{ github.ref == 'refs/heads/main' }}
    - run: cargo test --workspace --doc
```

Per-crate matrix in `test-matrix.yml` already covers unit/integration tests — no loss of coverage.

### 6.2 Move cross-platform smoke to its own paths-filtered workflow

New file `.github/workflows/cross-platform.yml`:

```yaml
name: Cross-Platform Smoke

on:
  pull_request:
    paths:
      - "crates/sandbox/**"
      - "crates/runtime/**"
      - "crates/plugin-sdk/**"
      - ".github/workflows/cross-platform.yml"
  push:
    branches: [main]
  merge_group:

permissions: { contents: read }

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

jobs:
  smoke:
    name: Cross-platform smoke (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    steps:
      - uses: actions/checkout@v6
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with:
          shared-key: cross-platform
          save-if: ${{ github.ref == 'refs/heads/main' }}
      - run: |
          cargo test -p nebula-sandbox
          cargo test -p nebula-runtime
          cargo test -p nebula-plugin-sdk
```

Then **delete** `cross-platform-sandbox-smoke` job from [`.github/workflows/test-matrix.yml`](.github/workflows/test-matrix.yml).

### 6.3 Stop double `Validate PR metadata` runs

Edit [`.github/workflows/pr-validation.yml`](.github/workflows/pr-validation.yml):

```yaml
jobs:
  lint-metadata:
    name: Validate PR metadata
    runs-on: ubuntu-latest
    if: github.event.action != 'edited' || github.event.changes.title != null
    # ... rest unchanged
```

### 6.4 Remove CodeQL entirely

GitHub Settings → Code security → Code scanning → **Disable** default code scanning. Removes `Analyze (actions)` + `Analyze (javascript-typescript)` checks (~5-7 min per PR).

### 6.5 Keep `warm-cache`

Removing it would parallelize 12 cold-cache builds vs one shared warm build. On cold cache: worse. On warm cache: neutral. **No change.**

---

## 7. Design — Section 4: CodeRabbit dim

New `.coderabbit.yaml`:

```yaml
# yaml-language-server: $schema=https://coderabbit.ai/integrations/schema.v2.json
language: "en-US"
tone_instructions: >-
  Deep, high-signal reviews for a Rust workflow engine. Prioritize correctness,
  safety (data loss, concurrency races, lock ordering), API contract breakage,
  silent fallbacks, and missing regression tests. Be terse. Skip style nits.

reviews:
  profile: "chill"                  # was: assertive
  request_changes_workflow: false   # was: true
  high_level_summary: true
  review_status: true
  collapse_walkthrough: true        # was: false
  changed_files_summary: true
  sequence_diagrams: false          # was: true

  auto_review:
    enabled: true
    drafts: false                   # was: true
    incremental_reviews: false      # was: true
    base_branches:
      - "main"
      - "release/.*"
    labels:
      - "!skip-coderabbit"
    description_keyword: "@coderabbit review"

  path_filters:
    - "!**/.claude/worktrees/**"
    - "!docs/plans/archive/**"
    - "!docs/superpowers/**"        # NEW
    - "!target/**"
    - "!**/*.snap"                  # NEW
    - "!**/Cargo.lock"              # NEW

  path_instructions:
    - path: "crates/credential/**"
      instructions: |
        Critical: check for secret leaks in logs/debug/error text.
        Verify zeroization on drop. Check encryption boundaries.
    - path: "crates/sandbox/**"
      instructions: |
        Critical: check for sandbox escape vectors, resource limits, IPC safety.
    - path: "crates/{engine,runtime,execution}/**"
      instructions: |
        Critical: check lock ordering, race conditions, version-bump invariants
        on state mutations. Flag direct ns.state = X without transition_node().
    - path: "**/tests/**"
      instructions: |
        Skip style/naming nits. Focus only on missing edge cases.

chat:
  auto_reply: false                 # was: true

knowledge_base:
  opt_out: false
  learnings: { scope: "auto" }
  issues: { enabled: true }
  pull_requests: { enabled: true }
  linear: { enabled: false }        # was: true (not used)
```

On-demand test generation (`@coderabbitai generate unit tests`) is preserved — it is a command, not auto, and was used productively in PR #408.

---

## 8. Design — Section 5: Copilot instructions rewrite

New `.github/copilot-instructions.md` structure (full content in implementation plan):

1. **Project context** (concise — 4 lines)
2. **What to flag** — critical (layer violations, panic in lib code, silent errors, direct state mutation per past incident #255, missing Send+Sync, untrusted Duration::from_secs_f64) and useful (logical bugs, missing edge tests with named cases, public API without doc).
3. **What NOT to flag (stop-list)** — style/formatting, naming preferences, generic suggestions, comments on private code, README updates, things CodeRabbit owns (credential/sandbox/engine).
4. **Project-specific patterns** — metrics path, error conventions, no Value crate.
5. **Test conventions** — nextest, doctests, real memory backends not mocks.
6. **Avoid suggesting** — unsafe (without SAFETY), Rc in async, heavy mocks, alternate metrics stacks, separate value crate, removed `.project/*` conventions.

The stop-list is the single most impactful change.

---

## 9. Design — Section 6: Cursor Bugbot, Dependabot, cleanup

### 9.1 Cursor Bugbot

GitHub Settings → Branches → main protection → **uncheck** `Cursor Bugbot` from required checks. Keep it as advisory. Re-evaluate in 1 month: if zero real findings, disable from Cursor settings.

### 9.2 Dependabot — minor improvements

Edit [`.github/dependabot.yml`](.github/dependabot.yml) — add to each `updates` block:

```yaml
open-pull-requests-limit: 5
labels:
  - "dependencies"
  - "skip-coderabbit"
```

`skip-coderabbit` matches `auto_review.labels: "!skip-coderabbit"` — version bumps don't need bot review.

### 9.3 Repository hygiene cleanup

- **Delete** `commitlint.config.cjs` (keep `.mjs` — modern format).
- Verify CI still uses commitlint correctly after deletion.

---

## 10. Design — Section 7: release-plz (replace manual release)

### 10.1 New `release-plz.toml` at repo root

```toml
[workspace]
allow_dirty = false
changelog_update = true
git_release_enable = true
publish = false             # alpha: do not publish to crates.io yet; flip to true at 1.0
semver_check = true         # uses cargo-semver-checks
pr_branch_prefix = "release-plz/"
pr_labels = ["release", "skip-coderabbit"]
```

### 10.2 New `.github/workflows/release-plz.yml`

```yaml
name: Release-plz

on:
  push:
    branches: [main]

permissions:
  contents: write
  pull-requests: write

concurrency:
  group: release-plz-${{ github.ref }}

jobs:
  release-plz:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
        with: { fetch-depth: 0 }
      - uses: dtolnay/rust-toolchain@stable
      - uses: MarcoIeni/release-plz-action@v0.5
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          # CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}  # uncomment when publish=true
```

### 10.3 Delete old release flow

- **Delete** [`.github/workflows/release.yml`](.github/workflows/release.yml).
- Existing `cliff.toml` (git-cliff config) is reused by release-plz for changelog.

---

## 11. Design — Section 8: mise.toml (tool versions)

New `mise.toml` at repo root:

```toml
[tools]
rust = "1.94"
taplo = "0.9.3"
typos = "1.27"
"cargo:cargo-nextest" = "0.9"
"cargo:cargo-deny" = "0.16"
"cargo:cargo-shear" = "1.1"
"cargo:cargo-semver-checks" = "0.40"
"cargo:cargo-udeps" = "0.1"
"cargo:cargo-audit" = "0.21"
"cargo:cargo-release" = "0.25"
"cargo:sccache" = "0.8"
"cargo:convco" = "0.6"
"cargo:release-plz" = "latest"

[env]
RUSTC_WRAPPER = "sccache"
SCCACHE_CACHE_SIZE = "20G"
CARGO_TERM_COLOR = "always"
```

Versions pinned to current stable; updated via dependabot-style mise updates. Contributors install all tooling with one command: `mise install`.

`docs/dev-setup.md` documents the install path:
1. Install mise: `winget install jdx.mise` (Windows), `curl https://mise.run | sh` (Linux/Mac).
2. `cd nebula && mise install` — installs everything from `mise.toml`.
3. `mise run dev` (optional shortcuts) or just use `cargo`/`task` as normal.

---

## 12. Design — Section 9: cargo-semver-checks workflow (advisory)

New `.github/workflows/semver-checks.yml`:

```yaml
name: SemVer Checks

on:
  pull_request:
    paths:
      - "crates/**/Cargo.toml"
      - "crates/**/src/**"
  workflow_dispatch:

permissions: { contents: read, pull-requests: write }

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

jobs:
  semver:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: dtolnay/rust-toolchain@stable
      - uses: obi1kenobi/cargo-semver-checks-action@v2
        with:
          rust-toolchain: stable
          # advisory only during alpha — does not block merge
          continue-on-error: true
```

Becomes blocking when project leaves alpha (manual edit when ready for 1.0).

---

## 13. Acceptance criteria

A change is complete when **all** the following hold:

### Local

1. `git commit` on a Rust-only change runs fmt+clippy+typos+taplo+cargo-deny in parallel and completes in ≤ 10 sec on warm cache.
2. `git commit` with a non-conventional message is rejected by `convco` before the commit is created.
3. `git push` runs nextest + doctests + check-all-features + check-no-default + doc + MSRV + shear in parallel; fails the push if any fails; completes in ≤ 90 sec on warm cache.
4. `cargo check` and `cargo build` are 30-50% faster than baseline (measure: clean checkout, then bench three iterations of `cargo check --workspace`).
5. `mise install` installs all tooling from `mise.toml` on a fresh machine.

### CI

6. PR touching only one crate (no sandbox/runtime) shows ~20 status checks (down from 34) and completes in ≤ 7 min.
7. PR touching `crates/sandbox/**` triggers `cross-platform.yml` (3 OS smoke).
8. PR with title edit does not generate a duplicate `Validate PR metadata` check.
9. CodeQL `Analyze (actions)` and `Analyze (javascript-typescript)` no longer appear in PR checks.
10. `Tests` workspace job is gone; `Doctests` job present and green; per-crate `Test nebula-X` matrix unchanged.

### Bots

11. CodeRabbit review on a typical PR contains: high-level summary, walkthrough (collapsed by default), per-file changes — but **no** sequence diagram, **no** incremental re-review on subsequent pushes, **no** auto-reply in chat.
12. CodeRabbit does not create a "requested changes" review that blocks merge.
13. CodeRabbit on a draft PR makes no review (only when ready-for-review).
14. Copilot review on a typical PR does not include "consider adding tests/docs/error handling" without a named case.
15. Cursor Bugbot is not in branch protection required checks.

### Release flow

16. After merging a feature PR to main, release-plz opens (or updates) a release PR within 5 min with version bumps and changelog entries.
17. Merging the release PR creates GitHub release + (when `publish: true`) publishes to crates.io.
18. Old `release.yml` workflow is deleted.

### Hygiene

19. Only one of `commitlint.config.cjs` / `commitlint.config.mjs` exists in repo (the `.mjs` one).
20. Dependabot PRs have `dependencies` + `skip-coderabbit` labels and CodeRabbit does not review them.

---

## 14. Out of scope (explicit YAGNI)

- sccache distributed cache (S3 backend)
- Self-hosted GitHub runners
- Replacing Taskfile with `just` / `cargo-make`
- Composite actions for CI step deduplication
- `cargo-hack` for feature matrix testing
- Migrating from `dtolnay/rust-toolchain` to `actions-rust-lang/setup-rust-toolchain`
- husky / make / pre-commit (Python)
- flake.nix / shell.nix
- devcontainer.json / Codespaces
- MIRI workflow
- cargo-fuzz
- Renovate (replace Dependabot)
- cargo-tarpaulin (replace by cargo-llvm-cov if/when needed)
- codecov bot integration
- rust-toolchain.toml (conflicts with CI MSRV vs stable matrix; mise covers it)

---

## 15. Open questions (none blocking — all resolved during brainstorm)

All design decisions confirmed by user during the 2026-04-16 brainstorm. Remaining items to resolve **during implementation**:

- Q1: ~~Does `apps/` contain JS/TS that warrants CodeQL coverage?~~ **RESOLVED:** `apps/desktop` is Tauri+React+TS (with Biome linting), but user decision is to **disable CodeQL entirely**. If a security incident in desktop frontend later requires it, add a focused `apps/desktop` CodeQL workflow then — not now.
- Q2: Exact `mise.toml` versions — `latest` for some tools (release-plz) vs pinned. **Resolution path:** pin all to current stable at implementation time; let mise update flow bump them.

---

## 16. Implementation grouping (preview for writing-plans)

The 20 changes naturally cluster into 4-5 PRs:

1. **PR-1: lefthook + local speed** (changes 1-7 in summary list — lefthook split, .cargo/config.toml, .config/nextest.toml, mise.toml, dev-setup.md, sccache, lld). Pure additions; no risk to existing CI.
2. **PR-2: CI dedup** (changes 8-12 — ci.yml, cross-platform.yml split, pr-validation condition, CodeQL disable, test-matrix.yml prune). Touches workflow files; merge under `automerge`-style monitoring.
3. **PR-3: release-plz migration** (changes 13-15, plus 18 — release-plz.toml, release-plz.yml, delete release.yml, dependabot labels, semver-checks.yml). Replaces release flow — coordinate timing (no in-flight release).
4. **PR-4: Bot config + hygiene** (changes 16-17, 19-20 — coderabbit.yaml, copilot-instructions.md, delete commitlint.cjs, Cursor Bugbot setting). Lowest risk, immediate noise reduction.

Order: **PR-1 → PR-4 → PR-2 → PR-3** (start with safest local changes; finish with release flow which is highest blast radius).

---

## 17. Definition of done

- All 20 acceptance criteria in §13 verified.
- `docs/dev-setup.md` present and accurate.
- This spec referenced from a top-level `docs/` index (or linked from CLAUDE.md as the "dev environment" canonical doc).
- One representative PR through the new flow demonstrating: ≤ 7 min wall-time, ≤ 20 checks, no `request_changes` block from CodeRabbit, no duplicate `Validate PR metadata`.
- Memory updates: add an entry to `.claude/.../memory/` documenting the new `pre-push` mirror-CI guarantee so future sessions don't re-introduce the divergence.
