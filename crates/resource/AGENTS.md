# nebula-resource — Agent orientation
> Local guide for `crates/resource/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** Engine-owned resource lifecycle (acquire / health-check / hot-reload / scope-bounded release) for pool & SDK-client integrations, handed to actions as a drop-releasing `ResourceGuard`.
**Layer:** Business — depends only downward (root AGENTS.md → Layered Dependency Map).

## Commands

- `cargo nextest run -p nebula-resource --all-features`  ·  doctests: `cargo test -p nebula-resource --doc --all-features`
- `cargo nextest run -p nebula-resource --features rotation` — exercises the credential-rotation fan-out (`tests/resident_rotation_race.rs`, `tests/credential_slot_epoch_fold.rs`); neither this crate nor `nebula-credential` has a `test-util` feature.
- Derive crate: `cargo check -p nebula-resource-macros`; expansion contracts run in the parent and SDK suites below. Root examples are separate named targets, not a `--example resource_*` wildcard.

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
- For teardown changes, read `src/runtime/teardown.rs` and `src/manager/shutdown.rs` before relying on README hook descriptions. A declared provider hook is not proof of a runtime call site, and queued release is not completed physical destruction.
- `#![forbid(unsafe_code)]` + `#![deny(missing_docs)]` + `#![warn(missing_debug_implementations)]` are active; lifecycle work emits a `ResourceEvent` variant (observability is DoD).

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Acquire/guard/teardown | [acquire_lifecycle](tests/acquire_lifecycle.rs), [guard_release](tests/guard_release.rs), [recovery_and_shutdown](tests/recovery_and_shutdown.rs), [shutdown_race](tests/shutdown_race.rs). |
| Custom topologies or revoke fencing | [custom_topology_manager](tests/custom_topology_manager.rs), [revoke_recycle_toctou](tests/revoke_recycle_toctou.rs). |
| Credential rotation | [resident_rotation_race](tests/resident_rotation_race.rs), [credential_slot_epoch_fold](tests/credential_slot_epoch_fold.rs), both with `--features rotation`. |
| Derives | [derive_resource_compile_fail](tests/derive_resource_compile_fail.rs), [resource_config_derive](tests/resource_config_derive.rs), and SDK [derive_external_contract](../sdk/tests/derive_external_contract.rs). |

## See also

- `README.md` — full design, migration recipe (pre-v4 → v4), topology & shared-resource reference
- `docs/topology-reference.md` — topology selection guidance; canon invariants L2-§11.4 / §13.3
