# Phase 3 — Final Status (Closed)

Phase 3 is complete. The four-commit sequence on this branch:

1. `04be3cf9` — Variant A trait + ErasedAction + ActionFactory + plugin migration.
2. `defa983a` — `#[derive(Action)]` macro rewrite + 5 generic factories + selective fixture sweep.
3. `d5f79923` — DX integration tests + trybuild probes + engine fixture migration (~7K LoC, ~270 tests).
4. **(this)** — `ActionRuntime` dispatch crossover (Subsystem D) — production dispatch routes through `Arc<dyn ActionFactory>` + `Box<dyn ErasedAction>`.

## What Subsystem D shipped

### Runtime dispatch path
- `crates/engine/src/runtime/runtime.rs::ActionRuntime` now exposes
  `execute_action_with_node(&NodeDefinition, …)` as the production entry
  point. The legacy `execute_action`, `execute_action_versioned`,
  `execute_action_with_checkpoint` entry points remain for synthetic
  dispatch (admin tooling, tests) and synthesize a minimal
  `NodeDefinition` from the action key before delegating to the same
  inner `dispatch_action` helper.
- New private helper `dispatch_action` consults
  `ActionRegistry::get_factory()` first; on hit it routes through the
  new `run_factory()` which calls `factory.instantiate(node, ctx).await
  -> Box<dyn ErasedAction>` and dispatches the matching variant
  (`Stateless` / `Stateful` / `Control` execute; `Trigger` / `Resource`
  early-reject with the same metric labels as `run_handler`).
- Three new private helpers — `execute_erased_stateless`,
  `execute_erased_stateful`, `execute_erased_control` — mirror the
  legacy `execute_stateless` / `execute_stateful` paths against the
  `ErasedXxx` trait family. The stateful loop preserves the cancel
  race, checkpoint contract (#308), iteration cap, and stuck-state
  guard (spec 28 §9.0) 1:1.
- Legacy `run_handler` retained intact as the fallback path for
  `legacy_register_*_with_metadata` test fixtures and for any future
  registrations that bypass the factory pipeline.

### NodeDefinition plumbing
- `crates/engine/src/engine.rs::NodeTask` carries
  `node: Arc<nebula_workflow::NodeDefinition>` and dispatches via
  `runtime.execute_action_with_node(&self.node, …)` so the workflow
  node's `slot_bindings` reach the factory at instantiation time.
- `spawn_node` clones the borrowed `node_def` into the `Arc` at task
  construction time. The clone is `~200 bytes amortized` — well below
  the cost of cross-thread `Arc<NodeDefinition>` sharing for the
  uncommon multi-dispatch case.

### Regression coverage
- `crates/engine/src/engine.rs::tests::workflow_node_dispatches_through_factory_path` —
  pins that a `register_stateless_factory::<A>()` registration causes
  `factory.instantiate` to be called once per dispatch (verified via
  an atomic counter in `FromWorkflowNode::from_workflow_node`).
- `crates/engine/src/engine.rs::tests::factory_path_takes_precedence_over_legacy_handler` —
  pins that when both a factory and a legacy `ActionHandler` are
  registered for the same key, the factory wins. Tests serialize on a
  `tokio::sync::Mutex` so the global counter is observable per-test.

## Residual: legacy `XxxHandler` family kept

Audit results — the following production code paths still consume
`Arc<dyn XxxHandler>` directly and were intentionally NOT removed:

- **`crates/api/src/services/webhook/{routing,transport}.rs`** — webhook
  routing map keys on `Arc<dyn TriggerHandler>`. The transport delivers
  HTTP requests directly to a registered handler without going through
  `ActionRuntime`, so the factory path does not apply here.
- **`crates/sandbox/src/discovery.rs`** — plugin discovery wraps
  remotely-discovered actions as `Arc<dyn StatelessHandler>` for the
  Phase 0 in-process sandbox dispatch. Future RemoteActionFactory
  (post-Variant A sandbox slice) will move this onto the factory path.
- **`crates/sdk/src/runtime.rs`** — SDK testing harnesses (`run_poll`,
  `run_webhook`, etc.) wrap typed actions in adapter types and dispatch
  through the dyn-handler surface directly. These harnesses target
  unit-style "just run the trait" semantics rather than production
  dispatch, so they don't go through `ActionRuntime`.
- **`crates/engine/src/daemon/event_source.rs`** —
  `EventSourceAdapter<E>: TriggerHandler` carries dynamic per-instance
  metadata (caller-supplied `ActionMetadata` at construction). Per the
  comment at line 205-211, the typed `Action` / `TriggerAction`
  contracts require **static** metadata, so EventSource cannot ride
  the factory path; it implements the dyn-erased `TriggerHandler`
  surface directly.
- **`crates/engine/src/runtime/runtime.rs::run_handler`** — kept as the
  fallback dispatch path for `legacy_register_*_with_metadata` test
  fixtures. Production registrations via `register_*_factory::<A>()`
  flow through `run_factory` instead.

The `ActionHandler` enum, `XxxHandler` trait family, and
`XxxActionAdapter` types remain in `crates/action/src/` and are
re-exported from the crate root. They remain part of the public surface
for the four production consumers above plus the test escape.

## Verification gates (all green)

- `cargo check --workspace --all-targets` — green
- `cargo clippy --workspace --all-targets -- -D warnings` — green
- `cargo test --workspace` — all tests pass (engine lib: 236; +2 vs. previous)
- `RUSTDOCFLAGS=-D rustdoc::broken_intra_doc_links cargo doc --no-deps --workspace` — green (2 baseline warnings, same as previous commit)
- `cargo deny --log-level error check` — `advisories ok, bans ok, licenses ok, sources ok`
- `cargo fmt -- --check` — clean (touched crates verified individually due to long-path quirk on Windows)

## Phase 3 closure

Phase 3 (M6 dependency-redesign §6 Variant A) closes here. The
`ErasedAction`/`ActionFactory` infrastructure is now the production
dispatch path; the legacy `ActionHandler` enum + adapter family stays
as a service surface for transports/SDK harnesses/event sources that
operate outside the workflow-node dispatch loop and don't carry static
`ActionMetadata`.
