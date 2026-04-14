# [CLAUDE.md](http://CLAUDE.md)

Operational guidance for coding agents in this repository.

## Project Snapshot

Nebula is a modular, type-safe workflow automation engine in Rust (alpha stage).
The workspace contains core libraries, execution/runtime layers, API, CLI, and examples.

- Rust edition: `2024`
- Rust version: `1.94`
- Primary test runner: `cargo nextest`
- Formatting: nightly `rustfmt` (required by `rustfmt.toml`)

## Development Mode

This repository is in active development. Prefer the **best long-term design**, not the smallest diff.

- Bold refactors are allowed when they improve clarity, correctness, or architecture.
- Breaking changes are acceptable when they remove bad APIs or reduce complexity.
- Do not preserve flawed code for compatibility unless explicitly requested.
- When touching bad code, fix root causes instead of patching symptoms.

## Canonical Commands

```bash
# Fast local gate (default)
cargo +nightly fmt --all && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace

# Single crate iteration
cargo check -p nebula-<crate> && cargo nextest run -p nebula-<crate>

# Full validation (before PR)
cargo +nightly fmt --all && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace && cargo test --workspace --doc && cargo deny check
```

Notes:

- `cargo +nightly fmt` is required (unstable rustfmt options are enabled).
- Doctests are run separately with `cargo test --doc`.

## Architecture Boundaries

Layer direction is one-way:

```
API            api (+ webhook module)
  ↑
Exec           engine · runtime · storage · sandbox · sdk · plugin-sdk
  ↑
Business       credential · resource · action · plugin
  ↑
Core           core · validator · parameter · expression · workflow · execution

Cross-cutting  log · system · eventbus · telemetry · metrics · config · resilience · error
```

- No upward dependencies.
- Enforced partly by `deny.toml` (`cargo deny`) and partly by code review.
- Webhook is a module under `crates/api/src/webhook/`, not a separate crate.

## Engineering Defaults

- Prefer explicit, type-safe APIs over stringly-typed contracts.
- Use `serde_json::Value` as the workflow data interchange type.
- In library crates, use typed errors (`thiserror`); reserve `anyhow` for binaries.
- Keep secrets encrypted/redacted/zeroized when touching credential flows.
- Prefer deletion/simplification over compatibility shims when APIs are wrong.

## Agent Strategy

Do:

- Read current source and config files before making assumptions.
- Use `Cargo.toml`, `deny.toml`, and `.github/workflows/*` as policy sources of truth.
- Choose the best solution even if it requires broad edits.
- Refactor aggressively when it reduces technical debt.
- Run relevant verification commands for touched areas.
- Leave code simpler than you found it.

Don't:

- Reintroduce removed internal context systems or `.project/*` conventions.
- Assume historical crate layout/state from memory.
- Keep dead abstractions "just in case."
- Split obvious fixes into artificial micro-changes if it hurts solution quality.

## Safety Rails

Be bold on design, strict on safety:

- Keep security guarantees intact (credentials, secrets, auth boundaries).
- Preserve or improve test coverage around changed behavior.
- For high-risk changes, validate with targeted checks before finishing.
- If a refactor changes behavior intentionally, state that explicitly in the summary.

## Useful Local Workflows

```bash
task db:up
task db:migrate
task db:prepare
task desktop:dev
task obs:up
```

Use `task --list` for the full task catalog.