# AGENTS.md

**Derived from:** `docs/AGENT_PROTOCOL.md`; `docs/PRODUCT_CANON.md` §15; `rust-toolchain.toml`; `.github/workflows/ci.yml`; `.github/workflows/test-matrix.yml`; `.cursor/rules/*.mdc`.

This repository uses **Cursor MDC rules** under **`.cursor/rules/`** for scoped guidance. **`docs/AGENT_PROTOCOL.md`** is the single agent contract: **Universal principles** first (project rules, design stance, Rust 1.95+ defaults), then verbatim operational rules. **`docs/STYLE.md`** §0–2 house style and mindset; **`docs/IDIOM_REVIEW_CHECKLIST.md`** is the checkable review pass for pattern/control-flow edits (mechanics that support the principles). Optional: **`docs/RUST_EXPERT_STYLE_GUIDE.md`** (split under `docs/guidelines/`) for **rule-ID–style** deep Rust reference — subordinate to Nebula canon.

## Required reading order

1. **`docs/AGENT_PROTOCOL.md`** — meta-protocol (single source of truth).
2. **`docs/PRODUCT_CANON.md`** — normative product and engineering rules.
3. **`.cursor/rules/`** — modular rules; `alwaysApply` rules load every session.

Use **`docs/PRODUCT_CANON.md` §15** to pick satellites (`INTEGRATION_MODEL`, `STYLE`, `GLOSSARY`, `MATURITY`, `OBSERVABILITY`, `ENGINE_GUARANTEES`, `UPGRADE_COMPAT`, `pitfalls`, ADRs) for your task.

## Verification (**observable**)

- Toolchain: **`rust-toolchain.toml`** (`channel = "1.95.0"`).
- CI formatting: **`cargo +nightly fmt --all -- --check`** — `.github/workflows/ci.yml` `fmt` job.
- CI clippy: **`cargo clippy --workspace -- -D warnings`** — `.github/workflows/ci.yml` `clippy` job.
- Tests: **`cargo nextest run`** — `.github/workflows/test-matrix.yml`.

## Claude Code

**`CLAUDE.md`** duplicates this pointer with a Claude-oriented tone and the docs→rules mapping table.
