# budget-justified: topology bind-inversion migration plan — one coherent multi-task design document, prose + code sketch, not decomposable into smaller functions

# Topology Bind Inversion — restore safe-by-construction open topology

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or execute inline. Steps use checkbox (`- [ ]`) tracking.

**Goal:** Make the open `Topology` trait the real, *safe-by-construction* author surface again — invert control of the acquire loop so the **framework** owns the fenced checkout, stale-destroy, store, and guard-wrapping, and the per-topology bridge supplies only R-aware policy hooks it **cannot** use to skip the credential-revoke fence.

**Why:** The Phase-C convergence (the three commits ending at HEAD `5ad7a525`) landed registrability (a custom `impl Topology` registers + acquires through `Manager`) but regressed the three load-bearing properties the open trait exists for. Verified against the code:
- **`Topology::acquire` is dead** — `Manager` calls `ManagedHandle::acquire` → `TopologyDispatch::acquire_guard` (returns a full `ResourceGuard<R>`); the open `Topology::acquire` (→ `Lease<Slot>`) is never called. Built-ins are `type Slot = ()`; the open contract's `&InstanceStore<Slot>` arg is decorative.
- **Uniform fence is hand-rolled by custom authors** — `FfmpegPool::acquire_guard` itself writes `for stale in checkout.stale { resource.destroy(...) }`. An author who forgets it serves slots authorized under a since-revoked credential. Spec §2.6 "fence covers custom topologies by construction" is false — it is author discipline.
- **The `InstanceStore` rule is defeated** — `FfmpegPool` *owns* its `store: InstanceStore<u64>` field instead of receiving a borrowed `&store` it cannot retain. The §1 structural barrier against a cross-scope instance cache is gone for custom topologies (per-row registration limits the practical blast, but the principle is broken).

These hit exactly the target use case (community plugins) and exactly the product's moat (credential isolation). Built-ins stay safe (their own audited fence; TOCTOU tests green) — the regression is custom-topology-only.

**Architecture:** An R-aware bridge is *structurally unavoidable* — the open `Topology` is `R`-agnostic (a `FfmpegPool` knows nothing of the `Ffmpeg` `Provider`) yet a typed `ResourceGuard<R>` needs `R::Instance`, and create-on-acquire needs `&R`/config/ctx that `Topology::acquire(&self, ticket, store)` lacks. So the fix is **not** "remove the bridge" — it is **invert who owns the unsafe loop**: the framework runs `try_reserve → fenced checkout → destroy-stale → accept-or-create → wrap → (on drop) on_release + return_slot`, calling thin R-aware hooks for the create / accept / project bits. The bridge can no longer reach the fence.

**Tech Stack:** Rust edition 2024, Tokio, `async-trait`, `arc-swap`, `thiserror`. `#![forbid(unsafe_code)]`. Windows worktree (per-crate gates, never `--all`).

**Baseline:** branch `dreamy-kare-8698d4` at HEAD `5ad7a525`; `nebula-resource` 345 tests green, `nebula-engine` 421, `nebula-api`/examples compile, clippy `-D warnings` + rustdoc clean. That is the floor every step preserves.

---

## The new contract (target) — SLOT-CENTRIC (revised after plan review)

# budget-justified: slot-centric TopologyBind contract — single coherent design sketch + framework-loop pseudocode for this migration, prose not decomposable into smaller functions


`Topology::Slot` becomes **real** and carries the leasable unit for its whole lease — the guard holds the `Slot`, so per-slot metadata (`created_at` for max-lifetime, `fingerprint`, `checkout_count`) survives the checkout→lease→return round-trip. The framework store `ManagedResource.store: InstanceStore<<R::Topology as Topology>::Slot>` is the *actual* idle store the framework fences — no topology-owned store.

> **Review fix (🔴):** an earlier instance-centric draft (`create -> R::Instance` + `into_slot(instance)` on release) rebuilt the slot from the bare instance on every return, resetting `created_at` → **max-lifetime eviction would never fire**. Slot-centric (`create_slot -> Slot`, guard holds the slot, release returns the *same* slot) preserves metadata. `into_slot` is deleted.

