# Developer Setup

This document describes the local toolchain expected by Nebula's lefthook hooks
and CI mirror.

## Quick start

1. Install [rustup](https://rustup.rs/) and pick a toolchain:
   `rustup default stable`. The workspace MSRV is **1.94** (pinned via
   `workspace.package.rust-version` in `Cargo.toml`); CI also runs an
   explicit MSRV-1.94 check.
2. Install workspace dev tools:
   ```bash
   bash scripts/install-tools.sh
   ```
   The script bootstraps `cargo-binstall` (one-time) and uses prebuilt
   binaries for the rest. Re-running is safe and fast.
3. Install lefthook hooks: `lefthook install`.
4. Optional but recommended: configure sccache cache directory.

That is the entire setup.

## What `scripts/install-tools.sh` provides

| Tool | Purpose |
|---|---|
| `taplo` | TOML formatter; checked in pre-commit |
| `typos` | Spell-check; checked in pre-commit |
| `cargo-nextest` | Test runner; used in pre-push and CI |
| `cargo-deny` | License + advisory check; pre-commit |
| `cargo-shear` | Unused dependency detection; pre-push |
| `cargo-semver-checks` | SemVer compliance check; advisory CI |
| `cargo-audit` | Vulnerability scan; weekly CI cron |
| `cargo-release` | Workspace version management |
| `sccache` | Compilation cache; speeds up cargo across branches |
| `convco` | Conventional-commit check; commit-msg hook |

## Environment variables for agent / CI runs

A Claude Code PreToolUse hook (`.claude/hooks/inject-rust-env.sh`) prepends
the canonical Rust env vars to every `Bash` tool invocation:

- `RUSTC_WRAPPER=sccache`, `SCCACHE_CACHE_SIZE=50G`, `CARGO_INCREMENTAL=0`
- `CARGO_TERM_COLOR=never`, `CARGO_NET_RETRY=3`
- `RUST_BACKTRACE=1`, `RUSTDOCFLAGS="-D warnings"`, `RUST_LOG=warn`
- `NEXTEST_PROFILE=agent`
- `LANG=en_US.UTF-8`, `LC_ALL=en_US.UTF-8`

For interactive shells, mirror what you need in your shell profile (or run
the same `export` lines once per session).

## sccache cache directory

The cache directory defaults to `~/.cache/sccache` (Linux/Mac) or
`%LOCALAPPDATA%\sccache` (Windows). To override, set `SCCACHE_DIR` in your
shell:

```bash
# bash/zsh
export SCCACHE_DIR=$HOME/.cache/sccache

# PowerShell
$env:SCCACHE_DIR = "$env:LOCALAPPDATA\sccache"
```

Verify sccache is active: `sccache --show-stats`. The "Cache hits" counter
increases when cargo reuses cached artifacts.

## Multi-session, worktrees, and IDE separation

Nebula is set up for parallel work — multiple agent sessions, multiple
worktrees, IDE running alongside cargo. Two file-lock conflict classes
that cause "hang" symptoms are handled by repo config:

**1. Incremental + sccache conflict.** `.cargo/config.toml [build] incremental = false`
disables Cargo's incremental compilation universally. Required when using
sccache (otherwise incremental dirs bypass the cache and produce stale
fingerprints across processes). This single setting fixes ~80% of "lock
hang" symptoms.

**2. IDE rust-analyzer fighting cargo for `target/`.** RA holds write locks
on `target/debug/.fingerprint/*` and `target/debug/deps/*.exe` while
indexing. If you run `cargo build` (manually or via agent) at the same
time, both processes block.

The repo-shipped fix is **a separate target dir for RA**:

- **VSCode / Zed / any rust-analyzer-aware editor:** `.vscode/settings.json`
  sets `rust-analyzer.cargo.targetDir = "target/ra"` (and matching env
  in `cargo.extraEnv` / `check.extraEnv`).
- **RustRover / IntelliJ Rust:** **manual** one-time setting (per-user, not
  committed). Settings → Languages & Frameworks → Rust → External Linters
  → "Additional arguments": add `--target-dir target/ra`. Or set
  `CARGO_TARGET_DIR=target/ra` in the Cargo run-configuration env.

After this, RA writes to `target/ra/`, agents and your shell write to
`target/`. No file-lock collision. The `target/ra` directory is
gitignored automatically (covered by the `target/` rule).

**3. Worktrees: per-worktree `target/`.** Each `git worktree` checkout has
its own `target/` (Cargo's default — do **not** set a repo-wide shared
`CARGO_TARGET_DIR`). Compilation artifacts are shared across worktrees
through the sccache store, not through a shared target dir.

**Known unavoidable contention.** All worktrees and sessions share
`~/.cargo/registry/.package-cache.lock`. Two `cargo install` or
`cargo update` from different worktrees will serialize on this lock —
that's by design. Avoid running tool-install commands in parallel.

## Linker

The repo's `.cargo/config.toml` configures `rust-lld` for Windows MSVC
(bundled with rustup since 1.81 — no install needed). On Linux/Mac the
default linker is used.

For contributors who want the 3-5× faster Linux/Mac linker, add this to your
**personal** `~/.cargo/config.toml` (NOT the repo `.cargo/config.toml` —
GitHub Linux runners don't have `mold` and would fail to build):

```toml
# Linux: install mold first — apt install mold clang
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]

# macOS: install lld first — brew install llvm
[target.aarch64-apple-darwin]
rustflags = ["-C", "link-arg=-fuse-ld=lld"]

[target.x86_64-apple-darwin]
rustflags = ["-C", "link-arg=-fuse-ld=lld"]
```

If you see "linker not found" errors locally, either install the linker per
the above commands, or remove the override from your personal
`~/.cargo/config.toml`.

## Lefthook hooks

After `lefthook install`, three hooks run on git events:

- **pre-commit** (≤10s): fmt-check, clippy, typos, taplo, cargo-deny on changed files.
- **commit-msg** (instant): convco validates the commit message follows conventional commits.
- **pre-push** (≤90s parallel): nextest, doctests, --all-features check, --no-default-features check, docs, MSRV check, cargo-shear. This is a complete mirror of CI's required jobs.

If pre-push fails, fix locally before pushing. CI cannot fail without
pre-push also failing on the same code.

To bypass a hook in an emergency (will be caught by CI):
`git commit --no-verify` or `git push --no-verify`. Avoid this — it defeats
the purpose.

## LLM agent profile

For LLM-driven workflows (Claude Code, Cursor, etc.), use the `agent`
nextest profile:

```bash
cargo nextest run --profile agent -p <crate>
```

This profile is fail-fast, suppresses success output, and limits slow-test
reporting — optimized for tight LLM context.

For agent scripts that parse cargo output, set `CARGO_TERM_COLOR=never` to
strip ANSI escape codes (the agent hook already does this):

```bash
CARGO_TERM_COLOR=never cargo check -p <crate>
```

## Releases

Releases are currently manual (the previous `release-plz` automation was
removed in favour of explicit version bumps). To cut a release of a single
crate:

```bash
cargo release -p <crate> <patch|minor|major> --execute
```

For workspace-wide release planning, `cargo-release` reads `[workspace.package.version]`
from `Cargo.toml` — bump there and run with `--workspace`.

## Troubleshooting

**"convco: command not found" in commit-msg hook.** Re-run
`bash scripts/install-tools.sh`, or manually: `cargo install convco`.

**"cargo +1.94: toolchain not installed" in pre-push.** The MSRV check
skips gracefully if 1.94 is missing. To enable it locally:
`rustup install 1.94`.

**Pre-push too slow.** Expected ~60-90s on warm cache (sccache + workspace
cache). On cold cache the first run is several minutes. If consistently
>2min on warm cache, check `sccache --show-stats` for cache hit rate.

**lefthook does not run.** Re-install hooks: `lefthook install --force`.
Verify hooks exist:
`ls .git/hooks/pre-commit .git/hooks/commit-msg .git/hooks/pre-push`.
