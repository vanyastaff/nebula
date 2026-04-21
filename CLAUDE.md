# CLAUDE.md

Operational guidance for coding agents in this repository.

## Product canon (mandatory)

- Read [`docs/PRODUCT_CANON.md`](docs/PRODUCT_CANON.md) before non-trivial design or execution-lifecycle changes.
- Do not implement workarounds that violate its **Non-negotiable invariants**; fix the root cause, or stop and ask.
- Definition of done includes: relevant tests/commands green **and** alignment with `docs/PRODUCT_CANON.md` (no silent semantic drift, no duplicate undocumented lifecycles). See §17 there.

### Session read order (priming layer)

Every session, before proposing changes, load in this order:

1. `CLAUDE.md` (you are here) — commands, conventions, this read-order.
2. `docs/PRODUCT_CANON.md` — normative core. If you hit a rule that seems to block a good improvement, check §0.2 canon revision triggers before giving up.
3. `docs/MATURITY.md` — which crates are frontier vs stable; calibrates proposal ambition.
4. `docs/STYLE.md` — idioms, antipatterns, naming, error taxonomy. Gate on any new public type or API.
5. When working inside a specific crate: that crate's `README.md` and `lib.rs //!`.

Satellites loaded on demand:
- `docs/INTEGRATION_MODEL.md` — integration model details (Resource / Credential / Action / Plugin / Schema).
- `docs/COMPETITIVE.md` — positioning, peer analysis.
- `docs/OBSERVABILITY.md` — SLI / SLO / events / core analysis loop.
- `docs/GLOSSARY.md` — terms and architectural patterns.
- `docs/pitfalls.md` — recurring traps (expression builtin re-entry, two-valued skip, OTLP/tonic test reactor, serde MapAccess); read on review of new public types or dispatch surfaces.
- [`docs/adr/README.md`](docs/adr/README.md) — ADR index (past decisions, numbering rules, how to write a new one).

### Decision gate (before proposing an architectural change)

Answer these six questions to yourself. If any answer implies a canon violation,
stop and open an ADR — see `docs/PRODUCT_CANON.md §0.2`.

1. Does this strengthen the golden path (PRODUCT_CANON §10) or divert it?
2. Does this introduce a public surface the engine does not yet honor end-to-end (§4.5)?
3. Does this change an L2 invariant without an ADR?
4. Does this leak detail upward (cross-cutting crate depending on integration crate)?
5. Does this introduce an implicit durable backbone via in-memory channel (§12.2)?
6. Does this advertise a capability in docs that the code does not deliver (§11.6)?

### Quick Win trap catalog

Recognize these traps; prefer the deeper fix:

- **Rename / redefine to avoid a contract.** If a type conflicts with an invariant, do not rename the type — open an ADR about the invariant.
- **`Clone` to satisfy the borrow checker.** Consider `Cow<'_, T>`, lifetime redesign, or typestate first. Document the tradeoff if cloning is the right answer.
- **Suppress the error with `.unwrap_or_default()` / `.ok()`.** Surface the error with proper classification (`NebulaError`, `Classify`) unless the default is documented-correct.
- **Add a `_` prefix to an "unused" var to silence the lint.** The variable is either needed (use it) or not (delete it). Shim-naming is canon-level feedback (see memory `feedback_direct_state_mutation.md` equivalent).
- **Patch a symptom in a downstream crate.** Root cause may be upstream; propose the fix there even if the PR is bigger.
- **Log-and-discard on an outbox consumer.** Violates §12.2. Either wire a real consumer or mark the path `// DEMO ONLY`.

## Project Snapshot

Nebula is a modular, type-safe workflow automation engine in Rust (alpha stage).
The workspace contains core libraries, execution/runtime layers, API, CLI, and examples.

- Rust edition: `2024`
- Rust version: `1.95`
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
- `lefthook run pre-push` is a crate-diff local gate (`nextest` + `--all-features` checks on changed crates, plus selected `--no-default-features`). Full doctests/docs/MSRV remain CI-owned checks. When adding or removing push-time checks, update `lefthook.yml` in the same PR.
- Commit messages use conventional commits (`feat:`, `fix(scope):`, `chore:` …); `pr-validation.yml` enforces this via `convco`.
- Releases are manual: `cargo release -p <crate> <patch|minor|major> --execute` (see `docs/dev-setup.md`).

## Architecture Boundaries

Layer direction is one-way:

```
API / Public   api (HTTP + webhook module) · sdk (integration author façade)
  ↑
Exec           engine · runtime · storage · sandbox · plugin-sdk
  ↑
Business       credential · resource · action · plugin
  ↑
Core           core · validator · expression · workflow · execution · schema · metadata

Cross-cutting  log · system · eventbus · telemetry · metrics · resilience · error
```

- No upward dependencies.
- Enforced partly by `deny.toml` (`cargo deny`) and partly by code review.
- Webhook is a module under `crates/api/src/webhook/`, not a separate crate.
- `nebula-sdk` is the external integration-author surface (re-exports action / credential / resource / schema / workflow / plugin / validator). Only `examples` may depend on it (see `deny.toml`).
- `nebula-plugin-sdk` is the out-of-process plugin protocol; only `sandbox` may depend on it directly.

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

- Keep security guarantees intact (credentials, secrets, auth boundaries). See [`docs/STYLE.md §6 — Secret handling`](docs/STYLE.md#6-secret-handling) for mandatory patterns, anti-patterns, and the log-redaction test helper.
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