```rust
// crates/resource/src/runtime/managed.rs — replaces TopologyDispatch<R>
/// R-aware policy hooks the framework calls *inside* its own acquire loop.
/// The framework owns try_reserve, the fenced checkout, stale-destroy, the
/// store, guard-wrapping, warmup, maintenance, and on-release return — so a
/// bind impl CANNOT skip the revoke fence or retain the store. The open
/// `Topology` (R-agnostic) supplies the concurrency gate + admission surface;
/// this supplies the slot lifecycle keyed to `R`.
#[async_trait]
pub trait TopologyBind<R: Provider>: Topology {
    /// Make one fresh, credential-resolved leasable slot. Pool builds
    /// `PoolSlot { instance: <create>, metrics: now, fingerprint }`; Resident
    /// clones the shared handle into `Slot = R::Instance`; a permit pool stores
    /// an id/handle. Credentials are resolved into the resource's slot cells
    /// before this runs (§2.6). Framework drives it (on idle-miss / warmup).
    async fn create_slot(&self, resource: &R, config: &R::Config, ctx: &ResourceContext)
        -> Result<Self::Slot, Error>;

    /// Project a held slot to its leasable instance — the guard's `Deref`
    /// target. Pool: `&slot.instance`; Resident: the slot itself.
    fn slot_instance<'s>(&self, slot: &'s Self::Slot) -> &'s R::Instance;

    /// Consume a slot back into its instance for `Provider::destroy`
    /// (stale-fenced / accept-rejected / maintenance-evicted slots).
    fn into_instance(&self, slot: Self::Slot) -> R::Instance;

    /// Validate a checked-out idle slot **in place** (Pool: stale-fingerprint /
    /// max-lifetime / `is_broken` / `test_on_checkout`). `false` => the
    /// framework destroys it (`into_instance` → `destroy`) and loops to the
    /// next idle slot / create. Default `true` (no post-checkout policy).
    async fn accept(&self, _slot: &mut Self::Slot, _resource: &R, _ctx: &ResourceContext) -> bool { true }

    /// Per-acquire session-init on the slot about to be leased (Pool `prepare`,
    /// `SET search_path`, …). Err => framework destroys the slot + fails the acquire.
    async fn prepare(&self, _slot: &mut Self::Slot, _ctx: &ResourceContext) -> Result<(), Error> { Ok(()) }

    /// Whether a released slot returns to the framework idle store (Pool: true;
    /// Resident / pure-permit: false → released slot is dropped, not pooled).
    fn pools(&self) -> bool { false }

    /// Idle count the framework pre-warms by calling `create_slot` + storing
    /// (fenced) at registration. 0 = no warmup.
    fn warmup_target(&self, _config: &R::Config) -> usize { 0 }

    /// Predicate for the framework maintenance reaper: should this idle slot be
    /// evicted now (Pool: stale-fingerprint / max-lifetime / idle-timeout)?
    /// The framework already evicts revoke-stale slots via `store.evict_stale()`.
    fn idle_evictable(&self, _slot: &Self::Slot) -> bool { false }

    /// `Some((idle_timeout, max_lifetime, interval))` if the framework should
    /// spawn a maintenance reaper for this topology; `None` = none.
    fn maintenance_schedule(&self) -> Option<MaintenanceSchedule> { None }

    /// Per-slot credential rotation hook, framework-driven over the live store.
    /// Default no-op (a topology with no pooled instances has nothing to rotate).
    async fn dispatch_credential_hook(&self, _resource: &R, _slot: &str, _refresh: bool)
        -> Result<(), Error> { Ok(()) }

    /// Update the config fingerprint so stale idle slots evict on next sweep/acquire.
    fn set_fingerprint(&self, _fingerprint: u64) {}
}
```

