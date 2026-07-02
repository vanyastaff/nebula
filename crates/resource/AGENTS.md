# nebula-resource — Agent orientation
> Agent quick-map for `crates/resource/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** Engine-owned resource lifecycle (acquire / health-check / hot-reload / scope-bounded release) for pool & SDK-client integrations, handed to actions as a drop-releasing `ResourceGuard`.
**Layer:** Business — depends only downward (root AGENTS.md → Layered Dependency Map).

## Commands
- `cargo check -p nebula-resource`
- `cargo nextest run -p nebula-resource --all-features`  ·  doctests: `cargo test -p nebula-resource --doc --all-features`
- `cargo nextest run -p nebula-resource --features rotation` — exercises the credential-rotation fan-out (`tests/resident_rotation_race.rs`, `tests/credential_slot_epoch_fold.rs`); no `test-util` feature exists on this crate or on `nebula-credential` (removed step 8 of ADR-0092)
- Derive crate: `cargo check -p nebula-resource-macros` (companion in `macros/`); examples in root `nebula-examples` (`--example resource_*`)

## Key files
- `src/lib.rs` — crate facade + re-exports; `cell::Cell` deliberately NOT re-exported (use `SlotCell`)
- `src/resource.rs` — `Provider` trait (`Config`/`Instance` assoc types, slot-rotation hooks), `HasCredentialSlots`, `ResourceConfig`, `ResourceMetadata`; `Resource` is the derive macro (slot plumbing only)
- `src/slot.rs` / `src/cell.rs` — `SlotCell` (public, generation-stamped) vs internal epoch-blind `cell::Cell`
- `src/registry.rs` — type-erased registry, scope-aware lookup, `(key, scope)` dedup
- `src/manager/` — `Manager::register(RegistrationSpec)` funnel, acquire dispatch, shutdown/drain
- `src/topology/contract.rs` — the open `Topology<R>` trait (entry-centric, framework-driven; **slot** = credential axis, **entry** = store axis — see `src/topology/store.rs` module docs). The **framework** owns the acquire loop (`ManagedResource::run_acquire_loop`): fenced `store.checkout()`, stale-entry destroy, cancel-safe wrap, on-release return-or-destroy. A topology supplies only thin R-aware hooks (`create_entry` / `entry_instance` / `into_instance` / `accept` / `prepare` / `on_release` / `pools` / `store_capacity` / `dispatch_credential_hook` / …) and **cannot** reach the revoke fence — never write `store.checkout` / `resource.destroy` / a stale loop / an epoch compare in a `Topology` impl.
- `src/topology/` + `src/runtime/` — `Pooled<R>` / `Resident<R>` / `Bounded<R>` built-in topologies (`Topology<R>` impls; Bounded = runtime concurrency cap, capped/exclusive/unbounded, no warm pool); the framework-owned `InstanceStore<Entry>` is the real idle queue (`ManagedResource.store`)
- `src/release_queue.rs` — `ReleaseQueue` best-effort async drain (canon §11.4); `src/recovery/` — thundering-herd `RecoveryGate`

## Conventions & never-do
- Credentials are declared as `#[credential(key="…")] field: SlotCell<CredentialGuard<C>>`; read via derive-emitted `self.<field>_slot()` (`Option<Arc<…>>`, handle `None`/unbound) — never off the raw cell. No singular `Resource::Credential`; `NoCredential` is gone.
- This crate is NOT a connection driver, retry pipeline, secret holder, or expression evaluator — it owns the lifecycle wrapper only (see Non-goals).
- Async release is best-effort on crash; never assume "release ran" without an explicit checkpoint (canon §11.4).
- `#![forbid(unsafe_code)]` + `#![deny(missing_docs)]` + `#![warn(missing_debug_implementations)]` are active; lifecycle work emits a `ResourceEvent` variant (observability is DoD).
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design, migration recipe (pre-v4 → v4), topology & shared-resource reference
- `docs/topology-reference.md` — topology selection guidance; canon invariants L2-§11.4 / §13.3
