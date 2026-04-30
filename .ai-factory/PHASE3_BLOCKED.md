# Phase 3 — Closeout Status (Final)

This is the post-closeout status note. Phase 3 foundation (commit
`04be3cf9`) shipped the trait/type bedrock; commit `defa983a` shipped the
macro rewrite + factory infrastructure; this final commit completes the
test-fixture migration sweep.

## What shipped (this commit)

### Subsystem A — DX integration tests (Variant A migration)
- `crates/action/tests/dx_control.rs` (760 LoC, 23 tests passing)
- `crates/action/tests/dx_poll.rs` (1259 LoC, 36 tests passing)
- `crates/action/tests/dx_webhook.rs` (569 LoC, 13 tests passing)

All `#![cfg(any())]` markers and `// phase3_disabled` comments removed.
Each fixture migrated from `meta: ActionMetadata` field + instance-method
shape to Variant A static metadata + struct-as-instance-state separation
per the established pattern.

### Subsystem B — `#[derive(Action)]` trybuild probes
- New `crates/action/tests/derive_action_compile_fail.rs` driver.
- 8 compile-fail probes covering: missing `input = ...`, missing
  `output = ...`, unknown attribute key, conflicting slot keys,
  `#[resource]` on non-`ResourceGuard` field, `#[credential]` on
  non-`CredentialGuard` field, both `#[resource]` and `#[credential]` on
  same field, tuple struct rejection.
- 1 compile-pass smoke probe for unit + named-field structs.

### Subsystem C — engine + integration tests
- `crates/engine/src/runtime/registry.rs` — added test escape
  `ActionRegistry::legacy_register_*_with_metadata()` (4 variants:
  stateless/stateful/trigger/resource) per Plan-agent R-NEW-7.
  Marked `pub` (not `pub(crate)`) so integration tests across crates
  can use it. Documented as "LEGACY test-only — production code uses
  register_*_factory::<A>()".
- `crates/engine/src/runtime/registry.rs::tests` — 3 tests passing.
- `crates/engine/src/runtime/runtime.rs::tests` — 24 tests passing
  (1.4K LoC tests block migrated; ~14 register sites).
- `crates/engine/src/engine.rs::tests` — 4200 LoC tests block migrated;
  6 fixture types + 5 inline fixtures + ~55 register sites; all 234
  engine lib tests pass.
- `crates/engine/tests/integration.rs` — 18 tests passing (33 register
  sites; 7 fixture types migrated via shared `variant_a_action!` macro).
- `crates/engine/tests/control_dispatch.rs` — 12 tests passing
  (2 fixture types).
- `crates/engine/tests/resource_integration.rs` — 3 tests passing
  (2 fixture types).
- `crates/plugin/tests/resolved_plugin.rs` — 16 tests passing.
  `StubAction` rewritten to impl `ActionFactory` directly (since
  `Plugin::actions()` returns `Vec<Arc<dyn ActionFactory>>`).
- `crates/api/tests/knife.rs` — 2 fixture types migrated.
- `crates/api/tests/webhook_transport_integration.rs` — 4 fixture types
  migrated.
- `crates/api/Cargo.toml` — added `nebula-schema` to dev-dependencies.
- `crates/plugin/Cargo.toml` — added `nebula-workflow` to
  dev-dependencies.

## What remains deferred (Subsystem D — dispatch crossover)

Final dispatch crossover (replace `ActionHandler` consumption with
`ErasedAction` from the factory path) remains as a follow-up commit:

- `crates/engine/src/runtime/runtime.rs::ActionRuntime::run_handler`
  still matches on `ActionHandler` enum directly. Switching to factory
  dispatch requires plumbing `NodeDefinition` through every public
  `execute_action_*` entry point so the factory can be invoked with the
  current node — non-trivial because today's `execute_action` only
  takes `(action_key, input, ctx)`.
- The legacy `XxxHandler` family + `XxxActionAdapter` types in
  `crates/action/src/{stateless,stateful,trigger,resource,control,...}`
  remain in production code paths because the legacy test escape +
  legacy production register helpers still produce them. Once dispatch
  crossover lands, these can be deleted.

The factory infrastructure (`ActionFactory`, `GenericXxxFactory<A>`,
`ErasedAction`, parallel `factories: DashMap` in `ActionRegistry`) is
in place and exercised by unit tests. The crossover is a structural
refactor of the runtime entry point, not new infrastructure.

## Verification gates (all green)

- `cargo check --workspace --all-targets` — green
- `cargo clippy --workspace --all-targets -- -D warnings` — green
- `cargo test --workspace` — all enabled tests pass
- `RUSTDOCFLAGS=-D rustdoc::broken_intra_doc_links cargo doc --no-deps --workspace` — green (2 redundant-link warnings only, not link-broken; same as baseline)
- `cargo deny --log-level error check` — `advisories ok, bans ok, licenses ok, sources ok`

## Test counts (post-migration)

| Subsystem | Tests added/re-enabled | Status |
|-----------|------------------------|--------|
| dx_control | 23 | passing |
| dx_poll | 36 | passing |
| dx_webhook | 13 | passing |
| derive_action_compile_fail | 8 fail + 1 pass probe | passing |
| engine lib (incl. engine.rs + registry + runtime tests blocks) | 234 | passing |
| engine integration.rs | 18 | passing |
| engine control_dispatch.rs | 12 | passing |
| engine resource_integration.rs | 3 | passing |
| plugin resolved_plugin.rs | 16 | passing |
| api knife.rs + webhook_transport_integration.rs | 19+ (combined) | passing |