```text
// crates/resource/src/registry.rs (inside Manager::run_acquire's dispatch closure,
// which keeps owning resilience-gate + drain bookkeeping + post-taint re-check).
// blanket impl<R> ManagedHandle for ManagedResource<R> where R::Topology: TopologyBind<R>
async fn acquire(self: Arc<Self>, mgr, ctx, opts) -> ResourceGuard<R> {
    let ticket = self.topology.try_reserve(&self.store)?;          // gate (sync)
    let (slot, epoch) = loop {
        let checkout = self.store.checkout().await;                // FRAMEWORK fences on pop
        for stale in checkout.stale {                              // FRAMEWORK destroys stale — author cannot skip
            let _ = self.resource.destroy(self.topology.into_instance(stale)).await;
        }
        match checkout.fresh {
            Some(co) => {
                let (mut slot, epoch) = co.into_parts();
                if self.topology.accept(&mut slot, &self.resource, &ctx).await {
                    break (slot, epoch);
                }
                let _ = self.resource.destroy(self.topology.into_instance(slot)).await;
                // loop: try next idle, then create
            }
            None => break (self.topology.create_slot(&self.resource, self.config(), &ctx).await?,
                           self.store.stamp_epoch()),
        }
    };
    // Cancel-safety: from create_slot/checkout until the guard is built, a drop
    // here must `destroy` `slot` via the ReleaseQueue (reuse the existing
    // `CreateGuard` pattern, generalized over `Slot`) — without it a cancelled
    // acquire leaks a server-side instance (guarded by the cancel-drop test).
    let mut slot = /* CreateGuard-wrapped */ slot;
    self.topology.prepare(&mut slot, &ctx).await?;                 // FRAMEWORK session-init; Err => destroy + fail
    let guard = ResourceGuard::leased(self.clone(), slot, epoch, ticket);
    Ok(guard)
}
// Guard `Deref` = self.topology.slot_instance(&slot).
// On guard drop, scheduled on the ReleaseQueue, the closure captures
// `Arc<ManagedResource<R>>` (giving store + topology + resource):
//   topology.on_release(&mut slot)? ;
//   if topology.pools() && kept { store.return_slot(slot, epoch).await }   // FENCE: under-lock epoch re-read
//   else { resource.destroy(topology.into_instance(slot)).await }
```

**Resident** (review fix 🟡): `Slot = R::Instance` (the cloned shared handle the guard holds), `create_slot` clones the master handle, `slot_instance`/`into_instance` are identity, `pools() = false` (released clone is dropped, never pooled), `accept`/`prepare` default. The framework idle store stays empty (nothing is ever `return_slot`-ed), so every acquire `create_slot`s a fresh clone — correct clone-on-acquire, zero pooled machinery. Resident keeps its **own** master-handle cell (`ArcSwap`) inside `Resident<R>`, separate from the empty framework store; `dispatch_credential_hook` rebuilds that cell.

**Warmup / maintenance** (review fix 🟠): both are **framework-driven over `self.store`** — warmup loops `create_slot` → `store.return_slot` (fenced) `warmup_target` times; the reaper runs `store.evict_stale()` (revoke) + `store.retain(|s| !topology.idle_evictable(s))` (fingerprint/lifetime/idle-timeout), destroying evicted slots via `into_instance`. No hook needs `&store`; the framework holds it.

---

## Tasks

### Task 1 — make `Pooled<R>::Slot` real (`PoolSlot<R>`), store framework-owned

**Files:** `crates/resource/src/runtime/pool.rs`, `crates/resource/src/runtime/managed.rs`, `crates/resource/src/topology/contract.rs`.

- [ ] **1a.** Change `impl Topology for Pooled<R>` from `type Slot = ()` to `type Slot = PoolSlot<R>` (the `PoolSlot { instance, metrics, fingerprint, returned_at }` the convergence already defined). `Pooled<R>` no longer holds its own `InstanceStore<PoolSlot<R>>` — it keeps only the `semaphore`, `create_semaphore`, `config`, `current_fingerprint`; the idle store now lives in `ManagedResource.store` (`InstanceStore<PoolSlot<R>>`), reached as the `&store` argument.
- [ ] **1b.** `Resident<R>::Slot = R::Instance` (the cloned shared handle the guard holds; `pools()=false` so it never enters the store). `ManagedResource.store` for a resident is `InstanceStore<R::Instance>` and stays empty (never `return_slot`-ed). Resident keeps its master-handle `ArcSwap` cell inside `Resident<R>`, separate from that empty store.
- [ ] **1c.** Build: `cargo check -p nebula-resource --lib`. Expect errors at the bridge + loop sites — fixed in Tasks 2-3. Commit only when Task 3 compiles.

