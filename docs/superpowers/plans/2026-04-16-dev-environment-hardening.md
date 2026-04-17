# Dev Environment & CI Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate "passed locally, failed CI" cases, halve PR wall-time, dim bot noise without losing CodeRabbit's depth, and add modern Rust 2026 stack additions (release-plz, cargo-shear, mise, cargo-semver-checks, convco).

**Architecture:** Four sequential phases mapping 1:1 to four PRs. Phase 1 = lefthook + local toolchain (zero-risk additions). Phase 2 = bot config dim + repo hygiene (config-only). Phase 3 = CI workflow dedup + paths-filtering (touches required checks). Phase 4 = release-plz migration replacing manual `release.yml` (highest blast radius, last). Each phase produces a working, mergeable PR.

**Tech Stack:** lefthook (git hooks), convco (commit-msg), sccache + rust-lld (cargo speed), nextest (test runner), mise (tool versions), CodeRabbit + Copilot review (dim configs), release-plz + cargo-semver-checks (release flow), GitHub Actions.

**Source spec:** [`docs/superpowers/specs/2026-04-16-dev-environment-design.md`](../specs/2026-04-16-dev-environment-design.md)

---

## File Structure (whole plan at a glance)

### Files to Create

| Path | Purpose |
|---|---|
| `.cargo/config.toml` | Repo-wide linker config (rust-lld on Windows, mold/lld on Linux/Mac) |
| `mise.toml` | Tool version manifest (Rust toolchain, taplo, typos, all cargo-tools) |
| `docs/dev-setup.md` | Contributor onboarding (mise, sccache, convco install) |
| `release-plz.toml` | release-plz workspace config |
| `.github/workflows/cross-platform.yml` | Paths-filtered cross-platform sandbox smoke |
| `.github/workflows/release-plz.yml` | Auto release-PR workflow on push to main |
| `.github/workflows/semver-checks.yml` | Advisory cargo-semver-checks on PR |

### Files to Modify

| Path | Change |
|---|---|
| `lefthook.yml` | Split pre-commit (fast, glob-filtered) / pre-push (full CI mirror) / commit-msg (convco) |
| `.config/nextest.toml` | Add `[profile.agent]` for LLM-friendly output |
| `.coderabbit.yaml` | profile=chill, kill diagrams/drafts/incremental/auto_reply/linear, add path_instructions |
| `.github/copilot-instructions.md` | Stop-list, division of labor with CodeRabbit, past incidents |
| `.github/dependabot.yml` | Add labels [dependencies, skip-coderabbit] + open-pull-requests-limit |
| `.github/workflows/ci.yml` | Remove `test` job, add `doctests` job |
| `.github/workflows/test-matrix.yml` | Remove `cross-platform-sandbox-smoke` job (moved to its own workflow) |
| `.github/workflows/pr-validation.yml` | Add `if:` condition skipping duplicate runs on body-only edits |

### Files to Delete

| Path | Reason |
|---|---|
| `commitlint.config.cjs` | Duplicate of `.mjs`; `.mjs` is the canonical one |
| `.github/workflows/release.yml` | Replaced by release-plz |

### External (GitHub UI) Actions

- **Phase 2:** Settings → Branches → main → uncheck `Cursor Bugbot` from required checks.
- **Phase 3:** Settings → Code security → Code scanning → Disable default code scanning (removes CodeQL `Analyze (actions)` and `Analyze (javascript-typescript)`).

---

## Phase 1: Lefthook split + local toolchain

**PR title:** `chore(dev-env): mirror CI in pre-push, add sccache/lld/mise toolchain`
**Risk:** zero — pure additions, no CI changes, no production code.
**Acceptance from spec §13:** items 1, 2, 3, 4, 5.

### Task 1.1: Create `.cargo/config.toml` with linker config

**Files:**
- Create: `.cargo/config.toml`

- [ ] **Step 1: Create the file**

```toml
# .cargo/config.toml
# Repo-wide linker configuration.
# rust-lld ships with rustup since Rust 1.81 — no install needed on Windows.
# Linux/Mac contributors need `mold` (Linux) or `lld` (Mac) installed locally.

[target.x86_64-pc-windows-msvc]
linker = "rust-lld.exe"

[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]

[target.aarch64-apple-darwin]
rustflags = ["-C", "link-arg=-fuse-ld=lld"]

[target.x86_64-apple-darwin]
rustflags = ["-C", "link-arg=-fuse-ld=lld"]
```

- [ ] **Step 2: Verify a clean build still works**

Run: `cargo check -p nebula-core`
Expected: success (PASS). If linker error on Linux/Mac → contributor needs `mold`/`lld`; on Windows should just work with rust-lld.

- [ ] **Step 3: Stage**

```bash
git add .cargo/config.toml
```

### Task 1.2: Add `[profile.agent]` to nextest config

**Files:**
- Modify: `.config/nextest.toml` (append to end)

- [ ] **Step 1: Append the agent profile**

Open `.config/nextest.toml` and append at the end (file currently ends after `[profile.ci.junit]` block):

```toml

[profile.agent]
# Optimized for LLM-driven invocation: tight, fail-fast, no spinner/emoji.
status-level = "fail"
final-status-level = "fail"
failure-output = "immediate"
success-output = "never"
fail-fast = true
slow-timeout = { period = "30s", terminate-after = 2 }
```

- [ ] **Step 2: Verify the profile parses**

Run: `cargo nextest list --profile agent -p nebula-core`
Expected: success — lists tests, no config error.

- [ ] **Step 3: Stage**

```bash
git add .config/nextest.toml
```

### Task 1.3: Rewrite `lefthook.yml` with full CI mirror

**Files:**
- Modify: `lefthook.yml` (full rewrite)

- [ ] **Step 1: Replace file content**

```yaml
# lefthook.yml
# Local git hooks. Pre-commit = fast on changed files. Pre-push = full mirror of CI required jobs.
# All "missing" checks (taplo, doctests, --all-features, --no-default-features, MSRV) are now run
# locally before push, so CI required jobs cannot fail without lefthook also failing.

skip_output:
  - meta
  - summary
  - success
  - skips
  - execution_info

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
      env:
        RUSTDOCFLAGS: "-D warnings"
      run: cargo doc --workspace --no-deps --all-features -q
    msrv:
      # Graceful skip if 1.94 toolchain not installed locally.
      # Install with: rustup install 1.94
      run: |
        if rustup toolchain list | grep -q '^1.94'; then
          cargo +1.94 check --workspace --all-targets
        else
          echo "MSRV skipped: rustup install 1.94 to enable"
        fi
    shear:
      run: cargo shear

# Note: 'cargo udeps' is NOT in pre-push. It requires nightly + is slow. Kept in weekly CI workflow.
```

