# Phase 3 — Closeout Status

This is the post-closeout status note. Phase 3 foundation (commit 04be3cf9)
shipped the trait/type bedrock; this commit adds:

- **S3 macro rewrite** (full Variant A `#[derive(Action)]` with field-level
  `#[resource]` / `#[credential]` slot detection + auto-generated
  `FromWorkflowNode` impl).
- **GenericXxxFactory<A>** types — Variant A implementations of
  `ActionFactory` for each sub-trait family (`Stateless`, `Stateful`,
  `Trigger`, `Resource`, `Control`).
- **Engine `ActionRegistry` factory map** — parallel `DashMap<ActionKey,
  Vec<FactoryEntry>>` populated via new `register_*_factory::<A>()` helpers.
  Old `register_*(action)` helpers retained for backwards-compatible
  fixture migration.
- **Selective S5 fixture migration** — 12 disabled tests re-enabled and
  migrated to the new Variant A shape.

## What shipped (this commit)

### Macro (S3) — `crates/action/macros/src/`
- `action.rs` — derive emits Variant A `impl Action for X { type Input/Output;
  fn metadata/input_schema/output_schema/dependencies; }` plus a
  `FromWorkflowNode` impl with auto-generated slot resolution.
- `action_attrs.rs` — accepts `key`, `name` (defaults to struct ident),
  `description`, `version` (defaults to `0.1.0`), `input` (required),
  `output` (required). Rejects unknown keys with a clear diagnostic.
- `field_slots.rs` *(new)* — recognises four field shapes:
  `ResourceGuard<R>`, `Option<ResourceGuard<R>>`, `Lazy<ResourceGuard<R>>`,
  `Option<Lazy<ResourceGuard<R>>>` (and same for `CredentialGuard<C>`).
  Path-tail matching so both `ResourceGuard<...>` and
  `nebula_resource::ResourceGuard<...>` work. Detects duplicate slot keys.
- `crates/sdk/macros-support/src/attrs.rs` — added `parse_attr_optional()`
  helper that returns `Option<AttrArgs>` so callers can distinguish
  attribute-absent from attribute-present-with-no-items.

### Factory (S4) — `crates/action/src/factory.rs`
- `GenericStatelessFactory<A>` / `GenericStatefulFactory<A>` /
  `GenericTriggerFactory<A>` / `GenericResourceFactory<A>` /
  `GenericControlFactory<A>` — all `: ActionFactory`.
- Per-variant `ErasedXxxImpl<A>` private wrappers that bridge typed
  `Action` impl to `ErasedXxx` JSON-erased dispatch surface.

### Engine wiring — `crates/engine/src/runtime/registry.rs`
- New `register_*_factory::<A>()` helpers (one per variant: stateless,
  stateful, trigger, resource, control) that wrap typed actions in the
  matching `GenericXxxFactory<A>`.
- New `register_factory(metadata, factory)` low-level helper.
- New `get_factory(key)` / `get_factory_versioned(key, version)` lookup
  methods.
- Parallel `factories: DashMap<ActionKey, Vec<FactoryEntry>>` map alongside
  the legacy `actions: DashMap<ActionKey, Vec<ActionEntry>>` map. Engine
  dispatch can transition incrementally.

### Re-enabled / migrated tests (`crates/action/tests/`)
- `derive_action.rs` — fully migrated to new derive shape; 6 tests pass.
- `trigger_source_smoke.rs` — re-enabled (no migration needed).
- `idempotency_smoke.rs` — re-enabled (pure value tests).
- `metadata_max_concurrent_smoke.rs` — re-enabled (pure value tests).
- `type_sizes.rs` — re-enabled (pure value tests).
- `webhook_request_limits.rs` — re-enabled (no fixture migration).
- `webhook_signature.rs` — re-enabled (no fixture migration).
- `deferred_recovery.rs` — re-enabled (pure value tests).
- `contracts.rs` — re-enabled (pure serialization tests).
- `action_attr_phantom_rewrite.rs` — migrated to new derive shape.
- `dx_batch.rs` — migrated `BatchAction` fixture to Variant A static metadata.
- `dx_paginated.rs` — migrated `PaginatedAction` fixture to Variant A.
- `resource_roundtrip.rs` — migrated `ResourceAction` fixture to Variant A.
- `execution_integration.rs` — migrated 4 fixtures (Stateless, Stateful,
  Trigger, Stateful-with-migrate_state) to Variant A static metadata.

## What remains deferred (still `#![cfg(any())]`)