### Task 2 — replace `TopologyDispatch<R>` with `TopologyBind<R>` (thin R-aware hooks)

**Files:** `crates/resource/src/runtime/managed.rs` (trait def), `crates/resource/src/runtime/pool.rs` + `resident.rs` (impls).

- [ ] **2a.** Define `TopologyBind<R>` per the contract above (the rename signals the contract change; delete `acquire_guard`/`TopologyDispatch`). `accept` returns `bool` (no `Accept` enum). Re-export from `lib.rs`.
- [ ] **2b.** `impl TopologyBind<R> for Pooled<R>`: map the *existing* inherent pool logic into hooks — `create_slot` = `create_entry` (`Provider::create` + build `PoolSlot { instance, metrics: now, fingerprint }`); `slot_instance` = `&slot.instance`; `into_instance` = `slot.instance`; `accept(&mut slot)` = the `try_acquire_idle` checks (stale-fingerprint / max-lifetime / `is_broken` / `test_on_checkout` → `false`, else `true`); `prepare(&mut slot)` = `Provider::prepare(&slot.instance)`; `pools()` = `true`; `warmup_target` = `config.min_size`; `idle_evictable` = `should_evict` minus the revoke arm (framework owns revoke via `store.evict_stale`); `maintenance_schedule` / `dispatch_credential_hook` (walks `store.lock_idle()`) / `set_fingerprint` as today. `bump_revoke_epoch` moves to the framework store (`ManagedResource::bump_revoke_epoch` → `store.bump_revoke_epoch`).
- [ ] **2c.** `impl TopologyBind<R> for Resident<R>`: `Slot = R::Instance`; `create_slot` = clone the master shared handle (build it on first acquire if the cell is empty); `slot_instance` / `into_instance` = identity; `pools()` = `false` (released clone dropped, never pooled); `accept` / `prepare` / `warmup_target` / `idle_evictable` default. Resident keeps its master-handle `ArcSwap` cell internally; `dispatch_credential_hook` rebuilds that cell. The framework idle store stays empty.
- [ ] **2e.** **Shared-topology revoke footgun guard (🟠 observability/DoD).** Multiplexed/shared custom topologies (`pools() == false` holding a credential-bearing singleton — a gRPC channel, a WebSocket) are **not** in the framework store, so the revoke-epoch fence cannot evict them; their revoke teardown runs through `dispatch_credential_hook`, which **defaults to no-op**. At `Manager::register`, when `topology.pools() == false` AND the resource declares ≥1 credential slot (`HasCredentialSlots` non-empty), emit a `tracing::warn` + `debug_assert!` that the topology MUST override `dispatch_credential_hook` to tear down on revoke (a no-op leaks streams on a revoked credential). This is the one place a careful author still matters — make it loud, not silent. (Built-in `Resident` overrides the hook; the guard fires only for under-built customs.)
- [ ] **2d.** `cargo check -p nebula-resource --lib`.

### Task 3 — framework owns the acquire loop + release (the safety core)

**Files:** `crates/resource/src/registry.rs` (blanket `ManagedHandle::acquire`), `crates/resource/src/manager/acquire.rs`, `crates/resource/src/runtime/managed.rs`, `crates/resource/src/guard.rs`.

