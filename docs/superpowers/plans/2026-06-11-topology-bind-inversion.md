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

## The new contract (target)

`Topology::Slot` becomes **real** (carries the leasable unit), not `()` for storing topologies. The framework store `ManagedResource.store: InstanceStore<<R::Topology as Topology>::Slot>` becomes the *actual* idle store the framework fences — no separate topology-owned store.

```rust
// crates/resource/src/runtime/managed.rs — replaces TopologyDispatch<R>
/// R-aware policy hooks the framework calls *inside* its own acquire loop.
/// The framework owns try_reserve, the fenced checkout, stale-destroy, the
/// store, guard-wrapping, and on-release return — so a bind impl CANNOT skip
/// the revoke fence or retain the store. The open `Topology` (R-agnostic)
/// supplies the concurrency gate + admission surface; this supplies the
/// instance lifecycle keyed to `R`.
#[async_trait]
pub trait TopologyBind<R: Provider>: Topology {
    /// Make one fresh, already-credential-resolved instance. The framework
    /// drives this only when no fresh idle slot is available; credentials are
    /// resolved into the resource's slot cells before this runs (§2.6).
    async fn create(&self, resource: &R, config: &R::Config, ctx: &ResourceContext)
        -> Result<R::Instance, Error>;

    /// Wrap an instance into a storable `Slot` for the framework idle store,
    /// or `None` to never store (Resident / pure-permit). Pooled wraps
    /// `PoolSlot { instance, metrics, fingerprint, returned_at }`.
    fn into_slot(&self, instance: R::Instance) -> Option<Self::Slot> { let _ = instance; None }

    /// Consume a `Slot` back into its instance (for guard-wrapping after a
    /// fresh checkout, and for `Provider::destroy` on a stale/evicted slot).
    fn slot_into_instance(&self, slot: Self::Slot) -> R::Instance;

    /// Decide whether a checked-out idle slot is still usable (Pooled:
    /// fingerprint / max-lifetime / is_broken / test_on_checkout). `Discard`
    /// makes the framework destroy it and loop to the next idle / create.
    /// Default `Use` (no post-checkout policy).
    async fn accept(&self, instance: R::Instance, _resource: &R, _ctx: &ResourceContext)
        -> Accept<R::Instance> { Accept::Use(instance) }

    /// Per-acquire session-init on the instance about to be leased (Pooled
    /// `prepare`; `SET search_path`, etc.). Err => framework destroys + fails.
    async fn prepare(&self, _instance: &R::Instance, _ctx: &ResourceContext)
        -> Result<(), Error> { Ok(()) }

    // maintenance / rotation hooks (unchanged from TopologyDispatch, still
    // framework-driven, default no-op): warmup, run_maintenance,
    // maintenance_schedule, dispatch_credential_hook, set_fingerprint.
}

pub enum Accept<I> { Use(I), Discard }
```

```text
// crates/resource/src/registry.rs (or a Manager helper) — the ONE framework loop.
// blanket impl<R> ManagedHandle for ManagedResource<R> where R::Topology: TopologyBind<R>
async fn acquire(self, mgr, ctx, opts) -> ResourceGuard<R> {
    let ticket = self.topology.try_reserve(&self.store)?;        // gate (sync)
    loop {
        let checkout = self.store.checkout().await;              // FRAMEWORK fences on pop
        for stale in checkout.stale {                            // FRAMEWORK destroys stale — author cannot skip
            let _ = self.resource.destroy(self.topology.slot_into_instance(stale)).await;
        }
        let (instance, slot_epoch) = match checkout.fresh {
            Some(co) => {
                let (slot, epoch) = co.into_parts();
                let inst = self.topology.slot_into_instance(slot);
                match self.topology.accept(inst, &self.resource, &ctx).await {
                    Accept::Use(i) => (i, epoch),
                    Accept::Discard => { self.resource.destroy(/* i was moved; accept returns nothing on Discard — see note */).await; continue }
                }
            }
            None => (self.topology.create(&self.resource, self.config(), &ctx).await?, self.store.stamp_epoch()),
        };
        self.topology.prepare(&instance, &ctx).await?;           // FRAMEWORK runs session-init
        return Ok(ResourceGuard::pooled(instance, slot_epoch, ticket, /* return-to-store closure */));
    }
}
// On guard drop: topology.on_release(&mut slot)?  -> store.return_slot(into_slot(instance), epoch)  (FRAMEWORK, fenced)
```