- [ ] **Step 2: Reinstall lefthook hooks (lefthook detects new commands)**

Run: `lefthook install`
Expected: `sync hooks: ✔ (commit-msg, pre-commit, pre-push)`.

- [ ] **Step 3: Verify pre-commit runs cleanly on no-op**

Run: `lefthook run pre-commit`
Expected: all 5 commands run in parallel and pass (or skip if no matching files staged). No errors.

- [ ] **Step 4: Verify commit-msg accepts conventional**

Test: write a file `/tmp/msg.txt` with content `chore: test commit-msg hook` and run `lefthook run commit-msg /tmp/msg.txt`.
Expected: convco accepts. (Skip if `convco` not yet installed — Task 1.4 installs it via mise; manual install otherwise: `cargo install convco`.)

- [ ] **Step 5: Verify pre-push runs (will be slow first time, ~60-90s)**

Run: `lefthook run pre-push`
Expected: nextest + doctests + check-all-features + check-no-default + docs + msrv (or skip msrv) + shear all run in parallel and pass.
If `cargo shear` not installed: `cargo install cargo-shear --locked` (Task 1.4 adds it to mise).

- [ ] **Step 6: Stage**

```bash
git add lefthook.yml
```

### Task 1.4: Create `mise.toml` with pinned tool versions

**Files:**
- Create: `mise.toml`

- [ ] **Step 1: Create the file**

```toml
# mise.toml
# Tool version manifest for the Nebula workspace.
# Run `mise install` after cloning to install all tools at the pinned versions.
# Install mise itself: `winget install jdx.mise` (Windows), `curl https://mise.run | sh` (Linux/Mac).

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
# Enable sccache by default for everyone
RUSTC_WRAPPER = "sccache"
# 20 GB cache (default is 10 GB, project is large with many features)
SCCACHE_CACHE_SIZE = "20G"
# Cargo always-color is fine in interactive; set to "never" only in agent scripts
CARGO_TERM_COLOR = "always"
```

- [ ] **Step 2: Verify mise can parse it (if mise installed)**

Run: `mise ls --current` (after `mise install` if first time).
Expected: lists rust=1.94, taplo, typos, etc.
Skip if mise not yet installed locally — this is documented as a contributor install step in `docs/dev-setup.md` (Task 1.5).

- [ ] **Step 3: Stage**

```bash
git add mise.toml
```

### Task 1.5: Write `docs/dev-setup.md` for contributor onboarding

**Files:**
- Create: `docs/dev-setup.md`

- [ ] **Step 1: Create the file**

```markdown
# Developer Setup