- [ ] **3a.** Rewrite the blanket `ManagedHandle::acquire` (inside `Manager::run_acquire`'s dispatch closure, which keeps the resilience-gate + drain bookkeeping + post-taint re-check) as the framework loop above: `try_reserve` → `store.checkout()` → **framework** destroys `checkout.stale` via `destroy(into_instance(stale))` → `accept(&mut slot)`-or-`create_slot` → **`CreateGuard`-wrap the slot (cancel-safety)** → `prepare(&mut slot)` → build guard. `&store` = `self.store` (real `InstanceStore<PoolSlot<R>>` for pool; empty `InstanceStore<R::Instance>` for resident). Bound `R::Topology: TopologyBind<R>`.
- [ ] **3b.** Cancel-safety (🟠 review fix): reuse/generalize the existing `CreateGuard` so a drop between `create_slot`/checkout and the built guard schedules `destroy(into_instance(slot))` on the `ReleaseQueue` — the `cancel-drop` regression test guards this; do not regress it.
- [ ] **3c.** Release path: `ResourceGuard<R>` drop schedules (via `ReleaseQueue`, current best-effort contract — **do not** upgrade to Phase E) `topology.on_release(&mut slot)` (recycle decision), then **if `topology.pools()` and kept** `store.return_slot(slot, epoch)` — the **same** slot, metadata intact; **else** `destroy(into_instance(slot))`. The framework runs the fence, not the topology. Preserve the verified C3 atomicity: `on_release`/recycle → `return_slot` last; `return_slot`'s under-lock epoch re-read is the fence; rotation walks `store.lock_idle()`.
- [ ] **3d.** `ResourceGuard<R>` holds the `Slot` + `checkout_epoch` + `Arc<ManagedResource<R>>` (store + topology + resource for the release closure). `Deref → R::Instance` via `self.topology.slot_instance(&slot)`. Adapt `guard.rs`; the release closure is the **only** site that returns-or-destroys a slot.
- [ ] **3e.** Resolve the now-dead open `Topology::acquire`/`on_release` **and evaluate merging `Topology` + `TopologyBind<R>` into one `Topology<R>` trait**: every topology needs the R-aware `create_slot`, so the R-agnostic split buys no reuse — one `Topology<R>` is better author DX (a single `impl` per custom topology). Options: (i) keep two traits but make the open methods live (framework calls `Topology::acquire` as the gate→ticket→slot step + `Topology::on_release` for reset), or (ii) **merge into `Topology<R>`** (gate + admission + slot lifecycle in one trait), which also removes any dead-method question. Spec 2 reads admission through `ManagedHandle` (already R-erased), so merging does not leak `R` into the scheduler seam. Prefer (ii) unless it complicates the `ManagedHandle` erasure; state the choice + rationale in the commit. **No method may be dead.**
- [ ] **3f.** Gates: `cargo check/clippy/nextest -p nebula-resource` (bare). **345-floor holds with unchanged intent** — especially `revoke_recycle_toctou::*`, `resident_rotation_race::*`, `shutdown_race::*` (they prove the fence still runs and the re-seat preserved atomicity). Commit milestone.

### Task 4 — rewrite the custom-topology proof to be safe-by-construction

**Files:** `crates/resource/tests/custom_topology_manager.rs`.

- [ ] **4a.** Rewrite `FfmpegPool` to the new shape: `impl Topology` (try_reserve / phase / load) + `impl TopologyBind<Ffmpeg>` with `create_slot` (spawn/allocate a `Transcoder` slot) + `slot_instance`/`into_instance` projections (+ `pools()` if it pools). **It must NOT own an `InstanceStore` field and MUST NOT contain any `resource.destroy` / `store.checkout` / stale-handling code** — the framework owns the store, checkout, fence, and destroy. If a reviewer finds author code touching the store or the fence, the fix failed.
- [ ] **4b.** Keep both C8 assertions: (1) registers + acquires through `Manager::register`/`acquire_any` reporting `TopologyTag::Custom`; (2) **a slot idle before a revoke is evicted on the next acquire WITHOUT any author fence code** — i.e. bump the epoch via `Manager`/`ManagedHandle::bump_revoke_epoch`, then assert the next acquire does not hand out the stale slot and the framework destroyed it. This is the safety-by-construction proof the convergence lacked.
- [ ] **4c.** Add an assertion or doc-comment that `FfmpegPool` holds no store and no destroy logic (the structural proof). The 321-line block shrinks well under the intent-gate cap; if still large, decompose, do not `budget-justify`.

### Task 5 — InstanceStore-rule regression test + engine/examples + gates

**Files:** `crates/resource/tests/`, engine/examples if signatures shifted, docs.

- [ ] **5a.** Confirm a custom `TopologyBind` impl receives only `&InstanceStore<Slot>` (borrowed, non-retainable) in the hooks it touches — it has no owned-store field. (Compile-shape is the proof; no `'static` store handle is reachable.)
- [ ] **5b.** Reverse-dep gate: `cargo check -p nebula-engine -p nebula-api -p nebula-examples --tests`; fix any signature ripple from the bridge rename (`TopologyDispatch` → `TopologyBind`). Rustdoc `-D warnings` on resource.
- [ ] **5c.** Full per-crate gates (resource, resource-macros, engine, api, examples), `cargo fmt -p` each. Update `crates/resource/README.md` + `CLAUDE.md` + the Phase C plan note to the inverted contract (the open trait is the gate + admission; `TopologyBind` is the framework-driven instance lifecycle; the fence is framework-owned for all topologies).
- [ ] **5d.** Commit. Final report: `Topology::acquire` not dead (or removed, no dead method); custom topology owns no store + no fence code; 345-floor + C8 safety proof green; `git grep` shows no `TopologyDispatch`/`acquire_guard` survivors.

---

## Forward-compatibility & scaling (read before implementing — defines the boundaries)

This contract was stress-tested against the future custom-topology zoo. The boundaries the implementer must respect so future shapes are not foreclosed:

- **Exclusively-leased family — safe-by-construction, the design's sweet spot.** Pooled connections, SSH sessions, browser/Playwright pages, FFmpeg permits: `Slot` carries the unit, `pools() == true`, the framework store-fence handles revoke; the author writes zero fence/store/destroy code. This is the bar the C8 test pins.
- **Multiplexed/shared family — expressible, revoke is intrinsically hook-driven.** gRPC N-streams over one channel, a persistent WebSocket, an SSH master connection: the instance is held continuously (not checkout-and-return), so it cannot live in the idle store and the store-fence cannot reach it. Revoke teardown therefore runs through the framework-DRIVEN `dispatch_credential_hook` (the rotation fan-out calls it on every revoke), author-IMPLEMENTED. This is inherent to multiplexing, not a design flaw — but the no-op default is a footgun, guarded by Task 2e. Do **not** try to force shared instances through the exclusive store; it does not fit.
- **Affinity/stateful-session family — deferred, explicitly not foreclosed.** Per-conversation AI-agent sessions, sticky keyed sessions: need "check out THE slot for key K", which `store.checkout()` (FIFO) does not do. This is A10 / Phase G. The contract leaves room: a future `InstanceStore::checkout_keyed(key)` + an affinity hook on `TopologyBind` add it without reshaping the loop. Keep `checkout` and the loop factored so a keyed variant slots in beside it — do not hard-code FIFO assumptions into the bridge.
- **Generic reusable topologies work:** `impl<R: Bound> TopologyBind<R> for SemaphorePool<R>` is valid — a topology kind authored once, used across many resources.
- **The framework loop is the one rigidity:** it is `gate → checkout → accept/create → prepare → wrap`. Arbitrary acquire logic goes in `create_slot` (`pools()==false` + any I/O). Multi-slot atomic acquire (ResourceGroup) composes *above* the topology, not inside it — out of scope here.

## Acceptance (the review gate)

1. A custom topology is registrable + acquirable through `Manager` (kept from convergence).
2. A custom topology author writes **zero** fence / store / destroy code, yet a slot idle before a credential revoke is evicted (Task 4b proves it). **Safety-by-construction restored.**
3. No `acquire_guard` returning `ResourceGuard<R>` in author/bridge code; the framework owns the loop. `git grep "acquire_guard\|TopologyDispatch"` empty.
4. Built-ins unchanged in behavior: `revoke_recycle_toctou`/`resident_rotation_race`/`shutdown_race` green with unchanged intent; 345-floor holds.
5. The open `Topology` trait has no dead method (Task 3d resolved).

## Out of scope (do not pull forward)
Phase E (async/fallible/ordered/poison release — keep best-effort `ReleaseQueue`), Phase F (`Bounded`), Phase D redesign (admission already wired). This fix is *only* the bind-contract inversion.
