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

**lefthook does not run.** Re-install hooks: `lefthook install --force`. Verify hooks exist:
`ls .git/hooks/pre-commit .git/hooks/commit-msg .git/hooks/pre-push`.