This document describes the local toolchain expected by Nebula's lefthook hooks
and CI mirror. All tools listed here are managed by [mise](https://mise.jdx.dev/).

## Quick start

1. Install mise:
   - Windows: `winget install jdx.mise`
   - Linux/Mac: `curl https://mise.run | sh`
2. From the repo root: `mise install` — installs everything pinned in `mise.toml`.
3. Install lefthook hooks: `lefthook install`.
4. Optional but recommended: configure sccache cache directory.

That is the entire setup.

## What `mise install` provides

| Tool | Purpose |
|---|---|
| `rust` (1.94) | Project MSRV toolchain |
| `taplo` | TOML formatter; checked in pre-commit |
| `typos` | Spell-check; checked in pre-commit |
| `cargo-nextest` | Test runner; used in pre-push and CI |
| `cargo-deny` | License + advisory check; pre-commit |
| `cargo-shear` | Unused dependency detection; pre-push |
| `cargo-semver-checks` | SemVer compliance check; advisory CI |
| `cargo-udeps` | Unused deps (nightly); weekly CI cron |
| `cargo-audit` | Vulnerability scan; weekly CI cron |
| `cargo-release` | Workspace version management (used by release-plz under the hood) |
| `sccache` | Compilation cache; speeds up cargo across branches |
| `convco` | Conventional-commit check; commit-msg hook |
| `release-plz` | Auto release-PR workflow; runs in CI |

## sccache cache directory

`mise.toml` sets `RUSTC_WRAPPER=sccache` and `SCCACHE_CACHE_SIZE=20G`. The cache directory
defaults to `~/.cache/sccache` (Linux/Mac) or `%LOCALAPPDATA%\sccache` (Windows). To override,
set `SCCACHE_DIR` in your shell:

```bash
# bash/zsh
export SCCACHE_DIR=$HOME/.cache/sccache

# PowerShell
$env:SCCACHE_DIR = "$env:LOCALAPPDATA\sccache"
```

Verify sccache is active: `sccache --show-stats`. The "Cache hits" counter increases
when cargo reuses cached artifacts.

## Linker

The repo's `.cargo/config.toml` configures fast linkers:

| Platform | Linker | Install |
|---|---|---|
| Windows MSVC | `rust-lld.exe` | Bundled with rustup since 1.81 — no action |
| Linux GNU | `mold` (via clang) | `apt install mold clang` or [github.com/rui314/mold](https://github.com/rui314/mold) |
| macOS | `lld` | `brew install llvm` (provides lld) |

If you see "linker not found" errors after `mise install`, install the linker for your
platform per the table above.

## Lefthook hooks

After `lefthook install`, three hooks run on git events:

- **pre-commit** (≤10s): fmt-check, clippy, typos, taplo, cargo-deny on changed files.
- **commit-msg** (instant): convco validates the commit message follows conventional commits.
- **pre-push** (≤90s parallel): nextest, doctests, --all-features check, --no-default-features check, docs, MSRV check, cargo-shear. This is a complete mirror of CI's required jobs.

If pre-push fails, fix locally before pushing. CI cannot fail without pre-push also failing
on the same code.

To bypass a hook in an emergency (will be caught by CI): `git commit --no-verify` or
`git push --no-verify`. Avoid this — it defeats the purpose.

## LLM agent profile

For LLM-driven workflows (Claude Code, Cursor, etc.), use the `agent` nextest profile:

```bash
cargo nextest run --profile agent -p <crate>
```

This profile is fail-fast, suppresses success output, and limits slow-test reporting —
optimized for tight LLM context.

For agent scripts that parse cargo output, set `CARGO_TERM_COLOR=never` to strip ANSI
escape codes:

```bash
CARGO_TERM_COLOR=never cargo check -p <crate>
```

## Troubleshooting

**"convco: command not found" in commit-msg hook.** Run `mise install` (installs convco), or
manually: `cargo install convco`.

**"cargo +1.94: toolchain not installed" in pre-push.** The MSRV check skips gracefully if
1.94 is missing. To enable it locally: `rustup install 1.94`.

**Pre-push too slow.** Expected ~60-90s on warm cache (sccache + workspace cache). On cold
cache the first run is several minutes. If consistently >2min on warm cache, check:
`sccache --show-stats` for cache hit rate.

**lefthook does not run.** Re-install hooks: `lefthook install`. Verify hooks exist:
`ls .git/hooks/pre-commit .git/hooks/commit-msg .git/hooks/pre-push`.
```

- [ ] **Step 2: Stage**

```bash
git add docs/dev-setup.md
```

### Task 1.6: Verify the full Phase 1 setup end-to-end

- [ ] **Step 1: Verify `mise install` works (if mise is installed)**

Run: `mise install`
Expected: installs all tools listed in `mise.toml`. Takes 1-3 minutes first time.
If mise is not installed: skip this step; install will be done by contributor on first clone.

- [ ] **Step 2: Verify lefthook pre-commit on a touched file**

Touch any `.rs` file (e.g. add and remove a blank line in `crates/core/src/lib.rs`), stage it, then:

Run: `lefthook run pre-commit`
Expected: fmt-check + clippy + typos run on the changed Rust file; taplo skipped (no .toml staged); cargo-deny skipped. All pass.

- [ ] **Step 3: Verify lefthook pre-push completes**

Run: `lefthook run pre-push`
Expected: all 7 commands (nextest, doctests, check-all-features, check-no-default, docs, msrv, shear) run in parallel and pass. Wall time ≤ 90s on warm cache.

- [ ] **Step 4: Verify commit-msg blocks a bad commit**

Try: `git commit --allow-empty -m "bad message no convention"` (do NOT keep the commit).
Expected: convco rejects with non-zero exit; commit not created.
Then: `git commit --allow-empty -m "chore: verify convco accepts good message"` and immediately `git reset HEAD~1` to discard.

### Task 1.7: Commit Phase 1 and open PR

- [ ] **Step 1: Confirm staged files**

Run: `git status`
Expected staged files: `.cargo/config.toml`, `.config/nextest.toml`, `lefthook.yml`, `mise.toml`, `docs/dev-setup.md`.

- [ ] **Step 2: Commit**

```bash
git commit -m "chore(dev-env): mirror CI in pre-push, add sccache/lld/mise toolchain

Pre-push now runs nextest + doctests + --all-features check +
--no-default-features check + cargo doc -D warnings + MSRV (1.94)
+ cargo-shear, all in parallel. Pre-commit adds taplo (was CI-only).
commit-msg hook added via convco for conventional commits.

.cargo/config.toml configures rust-lld on Windows (bundled with rustup)
and mold/lld on Linux/Mac for 3-5x faster linking.

mise.toml pins all tooling versions (rust 1.94, taplo, typos, all
cargo-tools, sccache, convco, release-plz) — contributors run
\`mise install\` to provision the full toolchain.

.config/nextest.toml gains [profile.agent] for LLM-friendly tight
output during agent-driven workflows.

docs/dev-setup.md documents the contributor onboarding path.

Refs: docs/superpowers/specs/2026-04-16-dev-environment-design.md §4-§5, §11

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 3: Push and open PR**

```bash
git push -u origin <branch-name>
gh pr create --title "chore(dev-env): mirror CI in pre-push, add sccache/lld/mise toolchain" --body "$(cat <<'EOF'
## Summary
- Pre-push hook now mirrors CI required jobs (taplo, doctests, --all-features, --no-default-features, MSRV, docs) — eliminates "passed locally, failed CI" cases for these checks.
- Pre-commit gains taplo + glob-filtering for fast iteration (≤10s).
- Adds commit-msg hook via convco (Rust-native conventional-commit checker).
- Adds .cargo/config.toml with rust-lld (Windows) / mold / lld linkers — 3-5× faster linking.
- Adds mise.toml manifest pinning all tooling versions.
- Adds [profile.agent] to nextest for LLM-friendly tight output.
- Adds docs/dev-setup.md for contributor onboarding.

Phase 1 of [dev environment hardening plan](docs/superpowers/plans/2026-04-16-dev-environment-hardening.md). Phases 2-4 (bot config, CI dedup, release-plz) are separate PRs.

## Test plan
- [ ] `mise install` provisions all listed tools without error
- [ ] `lefthook run pre-commit` runs fmt+clippy+typos+taplo+cargo-deny in parallel
- [ ] `lefthook run pre-push` runs all 7 checks in parallel and completes in ≤90s on warm cache
- [ ] `git commit -m "bad msg"` is rejected by convco
- [ ] `cargo check -p nebula-core` builds successfully with new linker config

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 4: Wait for CI to pass on the PR**

Run: `gh pr checks <pr-number> --watch`
Expected: all checks pass. If a check fails, fix and amend (or new commit) and re-push.

---

## Phase 2: Bot config dim + repo hygiene

**PR title:** `chore(bots): dim CodeRabbit, scope Copilot, prune duplicates`
**Risk:** low — config-only changes, immediate noise reduction.
**Acceptance from spec §13:** items 11, 12, 13, 14, 15, 19, 20.

### Task 2.1: Rewrite `.coderabbit.yaml` with dim profile

**Files:**
- Modify: `.coderabbit.yaml` (full rewrite)

- [ ] **Step 1: Replace file content**

```yaml
# yaml-language-server: $schema=https://coderabbit.ai/integrations/schema.v2.json
language: "en-US"
tone_instructions: >-
  Deep, high-signal reviews for a Rust workflow engine. Prioritize correctness,
  safety (data loss, concurrency races, lock ordering), API contract breakage,
  silent fallbacks, and missing regression tests. Be terse. Skip style nits.

reviews:
  profile: "chill"                  # was: assertive — only serious findings
  request_changes_workflow: false   # was: true — does not block merge
  high_level_summary: true
  review_status: true
  collapse_walkthrough: true        # was: false — walkthrough collapsed by default
  changed_files_summary: true
  sequence_diagrams: false          # was: true — drop mermaid noise

  auto_review:
    enabled: true
    drafts: false                   # was: true — do not review drafts
    incremental_reviews: false      # was: true — review final PR, not each push
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
  learnings:
    scope: "auto"
  issues:
    enabled: true
  pull_requests:
    enabled: true
  linear:
    enabled: false                  # was: true (Linear not used)
```

- [ ] **Step 2: Verify YAML is valid**

Run: `taplo fmt --check .coderabbit.yaml || true` (taplo handles YAML too) — or use any YAML linter.
Expected: valid YAML, no syntax errors.

- [ ] **Step 3: Stage**

```bash
git add .coderabbit.yaml
```

### Task 2.2: Rewrite `.github/copilot-instructions.md` with stop-list

**Files:**
- Modify: `.github/copilot-instructions.md` (full rewrite)

- [ ] **Step 1: Replace file content**

```markdown
# Copilot Instructions for Nebula

## Project Context
Modular type-safe Rust workflow engine. Edition 2024, MSRV 1.94, alpha stage.
Architecture: Core → Business → Exec → API (one-way deps, no upward).
Universal data type: serde_json::Value.
Error handling: thiserror in libs, anyhow in binaries.

## What to Flag in Reviews

### Critical (always comment)

1. **Layer violations** — `crates/core/*` importing from `crates/engine/*` etc.
   Check Cargo.toml dependencies against the layer hierarchy:
   `core < business (credential/resource/action/plugin) < exec (engine/runtime/storage/sandbox/sdk/plugin-sdk) < api`.
2. **Panic in library code** — `unwrap()`, `expect()`, `panic!()`, indexing without bounds check, `unreachable!()` outside exhaustive match.
   Exception: `#[cfg(test)]` and binary crates allowed.
3. **Silent error suppression** — `let _ = result;` on `Result`, `.ok()` discarding meaningful errors, `.unwrap_or_default()` on fallible IO/parse.
4. **Direct state mutation in execution/engine** — `node_state.state = X` without going through `transition_node()`. Loses version bump. See past incident #255.
5. **Missing `Send + Sync`** on async types in runtime/engine paths.
6. **Untrusted Duration** — `Duration::from_secs_f64(user_input)` without clamping (NaN/inf/negative panics).

### Useful (comment if confident)

7. **Logical bugs in new code** — off-by-one, wrong comparison operator, swapped args.
8. **Missing edge case tests** — but only when adding new public API or branching logic. Be specific: name the case.
9. **Public API without doc comment** — only on `pub fn`/`pub struct`, only when fn name doesn't fully describe behavior or error contract.

## What NOT to Flag (stop-list)

DO NOT comment on:

- **Style / formatting** — rustfmt + nightly handles this. Never suggest reformatting.
- **Naming preferences** — no "consider renaming X to Y" unless name is actively misleading.
- **Generic suggestions** — no "consider adding logging", "consider error handling", "consider tests" without naming a specific case.
- **Missing comments on private code** — internal fns don't need doc comments.
- **README / CHANGELOG updates** — separate process.
- **Suggesting `#[derive(Debug)]`** etc — assume it's there for a reason if absent.
- **Test file nits** — naming, ordering, helper extraction — none of it.
- **Things CodeRabbit will catch** — secret handling in `crates/credential/**`, lock ordering in `crates/{engine,runtime,execution}/**`, sandbox escapes in `crates/sandbox/**`. Skip these — CodeRabbit owns them.
- **MSRV checks on syntax** — CI MSRV job catches it; don't comment on use of recent features unless there's actual breakage.

## Project-Specific Patterns

### Metrics

Single path: `nebula-telemetry::MetricsRegistry` → `nebula-metrics` (Prometheus export).
Domain crates consume via DI: `Option<Arc<MetricsRegistry>>`.
Flag PRs that introduce alternate metrics stacks.

### Errors

- Library crates: `thiserror` enums with `#[from]` for layer transitions.
- Binary crates: `anyhow::Result` only at the boundary.
- Errors must include actionable context — don't comment "add context" generically; only comment when error swallows the cause.

### No "Value" crate

Universal interchange = `serde_json::Value`. Flag PRs introducing `enum Value { ... }` or wrapper types.

## Test Conventions

- `cargo nextest run` for unit tests, `cargo test --doc` for doctests.
- Memory backends are real (concrete impls), not mocks. Don't suggest replacing them with mockall.
- Integration tests live in `tests/`, not `src/`.

## Avoid Suggesting

- `unsafe` blocks (require explicit `SAFETY:` comment + justification — only flag if missing on existing unsafe).
- `Rc<T>` in async paths (use `Arc<T>`).
- Heavy mocks instead of memory backends.
- Alternate metrics stacks.
- A separate "value" crate.
- Bringing back removed `.project/*` conventions.
```

- [ ] **Step 2: Stage**

```bash
git add .github/copilot-instructions.md
```

### Task 2.3: Delete duplicate `commitlint.config.cjs`

**Files:**
- Delete: `commitlint.config.cjs`

- [ ] **Step 1: Verify both files exist and `.mjs` is the keeper**

Run: `ls commitlint.config.* | sort`
Expected: lists both `commitlint.config.cjs` and `commitlint.config.mjs`. The `.mjs` file is the canonical one (modern ESM, includes `body-max-line-length: [0]` rule).

- [ ] **Step 2: Delete the .cjs duplicate**

Run: `git rm commitlint.config.cjs`

- [ ] **Step 3: Verify CI workflow still references commitlint correctly**

Read [`.github/workflows/pr-validation.yml`](.github/workflows/pr-validation.yml) lines 25-28: it uses `wagoid/commitlint-github-action@v6` which auto-detects `commitlint.config.mjs`. No workflow change needed.

- [ ] **Step 4: Verify commitlint locally finds the .mjs config**

Run: `npx --no-install commitlint --print-config 2>&1 | head -5`
Expected: commitlint loads `commitlint.config.mjs` (look for `extends: ['@commitlint/config-conventional']` in output).
Skip if Node/npm not installed locally — CI will catch issues.

### Task 2.4: Update `.github/dependabot.yml` with skip labels

**Files:**
- Modify: `.github/dependabot.yml`

- [ ] **Step 1: Add `open-pull-requests-limit` and `labels` to both update entries**

Replace the cargo block (lines 5-20) with:

```yaml
  # Cargo workspace (all crates)
  - package-ecosystem: 'cargo'
    directory: '/'
    schedule:
      interval: 'weekly'
      day: 'monday'
    assignees:
      - 'vanyastaff'
    open-pull-requests-limit: 5
    labels:
      - 'dependencies'
      - 'skip-coderabbit'
    commit-message:
      prefix: 'chore(deps)'
    groups:
      # Group all minor+patch updates into one PR
      rust-minor-patch:
        update-types:
          - 'minor'
          - 'patch'
      # Major updates get individual PRs (breaking changes)
```

Replace the github-actions block (lines 22-37) with:

```yaml
  # GitHub Actions versions
  - package-ecosystem: 'github-actions'
    directory: '/'
    schedule:
      interval: 'weekly'
      day: 'monday'
    assignees:
      - 'vanyastaff'
    open-pull-requests-limit: 5
    labels:
      - 'dependencies'
      - 'skip-coderabbit'
    commit-message:
      prefix: 'chore(ci)'
    groups:
      # Group all action updates into one PR
      actions:
        patterns:
          - '*'
```

- [ ] **Step 2: Verify YAML parses**

Open the file, eyeball indentation. Each `labels:` block should have 2 entries.

- [ ] **Step 3: Stage**

```bash
git add .github/dependabot.yml
```

### Task 2.5: Remove Cursor Bugbot from required checks (manual GitHub UI)

This is a **non-code** action requiring repo admin access.

- [ ] **Step 1: Open repo settings**

URL: `https://github.com/vanyastaff/nebula/settings/branches`

- [ ] **Step 2: Edit branch protection rule for `main`**

Click `Edit` on the `main` rule.

- [ ] **Step 3: Uncheck `Cursor Bugbot` from required status checks**

Find `Cursor Bugbot` in the required checks list, uncheck the box. Click `Save changes`.

- [ ] **Step 4: Verify by triggering a no-op PR**

After Phase 2 PR opens, observe in checks list: `Cursor Bugbot` still appears as a check (it still runs) but is NOT marked as required. Merge becomes possible without it being green.

### Task 2.6: Commit Phase 2 and open PR

- [ ] **Step 1: Confirm staged files**

Run: `git status`
Expected staged: `.coderabbit.yaml`, `.github/copilot-instructions.md`, `.github/dependabot.yml`. Deleted: `commitlint.config.cjs`.

- [ ] **Step 2: Commit**

```bash
git commit -m "chore(bots): dim CodeRabbit, scope Copilot, prune duplicates

CodeRabbit:
- profile chill (was assertive) — only serious findings
- request_changes_workflow off — does not block merge
- sequence_diagrams off, drafts off, incremental_reviews off, auto_reply off
- linear knowledge base off (not used)
- path_instructions added for credential/sandbox/engine/tests
- path_filters extended (docs/superpowers, *.snap, Cargo.lock)

Copilot review (.github/copilot-instructions.md):
- Stop-list added (style, naming, generic suggestions, README, things CodeRabbit owns)
- Critical patterns enumerated (layer violations, panic safety, silent errors,
  direct state mutation per #255, missing Send+Sync, untrusted Duration)
- Division of labor with CodeRabbit made explicit

Dependabot: open-pull-requests-limit + labels [dependencies, skip-coderabbit] —
CodeRabbit will skip dependabot PRs.

Removed duplicate commitlint.config.cjs (canonical: .mjs).

Manual step (post-merge): GitHub Settings → Branches → main →
uncheck Cursor Bugbot from required checks. Documented in plan.

Refs: docs/superpowers/specs/2026-04-16-dev-environment-design.md §7-§9

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 3: Push and open PR**

```bash
git push -u origin <branch-name>
gh pr create --title "chore(bots): dim CodeRabbit, scope Copilot, prune duplicates" --body "$(cat <<'EOF'
## Summary
- CodeRabbit: chill profile, no diagrams/drafts/incremental/auto-reply/linear, path_instructions for credential/sandbox/engine/tests
- Copilot review: stop-list to suppress generic noise; critical patterns enumerated
- Dependabot PRs labeled \`skip-coderabbit\` — bot won't review version bumps
- Removed duplicate \`commitlint.config.cjs\` (kept \`.mjs\`)

Phase 2 of [dev environment hardening plan](docs/superpowers/plans/2026-04-16-dev-environment-hardening.md).

**Manual follow-up after merge:** uncheck \`Cursor Bugbot\` from required status checks in GitHub Settings → Branches.

## Test plan
- [ ] CodeRabbit on this PR shows: high-level summary, walkthrough collapsed, NO sequence diagram, NO blocking review request
- [ ] Copilot review (if it comments) does not include generic "consider adding tests/docs"
- [ ] Next dependabot PR has \`dependencies\` and \`skip-coderabbit\` labels and CodeRabbit does not review it
- [ ] CI commitlint check still passes (uses \`.mjs\`)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 4: Wait for CI to pass**

Run: `gh pr checks <pr-number> --watch`

---

## Phase 3: CI dedup + paths-filtering

**PR title:** `ci: dedup tests, paths-filter cross-platform, drop CodeQL`
**Risk:** medium — touches required CI checks. Validate after merge that PR check list shrinks.
**Acceptance from spec §13:** items 6, 7, 8, 9, 10.

### Task 3.1: Edit `.github/workflows/ci.yml` — remove `test` job, add `doctests` job

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Remove the `test` job (lines 96-132)**

Open `.github/workflows/ci.yml` and delete the entire `test` block (job name `Tests`):

```yaml
  # ── Tests ──────────────────────────────────────────────────────────────────
  test:
    name: Tests
    needs: [clippy]
    runs-on: ubuntu-latest
    steps:
      - name: Checkout repository
        uses: actions/checkout@v6
      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
      - name: Cache Cargo artifacts
        uses: Swatinem/rust-cache@v2
        with:
          shared-key: ci-test
          save-if: ${{ github.ref == 'refs/heads/main' }}
          cache-on-failure: true
      - name: Install cargo-nextest
        uses: taiki-e/install-action@cargo-nextest
      - name: Run tests with nextest
        run: cargo nextest run --workspace --profile ci
      - name: Run doctests
        run: cargo test --workspace --doc
      - name: Test summary
        if: always()
        uses: test-summary/action@v2
        env:
          FORCE_JAVASCRIPT_ACTIONS_TO_NODE24: true
        with:
          paths: target/nextest/ci/junit.xml
      - name: Upload test results
        if: always()
        uses: actions/upload-artifact@v7
        with:
          name: test-results
          path: target/nextest/ci/junit.xml
          if-no-files-found: ignore
          retention-days: 3
```

- [ ] **Step 2: Update `bench` job dependencies**

The `bench` job currently has `needs: [test]`. Since `test` is gone, change it to `needs: [check]`.

In the `bench` job (around line 175), find:

```yaml
  bench:
    name: Benchmark Thresholds
    if: github.event_name != 'pull_request'
    needs: [test]
```

Change to:

```yaml
  bench:
    name: Benchmark Thresholds
    if: github.event_name != 'pull_request'
    needs: [check]
```

- [ ] **Step 3: Add `doctests` job after the `check` job**

Insert this new job right after the `check` job (around line 95, before MSRV):

```yaml
  # ── Doctests (workspace-wide) ──────────────────────────────────────────────
  doctests:
    name: Doctests
    needs: [check]
    runs-on: ubuntu-latest
    steps:
      - name: Checkout repository
        uses: actions/checkout@v6
      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
      - name: Cache Cargo artifacts
        uses: Swatinem/rust-cache@v2
        with:
          shared-key: ci-doctests
          save-if: ${{ github.ref == 'refs/heads/main' }}
          cache-on-failure: true
      - name: Run doctests
        run: cargo test --workspace --doc
```

- [ ] **Step 4: Verify YAML parses**

Run: `taplo fmt --check .github/workflows/ci.yml` (taplo doesn't lint YAML structure but catches gross indentation errors). Better: open in editor and verify indentation visually.

- [ ] **Step 5: Stage**

```bash
git add .github/workflows/ci.yml
```

### Task 3.2: Create `.github/workflows/cross-platform.yml`

**Files:**
- Create: `.github/workflows/cross-platform.yml`

- [ ] **Step 1: Create the file**

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

permissions:
  contents: read

env:
  CARGO_TERM_COLOR: always
  CARGO_INCREMENTAL: 0
  CARGO_NET_RETRY: 10
  RUSTUP_MAX_RETRIES: 10
  RUST_BACKTRACE: short

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
      - name: Checkout repository
        uses: actions/checkout@v6
      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
      - name: Cache Cargo artifacts
        uses: Swatinem/rust-cache@v2
        with:
          shared-key: cross-platform
          save-if: ${{ github.ref == 'refs/heads/main' }}
          cache-on-failure: true
      - name: Run platform-sensitive crate tests
        run: |
          cargo test -p nebula-sandbox
          cargo test -p nebula-runtime
          cargo test -p nebula-plugin-sdk
```

- [ ] **Step 2: Stage**

```bash
git add .github/workflows/cross-platform.yml
```

### Task 3.3: Edit `.github/workflows/test-matrix.yml` — remove cross-platform job

**Files:**
- Modify: `.github/workflows/test-matrix.yml`

- [ ] **Step 1: Delete the `cross-platform-sandbox-smoke` job (lines 146-169)**

Open `.github/workflows/test-matrix.yml` and delete the entire job block:

```yaml
  cross-platform-sandbox-smoke:
    name: Cross-platform sandbox smoke (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    steps:
      - name: Checkout repository
        uses: actions/checkout@v6
      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
      - name: Cache Cargo artifacts
        uses: Swatinem/rust-cache@v2
        with:
          shared-key: matrix-cross-platform
          save-if: false
          cache-on-failure: true
      - name: Run platform-sensitive crate tests
        run: |
          cargo test -p nebula-sandbox
          cargo test -p nebula-runtime
          cargo test -p nebula-plugin-sdk
```

The file should now end after the `test-crates` job's last step.

- [ ] **Step 2: Verify YAML parses**

Open in editor, verify file ends cleanly.

- [ ] **Step 3: Stage**

```bash
git add .github/workflows/test-matrix.yml
```

### Task 3.4: Edit `.github/workflows/pr-validation.yml` — condition on title-edit

**Files:**
- Modify: `.github/workflows/pr-validation.yml`

- [ ] **Step 1: Add `if:` condition to `lint-metadata` job**

Open `.github/workflows/pr-validation.yml`. Find the `lint-metadata` job (line 16) and add the `if:` line right after `runs-on: ubuntu-latest`:

```yaml
jobs:
  lint-metadata:
    name: Validate PR metadata
    runs-on: ubuntu-latest
    if: github.event.action != 'edited' || github.event.changes.title != null
    steps:
      - name: Checkout
        uses: actions/checkout@v6
        with:
          fetch-depth: 0
      # ... rest unchanged
```

The condition: skip the job entirely if the PR was `edited` but the change wasn't to the title (e.g. body or label edit). Other action types (`opened`, `synchronize`, `reopened`) always run.

- [ ] **Step 2: Stage**

```bash
git add .github/workflows/pr-validation.yml
```

### Task 3.5: Disable CodeQL default code scanning (manual GitHub UI)

This is a **non-code** action requiring repo admin access.

- [ ] **Step 1: Open code scanning settings**

URL: `https://github.com/vanyastaff/nebula/settings/security_analysis`

- [ ] **Step 2: Disable default code scanning**

Find `Code scanning` section. Click the gear icon next to `CodeQL analysis` → `Disable CodeQL`.

- [ ] **Step 3: Verify on next PR**

After Phase 3 PR opens, the checks list should NOT contain `Analyze (actions)` or `Analyze (javascript-typescript)` or `CodeQL`. If they still appear, repeat step 2 — make sure the toggle is fully off.

### Task 3.6: Commit Phase 3 and open PR

- [ ] **Step 1: Confirm staged files**

Run: `git status`
Expected staged: `.github/workflows/ci.yml` (modified), `.github/workflows/cross-platform.yml` (new), `.github/workflows/test-matrix.yml` (modified), `.github/workflows/pr-validation.yml` (modified).

- [ ] **Step 2: Commit**

```bash
git commit -m "ci: dedup tests, paths-filter cross-platform, drop CodeQL

ci.yml: removed Tests job (workspace nextest) — per-crate matrix in
test-matrix.yml fully covers it. Added Doctests job (off critical
path) to retain --doc coverage.

Cross-platform sandbox smoke moved to its own workflow with paths
filter on crates/{sandbox,runtime,plugin-sdk}/** — no longer wastes
runner-minutes on PRs touching unrelated crates.

pr-validation: skip lint-metadata on body-only edits (was duplicating
on title vs body edit events).

Manual step (post-merge): GitHub Settings → Code security → disable
CodeQL default code scanning. Removes Analyze (actions/javascript-typescript)
checks. Documented in plan.

Numerical effect: PR check count 34 -> ~20, wall-time ~12-15min -> ~5-7min.

Refs: docs/superpowers/specs/2026-04-16-dev-environment-design.md §6

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 3: Push and open PR**

```bash
git push -u origin <branch-name>
gh pr create --title "ci: dedup tests, paths-filter cross-platform, drop CodeQL" --body "$(cat <<'EOF'
## Summary
- ci.yml: removed duplicate \`Tests\` workspace job (per-crate matrix already covers it); added lighter \`Doctests\` job
- New \`cross-platform.yml\` workflow with paths-filter on sandbox/runtime/plugin-sdk — no longer runs on every PR
- \`pr-validation.yml\`: skip duplicate runs on body-only edits
- Manual follow-up: disable CodeQL default code scanning in repo settings

Phase 3 of [dev environment hardening plan](docs/superpowers/plans/2026-04-16-dev-environment-hardening.md).

## Test plan
- [ ] On this PR's checks: \`Tests\` (workspace) is gone; \`Doctests\` is present and green
- [ ] Per-crate \`Test nebula-X\` matrix from test-matrix.yml unchanged
- [ ] \`Cross-platform sandbox smoke\` does NOT appear on this PR (no sandbox/runtime/plugin-sdk changes)
- [ ] \`Validate PR metadata\` appears once, not twice
- [ ] After CodeQL disabled in settings: \`Analyze (actions)\` and \`Analyze (javascript-typescript)\` disappear
- [ ] Total checks ≤ 22 (was 34)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 4: Wait for CI and verify check count**

Run: `gh pr checks <pr-number>`
Expected: ≤ 22 checks. If `Tests` (workspace) still appears or count is higher, debug — likely a YAML edit error in ci.yml.

---

## Phase 4: release-plz migration

**PR title:** `ci(release): replace manual cargo-release with release-plz + semver-checks`
**Risk:** medium-high — replaces release flow. Coordinate timing (no in-flight release work).
**Acceptance from spec §13:** items 16, 17, 18.

### Task 4.1: Create `release-plz.toml`

**Files:**
- Create: `release-plz.toml`

- [ ] **Step 1: Create the file**

```toml
# release-plz.toml
# Workspace-wide release configuration.
# release-plz opens a release PR on each push to main containing version bumps
# + auto-generated changelog entries (using existing cliff.toml).
# Merging the release PR creates GitHub releases and tags.
# Publishing to crates.io is gated on `publish = true` per package.

[workspace]
allow_dirty = false
changelog_update = true
git_release_enable = true
publish = false             # alpha: do not publish to crates.io yet; flip to true at 1.0
publish_no_verify = false
semver_check = true         # uses cargo-semver-checks
pr_branch_prefix = "release-plz/"
pr_labels = ["release", "skip-coderabbit"]

# Per-package overrides go here when needed, e.g.:
# [[package]]
# name = "nebula-core"
# changelog_path = "crates/core/CHANGELOG.md"
```

- [ ] **Step 2: Stage**

```bash
git add release-plz.toml
```

### Task 4.2: Create `.github/workflows/release-plz.yml`

**Files:**
- Create: `.github/workflows/release-plz.yml`

- [ ] **Step 1: Create the file**

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
    name: release-plz
    runs-on: ubuntu-latest
    steps:
      - name: Checkout repository
        uses: actions/checkout@v6
        with:
          fetch-depth: 0
          token: ${{ secrets.GITHUB_TOKEN }}

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Run release-plz
        uses: MarcoIeni/release-plz-action@v0.5
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          # CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
          # ^ uncomment when leaving alpha and flipping publish=true in release-plz.toml
```

- [ ] **Step 2: Stage**

```bash
git add .github/workflows/release-plz.yml
```

### Task 4.3: Create `.github/workflows/semver-checks.yml`

**Files:**
- Create: `.github/workflows/semver-checks.yml`

- [ ] **Step 1: Create the file**

```yaml
name: SemVer Checks

on:
  pull_request:
    paths:
      - "crates/**/Cargo.toml"
      - "crates/**/src/**"
  workflow_dispatch:

permissions:
  contents: read
  pull-requests: write

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

env:
  CARGO_TERM_COLOR: always

jobs:
  semver:
    name: cargo-semver-checks
    runs-on: ubuntu-latest
    # Advisory only during alpha — does not block merge.
    # Make this `required: true` in branch protection when leaving alpha.
    continue-on-error: true
    steps:
      - name: Checkout repository
        uses: actions/checkout@v6
      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
      - name: Run cargo-semver-checks
        uses: obi1kenobi/cargo-semver-checks-action@v2
        with:
          rust-toolchain: stable
```

- [ ] **Step 2: Stage**

```bash
git add .github/workflows/semver-checks.yml
```

### Task 4.4: Delete old `.github/workflows/release.yml`

**Files:**
- Delete: `.github/workflows/release.yml`

- [ ] **Step 1: Verify nothing else references release.yml**

Run: `grep -rn "release.yml" .github/ docs/ README.md 2>/dev/null`
Expected: zero matches (or only matches in this plan / spec, which is fine — they describe the deletion).

- [ ] **Step 2: Delete the file**

Run: `git rm .github/workflows/release.yml`

### Task 4.5: Verify release-plz can run on a dry trigger (optional, manual)

This step verifies that release-plz finds the workspace and reads `release-plz.toml` correctly.

- [ ] **Step 1: Run release-plz locally in dry mode (if installed)**

Run: `release-plz update --dry-run`
Expected: outputs proposed version bumps and changelog entries based on commits since last tag. No errors about missing config.
Skip if release-plz not installed locally — `mise install` (Phase 1) provides it.

### Task 4.6: Commit Phase 4 and open PR

- [ ] **Step 1: Confirm staged files**

Run: `git status`
Expected staged: `release-plz.toml` (new), `.github/workflows/release-plz.yml` (new), `.github/workflows/semver-checks.yml` (new). Deleted: `.github/workflows/release.yml`.

- [ ] **Step 2: Commit**

```bash
git commit -m "ci(release): replace manual cargo-release with release-plz + semver-checks

release-plz opens a release PR on each push to main with version bumps
and auto-generated changelog entries (reuses cliff.toml). Merging the
release PR creates GitHub releases and tags. Publishing to crates.io is
gated on publish=true (currently false during alpha; flip at 1.0).

cargo-semver-checks runs on PRs touching crates/**/Cargo.toml or
crates/**/src/** as advisory (continue-on-error). Becomes required
when leaving alpha.

Removed manual workflow_dispatch release.yml — replaced by automated
release-plz flow.

Refs: docs/superpowers/specs/2026-04-16-dev-environment-design.md §10, §12

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 3: Push and open PR**

```bash
git push -u origin <branch-name>
gh pr create --title "ci(release): replace manual cargo-release with release-plz + semver-checks" --body "$(cat <<'EOF'
## Summary
- New \`release-plz.toml\` and \`.github/workflows/release-plz.yml\` — automated release-PR on each merge to main
- New \`.github/workflows/semver-checks.yml\` — advisory cargo-semver-checks on Cargo.toml/src changes
- Deleted manual \`.github/workflows/release.yml\` (workflow_dispatch flow)

\`publish = false\` in release-plz.toml during alpha — flip to true at 1.0 (and uncomment \`CARGO_REGISTRY_TOKEN\` secret in workflow).

Phase 4 of [dev environment hardening plan](docs/superpowers/plans/2026-04-16-dev-environment-hardening.md).

## Test plan
- [ ] After merge to main, release-plz workflow runs and opens (or updates) a release PR within 5 min
- [ ] Release PR contains version bumps + changelog entries
- [ ] semver-checks workflow runs on this PR (advisory — does not block)
- [ ] No reference to old release.yml remains

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 4: After merge, verify release-plz opened a release PR**

Within 5 min of merge to main:

Run: `gh pr list --label release`
Expected: a PR titled "chore: release vX.Y.Z" or similar exists, authored by `github-actions[bot]`, on branch `release-plz/main`.

If no PR appears within 10 min: check workflow run logs for errors.
Run: `gh run list --workflow=release-plz.yml --limit 3`

---

## Phase 5: Verification + memory update

**Goal:** verify all 20 acceptance criteria from spec §13 hold; update auto-memory so future sessions know about the new pre-push mirror-CI guarantee.

### Task 5.1: Walk through spec §13 acceptance criteria

Open [`docs/superpowers/specs/2026-04-16-dev-environment-design.md`](../specs/2026-04-16-dev-environment-design.md) §13. For each numbered item, verify it holds in the current state.

- [ ] **Local (items 1-5):**
  - [ ] 1. `git commit` on Rust-only change runs fmt+clippy+typos+taplo+cargo-deny in parallel ≤ 10s warm.
  - [ ] 2. `git commit` with non-conventional message rejected by convco.
  - [ ] 3. `git push` runs all 7 pre-push checks in parallel ≤ 90s warm.
  - [ ] 4. `cargo check` 30-50% faster than baseline (3 iterations, sccache hit rate visible).
  - [ ] 5. `mise install` provisions all tooling on a fresh shell.

- [ ] **CI (items 6-10):**
  - [ ] 6. PR touching one crate (no sandbox/runtime) shows ~20 status checks, ≤ 7 min wall-time.
  - [ ] 7. PR touching `crates/sandbox/**` triggers `cross-platform.yml`.
  - [ ] 8. PR with title edit does not generate duplicate `Validate PR metadata` check.
  - [ ] 9. CodeQL `Analyze (*)` no longer in PR checks.
  - [ ] 10. `Tests` workspace job gone; `Doctests` present and green; per-crate matrix unchanged.

- [ ] **Bots (items 11-15):**
  - [ ] 11. CodeRabbit review on typical PR: summary + collapsed walkthrough + per-file changes; NO sequence diagram, NO incremental re-review, NO auto-reply in chat.
  - [ ] 12. CodeRabbit does not create blocking "request changes" review.
  - [ ] 13. CodeRabbit makes no review on draft PR.
  - [ ] 14. Copilot review on typical PR has no generic "consider adding tests/docs/error handling" without named case.
  - [ ] 15. Cursor Bugbot not in branch protection required checks.

- [ ] **Release (items 16-18):**
  - [ ] 16. Merging feature PR triggers release-plz; release PR opens within 5 min.
  - [ ] 17. Merging release PR creates GitHub release + tag (publish=false → no crates.io publish during alpha; verify by setting publish=true on a single test crate after 1.0).
  - [ ] 18. Old `release.yml` deleted.

- [ ] **Hygiene (items 19-20):**
  - [ ] 19. Only `commitlint.config.mjs` exists (not `.cjs`).
  - [ ] 20. Dependabot PRs have `dependencies` + `skip-coderabbit` labels and CodeRabbit does not review them. (Wait for next dependabot run, ~weekly.)

For any item that does NOT hold: file an issue tagged `dev-env-followup` and link this plan + the spec section.

### Task 5.2: Update auto-memory with new pre-push guarantee

This prevents future sessions from re-introducing the lefthook ↔ CI divergence.

- [ ] **Step 1: Create memory file**

Path: `C:\Users\vanya\.claude\projects\C--Users-vanya-RustroverProjects-nebula\memory\feedback_lefthook_mirrors_ci.md`

Content:

```markdown
---
name: lefthook pre-push mirrors CI required jobs
description: Pre-push must run every CI required check (taplo, doctests, --all-features, --no-default-features, MSRV, doc -D warnings); never let the two diverge
type: feedback
---

After 2026-04-16 dev-env hardening: `lefthook.yml` `pre-push` is the
authoritative local mirror of CI's required jobs. Any new CI required
check MUST also be added to `pre-push`.

**Why:** prior to this work, lefthook skipped taplo, doctests, --all-features,
--no-default-features, MSRV, and commit-msg lint — which all ran in CI.
Result: pushes failed CI for checks the developer thought were green locally.
The user explicitly requested this fixed.

**How to apply:**
- When adding a new CI required job: simultaneously add the equivalent command
  to `lefthook.yml` `pre-push` (or `pre-commit` if it's a fast file-level check).
- When removing a check from CI: also remove from `lefthook.yml` to keep them
  in sync.
- Source of truth for the policy: `docs/superpowers/specs/2026-04-16-dev-environment-design.md` §1.1.
```

- [ ] **Step 2: Add pointer to MEMORY.md**

Open `C:\Users\vanya\.claude\projects\C--Users-vanya-RustroverProjects-nebula\memory\MEMORY.md` and add under `## Feedback`:

```markdown
- [feedback_lefthook_mirrors_ci.md](feedback_lefthook_mirrors_ci.md) — Lefthook pre-push must mirror every CI required job (don't let them diverge again)
```

- [ ] **Step 3: Verify the addition is one line under 150 chars**

Read the line you just added. Confirm: starts with `- [`, has the `— hook` separator, fits one line.

---

## Self-Review (post-write)

I performed self-review against the spec. Findings:

**1. Spec coverage:**
- §1.1 (Lefthook ↔ CI gaps): Task 1.3 closes all 6 gaps (taplo in pre-commit, doctests/all-features/no-default/MSRV/docs in pre-push, commit-msg via convco).
- §1.2 (CI duplication): Tasks 3.1-3.5 address all 5 listed sub-issues (test dedup, pr-validation dedup, cross-platform paths-filter, CodeQL disable, warm-cache kept as-is documented).
- §1.3 (CodeRabbit noise): Task 2.1 changes all 8 listed settings.
- §1.4 (Copilot stop-list): Task 2.2 rewrites file with stop-list.
- §1.5 (Cursor Bugbot): Task 2.5.
- §1.6 (Hygiene): Task 2.3 (commitlint dedup), Task 1.4 (mise.toml).
- §2 Goals 1-7: Tasks 1.3 (G1), 3.1-3.5 (G2), 2.1 (G3), 2.2 (G4), 1.1+1.4 (G5), 1.4 (G6), 4.1-4.4 (G7).
- §11 release-plz §10 mise.toml §12 semver-checks: Tasks 4.1-4.4 + 1.4 + 4.3.
- §13 Acceptance criteria: Phase 5 walks through all 20 items.
- §16 PR grouping: Phases 1-4 = the 4 PRs, in the order specified.
- §17 DoD: Task 5.1 verifies acceptance, Task 5.2 updates memory.

**Coverage gaps:** none found.

**2. Placeholder scan:** no TBD/TODO/"implement later"/"add appropriate X" remain. All commands explicit, all code blocks complete.

**3. Type consistency:**
- `mise.toml` uses `cargo:cargo-shear` consistently (not `cargo-machete` per user's correction).
- `release-plz.toml` `publish = false` matches §10 of spec (alpha, not 1.0 yet).
- `.coderabbit.yaml` `linear: enabled: false` matches user's "not used" answer.
- `pre-push` parallel = true (was sequential) — consistent across plan and spec.

No inconsistencies found.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-04-16-dev-environment-hardening.md`.**

Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task (or per phase), review between tasks, fast iteration. Best for this plan because phases are independent and can be parallelized after Phase 1 lands.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints. Best if you want to watch each step and intervene live.

**Which approach?**