The following test files use the old `meta: ActionMetadata` field /
`fn metadata(&self) -> &ActionMetadata` instance-method shape across many
fixtures. Each requires mechanical migration to Variant A static metadata
+ struct-as-instance-state separation. The migration pattern is now
established (see `dx_batch.rs`, `dx_paginated.rs`,
`execution_integration.rs` for templates) but the bulk of the work is
laborious rather than architecturally interesting.

### nebula-action large DX tests
- `crates/action/tests/dx_control.rs` (~760 LoC, ~6 fixtures)
- `crates/action/tests/dx_poll.rs` (~1259 LoC, ~10 fixtures)
- `crates/action/tests/dx_webhook.rs` (~569 LoC, ~5 fixtures)

### nebula-engine internal tests (~16K LoC, ~60 register sites)
- `crates/engine/src/engine.rs` `mod tests` — ~4200 LoC of tests, disabled
  at line 3868. Heavy use of `EchoHandler { meta: ActionMetadata }`-style
  fixtures with dynamic per-test metadata keys. Per Plan-agent R-NEW-7,
  these need a `#[cfg(test)] fn legacy_register_with_dynamic_metadata`
  test escape — not yet implemented.
- `crates/engine/src/runtime/registry.rs` `mod tests` — disabled.
- `crates/engine/src/runtime/runtime.rs` `mod tests` — disabled.
- `crates/engine/tests/integration.rs`, `control_dispatch.rs`,
  `resource_integration.rs` — disabled.

### nebula-plugin / nebula-api integration tests
- `crates/plugin/tests/resolved_plugin.rs` (~449 LoC)
- `crates/api/tests/knife.rs`, `webhook_transport_integration.rs`

## Final dispatch wiring (S4 — partial)

The engine `ActionRegistry` now stores both a legacy `ActionHandler` map
AND a parallel `Arc<dyn ActionFactory>` map. The `ActionRuntime`
dispatch path in `crates/engine/src/runtime/runtime.rs` still consumes
`ActionHandler` exclusively — the factory map is wired but not yet
read by the dispatcher. The full crossover (replace
`ActionHandler` enum with `ErasedAction` consumption per ADR-0043 §6)
remains as a follow-up commit. The infrastructure is ready: `ErasedAction`
+ `ErasedXxx` sub-traits + `GenericXxxFactory<A>` are defined and tested
in unit-test compilation; only the `ActionRuntime::run_handler` body
needs to switch from matching on `ActionHandler` variants to:

```rust
let (_meta, factory) = self.registry.get_factory(&action_key)?;
let erased = factory.instantiate(&node, &ctx).await?;
match erased {
    ErasedAction::Stateless(h) => h.dispatch(input, ctx).await,
    ErasedAction::Stateful(h)  => h.dispatch(input, state, ctx).await,
    // ...
}
```

Because the disabled engine.rs tests (~4200 LoC) drive this dispatch path
heavily, swapping it requires the test-fixture migration to land first.

## Verification gates (all green)

- `cargo check --workspace --all-targets` — green
- `cargo clippy --workspace --all-targets -- -D warnings` — green
- `cargo test --workspace` — all enabled tests pass
- `RUSTDOCFLAGS=-D rustdoc::broken_intra_doc_links cargo doc --no-deps --workspace` — green (2 redundant-link warnings only, not link-broken)
- `cargo deny --log-level error check` — `advisories ok, bans ok, licenses ok, sources ok`

## Next-step playbook

1. Migrate the three large DX integration test files
   (`dx_control.rs`, `dx_poll.rs`, `dx_webhook.rs`). Pattern is established
   — apply mechanically.
2. Introduce `legacy_register_with_dynamic_metadata` test escape on
   `ActionRegistry` (test-only `#[cfg(test)]`) so the engine.rs ~60-fixture
   test block can re-enable. Then re-enable engine.rs / registry.rs / runtime.rs
   inline tests + the engine integration tests.
3. Migrate plugin/api integration tests.
4. Once all engine tests green, replace `ActionHandler` consumption in
   `crates/engine/src/runtime/runtime.rs::dispatch` with `ErasedAction`
   from the factory path. Drop `ActionHandler` enum entirely.
5. Drop the seven legacy `register_*(action)` helpers in
   `ActionRegistry` once all callers use `register_*_factory::<A>()`.
6. Add ~12 trybuild compile-fail probes for the derive macro:
   missing `input`, missing `output`, conflicting slot keys,
   `#[resource]` on non-`ResourceGuard` type, `#[credential]` on non-`CredentialGuard`,
   both `#[resource]` and `#[credential]` on same field, unknown
   `#[action(...)]` key, plus 4 positive probes for each guard
   shape (bare / `Option` / `Lazy` / `Option<Lazy>`).
