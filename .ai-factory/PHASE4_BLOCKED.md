# Phase 4 — Deferred work (per-slot credential rotation dispatch)

> Status: documented at the Phase 4 commit boundary. The new
> `Resource::on_credential_refresh(&mut self, slot_name)` hook signature
> exists in the trait shape (per ADR-0044), but the engine-side
> reverse-index + fan-out machinery that delivers slot-name rotation
> events to per-resource hooks is **not** wired in this phase.

## What landed

- `Resource` trait reshape — Task 4.1 of
  `m6-resource-finalization-integration-audit.md`. Drops `type Credential`
  associated type. `create(&self, config, ctx)` no longer threads
  `scheme: &<R::Credential as Credential>::Scheme`. New per-slot
  `on_credential_refresh(&mut self, slot_name)` hook lives on the trait
  with a `Ok(())` default.
- `#[derive(Resource)]` macro rewrite — Task 4.2. Parses
  `#[resource(key, topology, config, runtime?, lease?, error?)]` struct
  attribute and `#[credential(key?, purpose?)]` field attributes; emits
  `Resource` impl shape (with `todo!()` `create` body — implementor
  supplies it) plus `DeclaresDependencies`.
- `Manager::register*` API rewrite — Task 4.3 simplified. All `register_*`
  variants now take a fully-resolved `R` value (slot fields populated).
  No more `R::Credential = NoCredential` bounds. `acquire_*_default`
  shorthand collapsed into the single `acquire_*` family.
- `Manager::register_from_value<R>(json, …)` — JSON path with template
  resolution. **Stubbed** — not implemented (per the spec's explicit
  permission to defer this to Phase 9 cross-crate audit).
- All 33+ existing `Resource` impl sites migrated — `type Credential` line
  removed, `scheme: &…` parameter dropped. Tests exercise the new shape.

## What is deferred

### 1. Per-slot rotation reverse-index + fan-out dispatch

The previous singular-credential model was implemented as:

- `Manager.credential_resources: DashMap<CredentialId, Vec<Arc<dyn ResourceDispatcher>>>`
- `TypedDispatcher<R>` downcast a type-erased `Box<SchemeFactory<R::Credential>>`,
  called `factory.acquire().await` to mint a fresh `SchemeGuard`, and
  forwarded it to `Resource::on_credential_refresh(scheme, ctx)`.
- `Manager::on_credential_refreshed::<C>(credential_id, factory, ctx)` and
  `on_credential_revoked(credential_id)` ran the fan-out via `join_all`
  with per-resource timeout budgets and `RotationOutcome` aggregate
  events.

This whole subsystem (`crates/resource/src/manager/rotation.rs`,
`crates/resource/src/manager/registration.rs`, the `credential_resources`
DashMap, `RegisterOptions::credential_id` /
`credential_rotation_timeout`, `ManagerConfig::credential_rotation_timeout`,
`ResourceEvent::CredentialRefreshed` / `CredentialRevoked`,
`metrics::OutcomeBoundCounters`/`OutcomeBoundHistograms`,
`error::RefreshOutcome` / `RevokeOutcome` / `RotationOutcome`,
`Error::scheme_type_mismatch::<R>()`) was deleted in Phase 4 because
every type signature in it referenced the now-removed `R::Credential`
projection. Re-adding it requires a fresh design that:

1. **Maps credentials to slot-bearing resources via `Dependencies`.** At
   register time, walk `R::dependencies().slot_fields()` to enumerate
   every `#[credential]` slot; for each slot, the caller (engine
   dispatch) supplies the resolved `CredentialId`; manager records
   `(CredentialId, ResourceKey, slot_name)` triples in a per-credential
   reverse index.
2. **Resolves `&mut self` reentrancy.** `on_credential_refresh(&mut self,
   slot_name)` requires exclusive access. `ManagedResource<R>` currently
   exposes `Arc<R>`; switching to `Arc<RwLock<R>>` (or per-slot field
   interior mutability) is a non-trivial design decision that affects
   acquire latency and was not finished in this phase.
3. **Re-binds the `Manager::on_credential_refreshed` API** to take only
   `(credential_id, ctx)` — the engine no longer threads
   `SchemeFactory<C>` through the manager. Per-slot refresh is
   credential-data-free at the Manager API surface; the resource hook
   reads its updated `CredentialGuard<C>` slot field directly off
   `&mut self` (the engine resolved the new credential into the slot
   before calling the hook).

This scope is comparable to the original ADR-0036 Wave-2 implementation
(Tech Spec §3.2-§3.5 plus security amendments B-1/B-2/B-3) and warrants
its own milestone. Issue / plan note: candidate **§M11.5 — Per-slot
rotation dispatch** in `.ai-factory/ROADMAP.md`.

### 2. `Manager::register_from_value<R>(json, expr_engine, …)` JSON path

The Phase 9 (cross-crate integration audit) seam from
`m6-resource-finalization-integration-audit.md`. Currently no
implementation — the typed `register*<R>(...)` API covers all callers
in this phase (every existing test/runtime call uses a typed Config
value).

When wired, the flow is: deserialize Config from JSON (resolving
`{{ ... }}` expression templates via `nebula_expression`), validate
against `<R::Config as HasSchema>::schema()`, then dispatch into the
typed `register<R>(...)`.

### 3. Migration guide / codemod

Phase 11.1 work item from the master plan
(`m6-resource-finalization-integration-audit.md`). The diff in this
phase already shows the mechanical migration shape (drop `type
Credential`, drop `_scheme:` arg, replace `on_credential_refresh(scheme,
ctx)` with `on_credential_refresh(&mut self, slot_name)`); a
`cargo-nebula-migrate-resource` codemod will land alongside the
Phase 11 docs sweep.

### 4. Topology trait re-validation

Pool / Resident / Service / Transport / Exclusive trait files (in
`crates/resource/src/topology/`) no longer thread the credential bound,
but their docstrings still reference scheme threading in spots —
mechanical doc cleanup, no behavioural change. Folded into Phase 11
docs sweep.

## Verification snapshot at this commit

- `cargo check --workspace --all-targets` — green
- `cargo clippy --workspace --all-targets -- -D warnings` — green
- `cargo test --workspace` — green (rotation-machinery tests deleted as
  obsolete)
- `RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps
  --workspace` — green for all touched crates (2 pre-existing
  redundant-link warnings in `nebula-action` are not Phase 4 surface)
- `cargo deny --log-level error check` — green