> **Implementer note on `Accept::Discard`:** `accept` must give the framework the instance back to destroy on `Discard`. Use `enum Accept<I> { Use(I), Discard(I) }` (Discard carries the rejected instance) so the framework destroys it without a second projection. Adjust the loop accordingly.

`Resident<R>`: `Slot = ()`, `into_slot` returns `None` (never stores), `create` clones the shared handle, `accept`/`prepare` default. The framework loop's `checkout` is always empty (no idle), so it always `create`s — correct clone-on-acquire, zero pooled machinery.

---

## Tasks

### Task 1 — make `Pooled<R>::Slot` real (`PoolSlot<R>`), store framework-owned

**Files:** `crates/resource/src/runtime/pool.rs`, `crates/resource/src/runtime/managed.rs`, `crates/resource/src/topology/contract.rs`.

- [ ] **1a.** Change `impl Topology for Pooled<R>` from `type Slot = ()` to `type Slot = PoolSlot<R>` (the `PoolSlot { instance, metrics, fingerprint, returned_at }` the convergence already defined). `Pooled<R>` no longer holds its own `InstanceStore<PoolSlot<R>>` — it keeps only the `semaphore`, `create_semaphore`, `config`, `current_fingerprint`; the idle store now lives in `ManagedResource.store` (`InstanceStore<PoolSlot<R>>`), reached as the `&store` argument.
- [ ] **1b.** `Resident<R>::Slot` stays `()` (honest — no idle store). Confirm `ManagedResource.store` for a resident is `InstanceStore<()>` and is never pushed to.
- [ ] **1c.** Build: `cargo check -p nebula-resource --lib`. Expect errors at the bridge + loop sites — fixed in Tasks 2-3. Commit only when Task 3 compiles.

### Task 2 — replace `TopologyDispatch<R>` with `TopologyBind<R>` (thin R-aware hooks)

**Files:** `crates/resource/src/runtime/managed.rs` (trait def), `crates/resource/src/runtime/pool.rs` + `resident.rs` (impls).

- [ ] **2a.** Define `TopologyBind<R>` + `Accept<I>` per the contract above (rename signals the contract change; delete `acquire_guard`/`TopologyDispatch`). Re-export from `lib.rs`.
- [ ] **2b.** `impl TopologyBind<R> for Pooled<R>`: map the *existing* inherent pool logic into hooks — `create` = `create_entry`'s `Provider::create` path; `into_slot` = build `PoolSlot` with metrics/fingerprint; `slot_into_instance` = `slot.instance`; `accept` = the `try_acquire_idle` checks (stale-fingerprint / max-lifetime / `is_broken` / `test_on_checkout` → `Accept::Discard`, else `Accept::Use`); `prepare` = `Provider::prepare`. Keep `warmup`/`run_maintenance`/`maintenance_schedule`/`dispatch_credential_hook`/`set_fingerprint`/`bump_revoke_epoch` (now operating over `&store` passed by the framework, or over `Pooled`'s own counters as today for fingerprint).
- [ ] **2c.** `impl TopologyBind<R> for Resident<R>`: `create` = clone/create the shared handle; `into_slot` = `None`; `slot_into_instance` = unreachable for `Slot=()` (document — resident never stores, so the framework never calls it; provide a body that cannot be reached, justified).
- [ ] **2d.** `cargo check -p nebula-resource --lib`.

### Task 3 — framework owns the acquire loop + release (the safety core)

**Files:** `crates/resource/src/registry.rs` (blanket `ManagedHandle::acquire`), `crates/resource/src/manager/acquire.rs`, `crates/resource/src/runtime/managed.rs`, `crates/resource/src/guard.rs`.

