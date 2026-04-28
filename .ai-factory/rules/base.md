# Project Base Rules

> Auto-detected from `clippy.toml`, `rustfmt.toml`, `CONTRIBUTING.md`, `deny.toml`,
> and the workspace `Cargo.toml`. Edit when conventions change. The README and
> CONTRIBUTING.md are the source of truth — this file is the agent-facing
> distillation.

## Toolchain

- Rust **1.95** stable for build / clippy / test (pinned via `rust-toolchain.toml`
  and `workspace.package.rust-version`).
- **Nightly rustfmt** required for formatting (`cargo +nightly fmt --all`) — see
  `rustfmt.toml`.
- Workspace edition: **2024**, resolver: **3**.
- Test runner: **`cargo nextest run`** for unit/integration tests; doctests run
  via `cargo test --workspace --doc`.

## Naming Conventions

- Files / modules: `snake_case`.
- Types / traits: `PascalCase`.
- Functions / vars: `snake_case`.
- Crates: `nebula-<area>` (e.g. `nebula-engine`, `nebula-credential`). Scope in
  commit messages drops the `nebula-` prefix.
- Test names describe behaviour (`rejects_negative_timeout`, not `test_1`).
- Single/double-letter idents allowed only from the whitelist in `clippy.toml`
  (`i`, `j`, `k`, `n`, `m`, `id`, `db`, `tx`, `rx`, `fs`, `io`, `to`, `up`, `ok`,
  `fn`, `a`–`c`, `x`–`z`).

## Module / Crate Structure

- Layered workspace enforced by `cargo deny` (`deny.toml` `[wrappers]`):
  - **Cross-cutting**: `log`, `system`, `eventbus`, `telemetry`, `metrics`,
    `resilience`, `error`.
  - **Core**: `core`, `validator`, `expression`, `workflow`, `execution`,
    `schema`, `metadata`.
  - **Business**: `credential`, `resource`, `action`, `plugin`.
  - **Exec**: `engine`, `storage`, `sandbox`, `plugin-sdk`.
  - **API / Public**: `api`, `sdk`.
- Cross-crate communication goes through `nebula-eventbus`, **not** direct
  imports between sibling crates at the same layer.
- Macros live in their own sub-crate (`crates/<crate>/macros/`) to keep proc-macro
  build cost out of the runtime crate.

## Error Handling

- Library crates: `thiserror`-derived typed errors. **No** `unwrap()`, `expect()`,
  or `panic!()` in library code (allowed in tests, `const`, and binaries — see
  `clippy.toml` `allow-*-in-tests` flags).
- Binary crates / examples: `anyhow` is acceptable.
- New error variants must carry enough context to drive a recovery decision; see
  the error taxonomy upstream of `nebula-error`.
- Avoid `Box<dyn Error>` defaults — prefer typed errors. Same for `async-trait`
  on hot paths and `Arc<Mutex<...>>` over single-writer designs (Rust 1.95+
  idioms).

## Logging / Tracing / Telemetry

- `tracing` is the only logging API in libraries. Crates expose typed events;
  formatting/sinks live in `nebula-log` / `nebula-telemetry`.
- Every new state, error, or hot path must ship with a typed error variant **and**
  a tracing span / event **and** an invariant check. Observability is part of
  Definition of Done, not a follow-up.
- `metrics` instrumentation goes through `nebula-metrics`.

## Resilience

- Outgoing calls and any operation that can fail under load go through
  `nebula-resilience` primitives (retry, circuit breaker, bulkhead, hedged,
  rate-limit). Compose, do not re-implement.

## Tests

- Unit tests: `mod tests` inside the source file.
- Integration tests: `crates/<crate>/tests/`.
- Runnable examples: root-level `examples/` workspace member, **not** per-crate
  `examples/` directories.
- `loom` / property tests / fuzz live in `crates/<crate>/fuzz/` (excluded from
  the workspace, run separately).

## Linting

- `cargo clippy --workspace --all-targets -- -D warnings` is the gate (zero
  warnings). Crate-specific allow-lists must be justified in the crate root.
- `clippy.toml` raises complexity and ergonomics thresholds appropriately for a
  generic-heavy codebase — do not tune them down per crate without discussion.
- `cargo deny check` (`deny.toml`) is part of pre-commit and CI: catches
  forbidden cross-crate wrappers, license issues, advisories, duplicate deps.

## Documentation

- Doc comment on every public item; broken intra-doc links fail CI
  (`-D rustdoc::broken_intra_doc_links`). Do not bracket out-of-scope paths in
  inner doc comments — `rustdoc -D warnings` cannot resolve them.
- Per-crate `README.md` is the human entry point; `cargo doc --no-deps` is the
  reference. README and CONTRIBUTING.md in the repo root cover project-level
  guidance.

## Git / Commits

- Branch from `main`. Names: `<username>/neb-<id>-<kebab-title>` (Linear-linked)
  or `<type>/<short-kebab-description>` (no issue).
- Conventional Commits, validated by `convco` in pre-commit and CI.
  Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `chore`,
  `ci`, `build`, `revert`.
- Scope = crate name without `nebula-` prefix, or top-level area
  (e.g. `feat(resilience): …`, `docs: …`).
- **Squash-merge only** to keep `main` linear.
- `lefthook pre-push` mirrors the CI required jobs — keep them in sync; do not
  let pre-push and CI diverge.

## Security

- `nebula-credential` and webhook paths require `CODEOWNERS` sign-off.
- Secrets must be encrypted (AES-256-GCM with AAD), zeroized on `Drop`, and
  redacted in `Debug`. There is no `legacy_compat` flag.
- `cargo deny advisories` is a CI gate.

## ADRs

- Architecture decisions live as ADRs in the project. ADRs are point-in-time —
  if following one forces workarounds, **supersede** it instead of patching
  around it.