- [ ] **3a.** Rewrite the blanket `ManagedHandle::acquire` (and/or the `Manager::run_acquire` dispatch closure) as the framework loop above: `try_reserve` → `store.checkout()` → **framework** destroys `checkout.stale` via `Provider::destroy(slot_into_instance(...))` → `accept`-or-`create` → `prepare` → wrap. The `&store` is `self.store` (the real `InstanceStore<PoolSlot<R>>` for pool). Bound `R::Topology: TopologyBind<R>`.
- [ ] **3b.** Release path: the `ResourceGuard<R>` drop schedules (via `ReleaseQueue`, current best-effort contract — **do not** upgrade to Phase E semantics) `topology.on_release(&mut slot)` then `store.return_slot(into_slot(instance), epoch)` — the framework runs the fence, not the topology. Preserve the verified C3 atomicity: `recycle`/policy → `return_slot` last, `return_slot`'s under-lock epoch re-read is the fence; rotation walks `store.lock_idle()`.
- [ ] **3c.** `ResourceGuard<R>` carries what release needs (the instance + the `checkout_epoch` + the bind handle to call `into_slot`/`on_release`). Adapt `guard.rs` minimally; keep `Deref → R::Instance`.
- [ ] **3d.** Delete the now-dead open `Topology::acquire`? Decide: either (i) keep `Topology::acquire` as the R-agnostic "ticket → Lease<Slot>" the framework calls for the *gate-to-slot* step, or (ii) remove it from the trait if the framework loop fully subsumes it. Pick whichever leaves no dead method. State the choice in the commit.
- [ ] **3e.** Gates: `cargo check/clippy/nextest -p nebula-resource` (bare). **345-floor holds with unchanged intent** — especially `revoke_recycle_toctou::*`, `resident_rotation_race::*`, `shutdown_race::*` (these prove the fence still runs and the re-seat preserved atomicity). Commit milestone.

### Task 4 — rewrite the custom-topology proof to be safe-by-construction

**Files:** `crates/resource/tests/custom_topology_manager.rs`.

- [ ] **4a.** Rewrite `FfmpegPool` to the new shape: `impl Topology` (try_reserve / phase / load) + `impl TopologyBind<Ffmpeg>` with only `create` (spawn/allocate a `Transcoder`) and, if it stores, `into_slot`/`slot_into_instance`. **It must NOT own an `InstanceStore` field and MUST NOT contain any `resource.destroy`/stale-handling code** — the framework owns that. If a reviewer can find author code touching the fence, the fix failed.
- [ ] **4b.** Keep both C8 assertions: (1) registers + acquires through `Manager::register`/`acquire_any` reporting `TopologyTag::Custom`; (2) **a slot idle before a revoke is evicted on the next acquire WITHOUT any author fence code** — i.e. bump the epoch via `Manager`/`ManagedHandle::bump_revoke_epoch`, then assert the next acquire does not hand out the stale slot and the framework destroyed it. This is the safety-by-construction proof the convergence lacked.
- [ ] **4c.** Add an assertion or doc-comment that `FfmpegPool` holds no store and no destroy logic (the structural proof). The 321-line block shrinks well under the intent-gate cap; if still large, decompose, do not `budget-justify`.

### Task 5 — InstanceStore-rule regression test + engine/examples + gates

**Files:** `crates/resource/tests/`, engine/examples if signatures shifted, docs.

- [ ] **5a.** Confirm a custom `TopologyBind` impl receives only `&InstanceStore<Slot>` (borrowed, non-retainable) in the hooks it touches — it has no owned-store field. (Compile-shape is the proof; no `'static` store handle is reachable.)
- [ ] **5b.** Reverse-dep gate: `cargo check -p nebula-engine -p nebula-api -p nebula-examples --tests`; fix any signature ripple from the bridge rename (`TopologyDispatch` → `TopologyBind`). Rustdoc `-D warnings` on resource.
- [ ] **5c.** Full per-crate gates (resource, resource-macros, engine, api, examples), `cargo fmt -p` each. Update `crates/resource/README.md` + `CLAUDE.md` + the Phase C plan note to the inverted contract (the open trait is the gate + admission; `TopologyBind` is the framework-driven instance lifecycle; the fence is framework-owned for all topologies).
- [ ] **5d.** Commit. Final report: `Topology::acquire` not dead (or removed, no dead method); custom topology owns no store + no fence code; 345-floor + C8 safety proof green; `git grep` shows no `TopologyDispatch`/`acquire_guard` survivors.

---

## Acceptance (the review gate)

1. A custom topology is registrable + acquirable through `Manager` (kept from convergence).
2. A custom topology author writes **zero** fence / store / destroy code, yet a slot idle before a credential revoke is evicted (Task 4b proves it). **Safety-by-construction restored.**
3. No `acquire_guard` returning `ResourceGuard<R>` in author/bridge code; the framework owns the loop. `git grep "acquire_guard\|TopologyDispatch"` empty.
4. Built-ins unchanged in behavior: `revoke_recycle_toctou`/`resident_rotation_race`/`shutdown_race` green with unchanged intent; 345-floor holds.
5. The open `Topology` trait has no dead method (Task 3d resolved).

## Out of scope (do not pull forward)
Phase E (async/fallible/ordered/poison release — keep best-effort `ReleaseQueue`), Phase F (`Bounded`), Phase D redesign (admission already wired). This fix is *only* the bind-contract inversion.
