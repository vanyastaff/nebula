# Open, Engine-Managed Resource Topology — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn `nebula-resource`'s closed topology enum into an open, engine-managed, `#[async_trait]` contract — authors ship any client shape (gRPC streams, SSH sessions, FFmpeg pools) without modifying the crate — while restoring `Bounded`, adding a TOCTOU-free admission surface, and making `release`/`destroy`/`check` real async work, all with the credential/tenant moat preserved by construction.

**Architecture:** The `Manager` owns instance storage (`InstanceStore<Slot>`) and every credential/tenant/drain/recovery invariant; an open `Topology` trait owns only lease/concurrency policy over storage it borrows but cannot retain. Type-erasure happens once at a renamed `ManagedHandle` boundary (was `AnyManagedResource`), so `Topology` is dispatched monomorphically and needs no erased twin. Admission is ticket-based `try_reserve` (gate) plus advisory `phase()`/`load()` (route/diagnose).

**Tech Stack:** Rust edition 2024 (~1.96), Tokio, `async-trait`, `arc-swap`, `thiserror`; `cargo nextest`, trybuild, lefthook per-crate gates. `#![forbid(unsafe_code)]`.

**Spec:** `docs/superpowers/specs/2026-06-11-resource-topology-design.md` (read §0 vocabulary, §2 contract, §2.6 credentials, §2.7 async/rename, §3 admission, §5 scope, §7 migration). Code sketches in the spec are the **target API** for structural tasks — reproduce them, adapting to the real internals you read.

---

## How to execute this plan

This is **eight phases (A–H), each a self-contained PR** that leaves the whole workspace green. Do not start a phase until the previous one is merged-green. Per-phase discipline:

- **Branch is `dreamy-kare-8698d4`** (4 breaking commits already landed: Bounded deleted, `Resource`=2 assoc types, `ResourceSlots` derive, `ResourceConfig` fingerprint). Commit at each per-crate-green point.
- **Gates per touched crate** (Windows worktree — see pitfalls):
  - `cargo check -p <crate> --tests`
  - `cargo clippy -p <crate> --all-targets` (warnings = errors)
  - `cargo nextest run -p <crate>`
  - `cargo test -p <crate> --doc`
  - `cargo fmt -p <crate>` (NEVER `cargo fmt --all` — os error 206 in deep worktree paths)
  - For trybuild/macros: plain `cargo test -p nebula-resource --test derive_resource_compile_fail` on a **warm** cache; **never** `TRYBUILD=overwrite` on a timeout.
  - Rustdoc gate (CI parity, not in lefthook): `RUSTDOCFLAGS="-D warnings" cargo doc -p <crate> --no-deps`.
- **Never `cd` into the worktree** in a script; use `git -C` + absolute paths (Stop/intent-gate count worktree untracked files via session cwd).
- **No shims / no compat layers.** Hard breaks are fine pre-1.0 — delete the old thing, migrate every call site in the same PR.
- **Observability = Definition of Done:** any new state/error/hot path ships a typed `thiserror` variant + a `tracing` span + an invariant `debug_assert!` in the same task.
- **`#![forbid(unsafe_code)]`, `#![warn(missing_docs)]` stay.** No `unwrap`/`expect`/`panic!`/`todo!` in lib code (tests exempt). Emitted macro code counts as lib code.

**Phase order and what each PR delivers:**

| Phase | PR delivers | Depends on |
|---|---|---|
| A | Vocabulary rename: `Resource`(trait)→`Provider`, `Runtime`→`Instance`, `#[derive(ResourceSlots)]`→`#[derive(Resource)]` | — |
| B | `#[async_trait]` crate-wide + erasure rename (`AnyManagedResource`→`ManagedHandle`, …) + delete `ErasedAcquireFn` | A |
| C | `InstanceStore<Slot>` + open `Topology` `#[async_trait]` trait; revoke-fence uniform over the store | B |
| D | Admission surface: `AdmissionPhase`/`Ticket`/`Unavailable`/`Load`/`CheckCost` + `try_reserve` gate | C |
| E | Async/fallible lifecycle: `on_release` (A5), `destroy(timeout)` (A6), per-acquire session-init (A4) | C |
| F | Restore `Bounded` as a built-in `Topology` (runtime cap, `Capped`/`Exclusive`/`Unbounded`) | C, D, E |
| G | A10 dedup affinity modes, A11 cost-aware `check`, A12 parent-generation recovery | E |
| H | Canon §11.4 ADR + mirror async-trait override into path-standard + memory; README/CLAUDE.md sweep | A–G |

Phases C–G are large; each may split into sub-PRs at a per-crate-green point if review pressure demands, but the phase boundary is the natural PR seam.

---

## Phase A — Vocabulary rename (mechanical, zero behavior change)

**Why:** every later phase writes the final names; do the rename first so nothing churns twice. Pure rename — no logic changes, all tests pass unchanged in intent.

**Renames (exact):**

| Old | New | Kind |
|---|---|---|
| `trait Resource` | `trait Provider` | trait (crates/resource/src/resource.rs) |
| `Resource::Runtime` (assoc type) | `Provider::Instance` | assoc type |
| `R::Runtime` everywhere | `R::Instance` | usage |
| `#[derive(ResourceSlots)]` | `#[derive(Resource)]` | derive (macros) |
| `nebula_resource_macros::ResourceSlots` | `nebula_resource_macros::Resource` | proc-macro export |

**Keep unchanged:** `ResourceConfig`, `ResourceContext`, `ResourceGuard`, `ResourceMetadata`, `ResourceKey`, `ResourceEvent`, `HasCredentialSlots`, `CredentialSlot`, the `nebula-resource` crate name, `AnyResource` (renamed in Phase B), the `#[resource(...)]`/`#[credential(...)]` attribute names.

**Files (authoritative list — regenerate before starting):**

- [ ] **Step A1: Inventory the rename surface.**

Run (from repo root, bare):
```bash
git -C . grep -l "trait Resource\b\|impl Resource for\|: Resource\b\|R::Runtime\|type Runtime\|ResourceSlots\|derive(Resource" -- 'crates/**/*.rs' 'examples/**/*.rs' > /tmp/rename-files.txt
wc -l /tmp/rename-files.txt
```
Expected: ~55 files. Paste the list into the PR description.

- [ ] **Step A2: Rename the trait + assoc type in the source of truth.**

Modify `crates/resource/src/resource.rs`:
- `pub trait Resource` → `pub trait Provider` (keep supertrait bounds, all method signatures).
- `type Runtime: Send + Sync + 'static;` → `type Instance: Send + Sync + 'static;`
- Every `Self::Runtime` → `Self::Instance` in this file (create/check/shutdown/destroy/on_credential_refresh/on_credential_revoke return types and params).
- Update the rustdoc table ("Runtime | the live resource handle" → "Instance | the live resource handle") and the trait-level doc.

- [ ] **Step A3: Rename in the macros crate.**

Modify `crates/resource/macros/src/lib.rs`:
- `#[proc_macro_derive(ResourceSlots, attributes(credential))]` → `#[proc_macro_derive(Resource, attributes(credential))]`
- `pub fn derive_resource_slots` → `pub fn derive_resource` (and update the doc that says "Two-derive pattern" / examples to `#[derive(Resource)]`).
- In `crates/resource/macros/src/slots.rs` + `field_slots.rs`: update any doc/comment mentioning `ResourceSlots` or `Runtime` assoc. The emitted code references `HasCredentialSlots` (unchanged) — verify the emitted accessor doc says `Instance` where relevant.

Modify `crates/resource/src/lib.rs`:
- `pub use nebula_resource_macros::ResourceSlots;` → `pub use nebula_resource_macros::Resource;`
- Re-export block: `pub use resource::{... Resource ...}` — the **trait** export changes from `Resource` to `Provider` (add `Provider`, remove the trait `Resource` — but the derive is now also `Resource`; a derive macro and a trait can share the name in Rust, so `pub use nebula_resource_macros::Resource` (derive) + `pub use resource::Provider` (trait) coexist). Verify no `pub use resource::Resource` remains.
- Update the crate-doc key-types table: `Resource` trait row → `Provider`; `Runtime` mentions → `Instance`.

- [ ] **Step A4: Mechanical sweep across the workspace.**

For each file in `/tmp/rename-files.txt`, apply (review each — do NOT blind-sed, the word "Resource" appears in many kept names):
- `impl Resource for X` → `impl Provider for X`
- `: Resource +` / `R: Resource` bounds → `R: Provider`
- `type Runtime =` (inside a `Provider` impl) → `type Instance =`
- `R::Runtime` / `Self::Runtime` / `<X as Resource>::Runtime` → `…::Instance`
- `#[derive(ResourceSlots)]` → `#[derive(Resource)]`
- `use nebula_resource::{… Resource …}` where it imports the **trait** → import `Provider`; where it imports the **derive** keep `Resource`.

Touched crates (from inventory): `nebula-resource` (src + tests), `nebula-engine` (src + tests), `nebula-action` (resource_produces + any `impl Resource`), `nebula-sdk` (prelude re-export `Resource`→`Provider` trait + `Resource` derive), `nebula-plugin`, `nebula-examples`.

Note for `crates/sdk/src/prelude.rs`: `pub use nebula_resource::{Resource, ResourceMetadata};` — `Resource` here was the trait; becomes `pub use nebula_resource::{Provider, Resource, ResourceMetadata};` (trait `Provider` + derive `Resource`).

- [ ] **Step A5: Per-crate gates, commit per green crate.**

Run for each touched crate in dependency order (resource-macros, resource, then engine/action/sdk/plugin/examples):
```bash
cargo check -p nebula-resource-macros -p nebula-resource --tests
cargo clippy -p nebula-resource-macros -p nebula-resource --all-targets
cargo nextest run -p nebula-resource
cargo test -p nebula-resource --doc
cargo test -p nebula-resource --test derive_resource_compile_fail   # warm cache
cargo check -p nebula-engine -p nebula-action -p nebula-sdk -p nebula-plugin -p nebula-examples --tests
cargo nextest run -p nebula-engine
cargo fmt -p nebula-resource -p nebula-resource-macros -p nebula-engine
```
Expected: all green; test COUNTS identical to pre-rename (pure rename adds/removes no tests). Update trybuild `.stderr` fixtures if they printed the old trait/derive name (re-bless ONLY by hand-editing the expected text to the new name, never `TRYBUILD=overwrite`).

- [ ] **Step A6: Commit.**
```bash
git -C . add -A
git -C . commit -m "refactor(resource)!: rename Resource trait->Provider, Runtime->Instance, derive ResourceSlots->Resource"
```
(End the message body with the Co-Authored-By line.) BREAKING CHANGE noted in body.

**Acceptance:** workspace compiles; `git grep "type Runtime\|trait Resource\b\|ResourceSlots"` over `crates/`/`examples/` returns only unrelated hits (e.g. tokio `Runtime`); all pre-existing tests pass with unchanged counts.

---

## Phase B — `#[async_trait]` crate-wide + erasure rename

**Why:** kill the hand-written `Erased*`/`*Fn` boxed-future twins and the `Any*`/`Typed*` jargon; one async-dispatch rule. Spec §2.7. This is structural but behavior-preserving (same dispatch, different mechanism).

**Renames + deletions:**

| Old | New |
|---|---|
| `AnyManagedResource` (sealed trait, registry.rs) | `ManagedHandle` |
| `AnyResource` (metadata-only trait, resource.rs) | `ResourceDescriptor` |
| `ErasedAcquireFn` (`Arc<dyn Fn → BoxFuture<Box<dyn Any>>>`, registry.rs) | **deleted** → `ManagedHandle::acquire()` method |
| `erased_acquire_pooled_for::<R>()` / `_resident_for::<R>()` (acquire.rs) | **deleted** |
| `Manager::acquire_erased_for` | `Manager::acquire_any` |
| engine `ErasedResourceRegistrar` (registrar.rs) | `ResourceActivator` |
| engine `TypedResourceRegistrar<R>` | concrete per-kind impl of `ResourceActivator` (rename to `KindActivator<R>`, keep internal) |

- [ ] **Step B1: Add `async-trait` to the resource crate (if absent).**

Check `crates/resource/Cargo.toml` `[dependencies]` for `async-trait`. If missing, add `async-trait = { workspace = true }` (confirm it's in root `[workspace.dependencies]`; if not, add a pinned version there and stage root `Cargo.lock`). Run `cargo check -p nebula-resource`.

- [ ] **Step B2: Convert `Provider` to `#[async_trait]`.**

Modify `crates/resource/src/resource.rs`:
- Add `use async_trait::async_trait;`
- `#[async_trait]` on `pub trait Provider`.
- Convert each async lifecycle method from RPITIT (`fn create(&self, …) -> impl Future<Output = Result<…>> + Send`) to `async fn create(&self, …) -> Result<…>`. Default-bodied methods (`check`/`shutdown`/`destroy`/`on_credential_refresh`/`on_credential_revoke`) become `async fn … { Ok(()) }` etc.
- Keep `key()`, `metadata()`, `schema()` (if present), `credential_slot_epoch` location (on `HasCredentialSlots`, unchanged) as plain sync.
- Every workspace `impl Provider for X` gains `#[async_trait]` and its methods drop the `impl Future` return for `async fn`. Sweep all 55 impl sites.

- [ ] **Step B3: Rename `AnyManagedResource`→`ManagedHandle`, fold acquire into a method.**

Modify `crates/resource/src/registry.rs`:
- Rename the sealed trait `AnyManagedResource` → `ManagedHandle` (keep `sealed::Sealed` supertrait).
- Add `#[async_trait]` and a method `async fn acquire(self: Arc<Self>, mgr: Arc<Manager>, ctx: ResourceContext, opts: AcquireOptions) -> Result<Box<dyn Any + Send + Sync>, Error>;` — this replaces the stored `ErasedAcquireFn`. The blanket `impl<R: Provider + HasCredentialSlots> ManagedHandle for ManagedResource<R>` implements it by calling the (now monomorphic) per-topology acquire on `self` — the logic currently inside the `erased_acquire_*_for::<R>()` closures moves here.
- Delete `pub type ErasedAcquireFn = …`.
- Rename the existing erased methods to drop the `_erased` suffix where it reads cleaner (e.g. `phase_erased`→`phase`, `bump_revoke_epoch_erased`→`bump_revoke_epoch`) — optional polish; keep behavior. (If you rename, update all call sites.)

Modify `crates/resource/src/runtime/managed.rs` and `manager/acquire_dispatch.rs`: the per-topology acquire bodies (currently behind `erased_acquire_*`) become the `ManagedHandle::acquire` impl, dispatching on the stored `TopologyRuntime<R>` (Pool/Resident — Bounded re-added in Phase F). After Phase C this dispatches on the open `Topology`.

- [ ] **Step B4: Delete `erased_acquire_*_for`, rename `acquire_erased_for`→`acquire_any`.**

Modify `crates/resource/src/manager/acquire.rs` + `mod.rs`:
- Delete `erased_acquire_resident_for::<R>()` and `erased_acquire_pooled_for::<R>()` and the `RegistrationSpec.acquire` field that stored the closure (the acquire is now intrinsic to `ManagedHandle`). Update `RegistrationSpec`/`register_spec` to not take an `acquire` closure.
- Rename `Manager::acquire_erased_for(...)` → `Manager::acquire_any(...)`; it now resolves the row to `Arc<dyn ManagedHandle>` and calls `handle.acquire(...)`.
- Engine: update `crates/engine/src/resource_accessor.rs` (`acquire_erased_for`→`acquire_any`), `crates/engine/src/resource/registrar.rs` (drop the `FAcq` factory; the registrar no longer supplies an acquire closure), and every test/plugin reference from the Step-A inventory.

- [ ] **Step B5: Rename `AnyResource`→`ResourceDescriptor`, engine `ErasedResourceRegistrar`→`ResourceActivator`.**

- `crates/resource/src/resource.rs`: `AnyResource` → `ResourceDescriptor` (the metadata-only erased trait); update `lib.rs` re-export and all consumers (engine `engine.rs`, plugin crate, `resource_registrar_from_plugins` test).
- `crates/engine/src/resource/registrar.rs`: `ErasedResourceRegistrar` → `ResourceActivator`; `TypedResourceRegistrar<R>` → `KindActivator<R>`; update module doc + the registrar map type + all references.

- [ ] **Step B6: Gates + commit (per-crate green).**

Run the full per-crate gate set (B-touched crates: resource, resource-macros, engine, action, sdk, plugin, examples). Pay attention: `cargo nextest run -p nebula-engine` must stay green (it drives the registrar/accessor). Commit:
```bash
git -C . commit -m "refactor(resource)!: #[async_trait] crate-wide; rename erasure (ManagedHandle/ResourceDescriptor/ResourceActivator/acquire_any), delete ErasedAcquireFn"
```

**Acceptance:** `git grep "Erased\|AnyManagedResource\|AnyResource\b\|acquire_erased\|impl Future<Output" crates/resource/src` returns no hits (the boxed-future RPITIT signatures are gone); all tests green with unchanged counts; `dyn ManagedHandle` is the only resource erasure boundary.

---

## Phase C — Topology convergence via `Provider::Topology` assoc type (REWORK)

**Status of the original Phase C (commit `9667e821` + partial `fa0b4070`):** the *types* landed and are real — `InstanceStore<S>` (`topology/store.rs`, revoke-fenced on `return_slot`, 8 tests), the open `Topology` `#[async_trait]` trait (`topology/contract.rs`), `Ticket`/`Lease`/`Unavailable`, `AdmissionPhase`, and a standalone `tests/custom_topology.rs`. **But the convergence was faked, not built.** This rework finishes it. The deviations to close:

1. **`TopologyKind<R>` is still a closed `Pool|Resident` enum** with fn-pointer dispatch (`runtime/mod.rs`). There is **no path from `Manager::register()` to a custom `impl Topology`** — "open" is cosmetic; `custom_topology.rs` only exercises a topology *standalone*, never through the Manager.
2. **Built-in `impl Topology for PoolRuntime`** (`pool.rs:1387`) is a **façade**: its `acquire` is a 2-line permit no-op (real acquire is the inherent 7-arg `PoolRuntime::acquire` via `pooled_pipeline`), and every dispatch hands it a throwaway `InstanceStore::new(None)` it ignores (`runtime/mod.rs:199,212,224,278,290,302`).
3. **Two revoke fences coexist** — `PoolRuntime.revoke_epoch` + idle-queue eviction (live) and `InstanceStore.return_slot` (only `custom_topology.rs` uses it).
4. **`InstanceStore::checkout()` has a fence-on-checkout GAP (security).** It pops and hands out an idle slot **without** comparing its `checkout_epoch` to the live revoke counter — only `return_slot`/`evict_stale` fence. A slot that went idle at epoch 0, then had its credential revoked (live→1), is **served to the next acquirer** under a since-revoked credential. The built-in pool is safe today only because `try_acquire_idle` re-checks (`pool.rs:610`); the new `InstanceStore` path is **not**, so the spec's "uniform fence covers custom topologies too" (§2.6) is currently false on the checkout direction.

**Decision (spike-proven GREEN against production types — `cargo check -p nebula-resource --lib`):** topology is reached monomorphically via a new `Provider::Topology` **associated type**. `ManagedResource<R>` stays single-parameter (`InstanceStore<<R::Topology as Topology>::Slot>` needs no extra generic); one blanket `impl<R: Provider> ManagedHandle for ManagedResource<R>` dispatches `self.topology.{try_reserve,acquire}` directly; the entire `TopologyKind`/`TopologyRuntime`/`*Dispatch` fn-pointer table is **deleted**. The alternative (`ManagedResource<R, T>` second generic) was rejected: it threads `T` through all 22 `ManagedResource<R>` sites and **breaks `get_typed::<R>`** (the `TypeId` registry index would need `T` at lookup, callers only have `R`).

```rust
#[async_trait]
pub trait Provider: Send + Sync + 'static {
    type Config;
    type Instance: Send + Sync + 'static;
    type Topology: Topology;                       // ← NEW: topology pinned to the resource type
    // ...existing lifecycle (create/check/shutdown/destroy/on_credential_*)...
}
```

The `Topology` trait, `InstanceStore`, `Ticket`/`Lease`/`Unavailable`, `AdmissionPhase` already exist — keep them. This rework wires them and deletes the enum.

**Spike-confirmed facts the executor must honor:**
- Every `impl`/method touching the store re-states `<R::Topology as Topology>::Slot: Send + Sync + 'static` in its where-clause (the projection doesn't carry its own bounds). ~6-8 repeated clauses in the manager — annoying, not blocking. This is the accepted tax.
- `#[async_trait]` is **required** (not optional) — `ManagedHandle` must stay object-safe (`Arc<dyn ManagedHandle>`); native async-fn-in-trait can't give that. The blanket impl's where-clause must carry the `Slot: Send + Sync + 'static` bound or the boxed future isn't provably `Send`.
- Orphan rules are a non-issue: one blanket impl over crate-local `ManagedResource<R>`, sealed `ManagedHandle`. No per-topology impls.
- The fence-fix stays framework-owned and is *easier* here: `checkout` + fence + destroy live in `InstanceStore`/the blanket pipeline; `Topology::Slot` is opaque to the framework, so the author never sees the epoch.

**Scope guard:** this rework re-seats storage and routes release **through** the store fence, but keeps the *current* release contract (best-effort via `ReleaseQueue`). Making `on_release`/`destroy` fully async/fallible/ordered/poison-on-panic is **Phase E** — do not pull it forward. Pool *correctness policy* (fingerprint/lifetime/broken/health/prepare) is preserved verbatim, only re-homed over the checked-out slot.

- [ ] **Step C1: Fix the `InstanceStore::checkout()` fence-on-checkout gap (security).**

In `crates/resource/src/topology/store.rs`: `checkout()` must run the revoke fence on pop, framework-owned. Pop entries under the idle lock; an entry whose `checkout_epoch != live_revoke_epoch` is **stale** — it must NOT be handed out. Because the store cannot call `Provider::destroy` (it has no `Provider`), return the skipped stale slots to the caller for destruction. Change the signature to e.g.:
```rust
/// Pops the first fresh idle slot, discarding (and returning for destruction)
/// any slots whose checkout epoch is behind the live revoke counter.
pub async fn checkout(&self) -> Checkout<S>;   // { fresh: Option<CheckedOut<S>>, stale: Vec<S> }
```
The framework acquire pipeline (Step C5) destroys `stale` via `Provider::destroy`/`ReleaseQueue` before using `fresh`. Add unit tests: (a) idle slot revoked-while-idle is in `stale`, never `fresh`; (b) a mix returns only fresh ones, all stale collected; (c) the existing return-fence tests still pass. Update `evict_stale`/`drain_all` doc to state the fence now runs on *both* checkout and return.

- [ ] **Step C2: Add `Provider::Topology` associated type.**

In `crates/resource/src/resource.rs`: add `type Topology: Topology;` to `trait Provider`. The assoc type **cannot** be defaulted (no per-resource associated-type defaults in stable Rust), so every `impl Provider` must spell it (`type Topology = Pooled<Self>` / `= Resident<Self>` / `= TheirCustom`). Add a `fn build_topology(&self, config: &Self::Config) -> Self::Topology` (or carry the built topology through registration — see C6) so the framework can construct the topology instance at registration where config is known.

- [ ] **Step C3: `PoolSlot<R>` + built-in `Pooled<R>` framework topology; re-seat pool storage.**

In `crates/resource/src/runtime/pool.rs`:
- Define `pub struct PoolSlot<R: Provider> { instance: R::Instance, metrics: InstanceMetrics, fingerprint: u64, returned_at: Option<Instant> }`. **Note `revoke_epoch` leaves the slot** — it becomes the store's `checkout_epoch` (the store owns the fence now).
- Reframe the pool runtime as a framework topology TYPE `pub struct Pooled<R> { store: InstanceStore<PoolSlot<R>>, semaphore: Arc<Semaphore>, create_semaphore: Arc<Semaphore>, config: PoolConfig, current_fingerprint: Arc<AtomicU64> }` (drop the old `idle: VecDeque` and `revoke_epoch: AtomicU64` — both move into `store`). `impl Topology for Pooled<R>` with `type Slot = PoolSlot<R>`:
  - `try_reserve(&self, store)` — take a semaphore permit (non-blocking `try_acquire_owned`); on success `Ticket::permit(...)`. (Idle checkout is async → deferred to `acquire`/the framework pipeline, per the existing `Ticket` model.)
  - `acquire(&self, ticket, store)` — drive the existing `try_acquire_idle` logic over `store.checkout()` (now fenced — C1): inspect the checked-out `PoolSlot` for **pool policy** (stale fingerprint, max-lifetime, `is_broken`, `test_on_checkout`, `prepare`); a slot failing any → destroy + loop; empty → create via the existing `create_entry` funnel (gated on `create_semaphore`). The revoke-epoch re-check at the old `pool.rs:610` is **deleted** — `store.checkout()` already fenced it.
  - `on_release(&self, slot)` — the recycle decision (`RecycleDecision`); keep current best-effort contract (Phase E hardens). The framework returns the slot via `store.return_slot(slot, checkout_epoch)` (uniform fence).
  - `phase`/`load` — honest `Saturated`/`Load::permits` off the semaphore (already implemented on the façade; move them here).
- `bump_revoke_epoch` delegates to `store.bump_revoke_epoch()`. `run_maintenance` uses `store.evict_stale()` for the revoke arm + a new `store.retain(|s| !pool_should_evict_nonrevoke(s, fp, now)) -> Vec<S>` for fingerprint/lifetime/idle-timeout (add `retain` to `InstanceStore`). `dispatch_slot_hook_over_idle` (rotation fan-out) needs locked idle iteration with `&R::Instance` — add `pub(crate) async fn lock_idle(&self) -> MutexGuard<'_, VecDeque<StoreEntry<S>>>` to `InstanceStore` (crate-internal; authors only get `&InstanceStore`, never the guard, so the "cannot retain" rule holds). Preserve the head-of-line-blocking-but-correct rotation semantics verbatim (`pool.rs:336` doc).

- [ ] **Step C4: Built-in `Resident<R>` framework topology (store-less, honest).**

In `crates/resource/src/runtime/resident.rs`: reframe as `pub struct Resident<R> { /* shared-handle cell + config */ }`, `impl Topology for Resident<R>` with `type Slot = ()` (or the shared handle). `try_reserve` infallible (`Ticket::infallible()`), `acquire` clones the resident handle, `on_release` no-op, `phase` `Ready`, `load` `None`. Resident legitimately holds **no idle store** — document that the `&InstanceStore` arg is unused-by-design (like permit-only topologies), NOT a façade. No `InstanceStore::new(None)` dummy is passed per-call; the resident's (empty) store lives in `ManagedResource`.

- [ ] **Step C5: `ManagedResource<R>` holds `R::Topology`; one blanket `ManagedHandle` impl; delete the dispatch enum.**

- `crates/resource/src/runtime/managed.rs`: `ManagedResource<R>` drops `topology: TopologyRuntime<R>`, gains `topology: R::Topology` + `store: InstanceStore<<R::Topology as Topology>::Slot>`. Add the `<R::Topology as Topology>::Slot: Send + Sync + 'static` where-clause where needed.
- `crates/resource/src/registry.rs`: the blanket `impl ManagedHandle for ManagedResource<R>` (from Phase B) becomes the **single** dispatch: `acquire` runs the framework pipeline — `topology.try_reserve(&store)` (sync gate) → framework fenced-checkout + destroy-stale (C1) → `topology.acquire(ticket, &store)` → wrap `Lease` in `ResourceGuard` whose drop schedules `on_release` + `store.return_slot`. `admission_phase`/`admission_load`/`try_reserve_gate` call `topology.{phase,load,try_reserve}` directly. **Delete** `runtime/mod.rs` wholesale (`TopologyKind`, `TopologyRuntime`, `AcquireDispatch`, `AdmissionPhaseDispatch`, `TryReserveGateDispatch`, `LoadDispatch`, both constructors). Collapse `manager/acquire.rs` `pooled_pipeline`/`resident_pipeline`/`unexpected_topology` + the `match managed.topology.kind` arms in `manager/{registration,rotation,mod}.rs` and `runtime/managed.rs` into the one monomorphic path. Remove the dummy-store dispatch and the dead façade `impl Topology for PoolRuntime`.

- [ ] **Step C6: Registration carries `R::Topology` by value; derive emits `type Topology`.**

- `crates/resource/src/manager/{options,registration}.rs`: `RegistrationSpec<R>` drops `topology: TopologyRuntime<R>`, carries the built `R::Topology` (or builds it from config via `Provider::build_topology`). `register`/`register_resolved` thread it into `ManagedResource`.
- `crates/resource/macros/src/{resource_attrs,resource}.rs`: `#[resource(topology = Pooled)]` must now **emit `type Topology = Pooled<Self>;`** into the generated `impl Provider` (today it emits only a `RESOURCE_TOPOLOGY` `TopologyTag` const — keep the tag for diagnostics, add the assoc-type emission) plus the `build_topology` body constructing `Pooled::new(config_attrs)`. A hand-written `impl Provider` spells `type Topology` itself. Add a trybuild pass proving `#[resource(topology = Pooled)]` expands with the assoc type.

- [ ] **Step C7: Migrate engine + examples off `TopologyRuntime`.**

- `crates/engine/src/resource/registrar.rs` (the `FTopo` closures at 332/345/373 + `TopologyRuntime::Resident` constructs) and `crates/engine/src/resource_accessor.rs` (269/308 specs, 274/313 constructs): the activator builds `R::Topology` from config instead of a `TopologyRuntime` variant. Topology **kind** is static per `R` (a Postgres is always `Pooled`); only the **config** (cap, sizes) stays runtime — this fits the erased registration (spike §6).
- `examples/examples/resource_{postgres_pool,resident_http,telegram_multi_workflow}.rs`: set `type Topology` + construct via the registration helper.

- [ ] **Step C8: Manager-integration test (the missing proof) + port all existing tests.**

- Add `crates/resource/tests/custom_topology_manager.rs`: a custom permit-only `FfmpegPool` (own struct, `type Slot = ()`) on a resource whose `type Topology = FfmpegPool`, **registered through `Manager::register()`**, acquired + released end-to-end, and proven to receive the revoke fence (bump epoch → in-flight slot evicted on return). This is the proof the standalone `custom_topology.rs` never gave.
- Port every existing pool/resident/rotation/toctou/revoke test — they must stay green **with unchanged intent**, especially `revoke_recycle_toctou::*`, `resident_rotation_race::*`, `shutdown_race::*`, `basic_integration::pool_maintenance_reaper_*` (340 tests green at HEAD `b799b54d` — that is the floor).

- [ ] **Step C9: Gates + commit.**

Full per-crate gates for `nebula-resource`, `nebula-resource-macros`, `nebula-engine`, `nebula-examples` (bare `-p`, never `--all`; `cargo fmt -p` per crate). Rustdoc gate on resource. Commit:
```bash
git -C . commit -m "feat(resource)!: converge topology onto Prov::Topology assoc type — open trait through Manager, uniform fence (incl. checkout), delete TopologyKind dispatch"
```

**Acceptance:** a custom `impl Topology` registers through `Manager::register()` + acquires + releases + gets the revoke fence (C8 test green); `InstanceStore::checkout` evicts stale-epoch slots (C1 tests green); `git grep "TopologyKind\|TopologyRuntime\|AcquireDispatch\|InstanceStore::new(None)"` over `crates/` returns no live hits; `ManagedResource<R>` is still single-parameter (`get_typed::<R>` unchanged); the 340-test floor passes with unchanged intent; clippy `-D warnings` + rustdoc `-D warnings` clean.

---

## Phase D — Admission surface (the Spec 2 seam)

**Why:** expose TOCTOU-free availability so the engine (Spec 2, separate) can defer-or-dispatch. Spec §3. Additive — no behavior change to acquire beyond returning typed `Unavailable`.

**New types (spec §3):** `AdmissionPhase { Ready, Warming, Recovering, Saturated, Tainted }`; `Load { saturation: f32, est_wait: Option<Duration>, detail: LoadDetail }`; `LoadDetail { Permits{used,total}, Inflight(u32), Lag(u64), ByteBudget{used,max}, None }`; `CheckCost { Cheap, Moderate, Expensive }`.

- [ ] **Step D1: Define the admission types** in `crates/resource/src/admission.rs`, re-export from lib.rs. `AdmissionPhase` is **orthogonal** to the existing `state::ResourcePhase` (do not merge). Add `#[non_exhaustive]` on the enums.

- [ ] **Step D2: Wire `try_reserve` as the gate.** `Topology::try_reserve` already returns `Result<Ticket, Unavailable>` (Phase C). Implement `Pooled`/`Resident` `phase()` + `load()` honestly: Pooled returns `Saturated` when permits exhausted + `Load::permits(used,total)`; Resident returns `Ready` + `load()=None`. `Manager::acquire_any` returns the typed `Unavailable` mapped to `Error` (`Saturated`→`Backpressure{retry_after}`, `Warming/Recovering`→`Transient`, `Tainted`→`Revoked`) so the existing `ErrorKind` taxonomy carries it.

- [ ] **Step D3: Add `check_cost()` to `Provider`** (default `Cheap`) — sync method, used by the future engine probe scheduler; no consumer in Spec 1, but it's part of the seam.

- [ ] **Step D4: Expose the read surface on `ManagedHandle`** — `fn admission_phase(&self) -> AdmissionPhase` and `fn admission_load(&self) -> Option<Load>` delegating to the row's topology, so Spec 2 reads them without `R`. Add a test asserting a saturated pool reports `phase()==Saturated` + `load().saturation==1.0`, and `try_reserve` on it returns `Err(Saturated)`.

- [ ] **Step D5: Gates + commit.**
```bash
git -C . commit -m "feat(resource): TOCTOU-free admission surface (try_reserve Ticket + AdmissionPhase/Load/CheckCost)"
```

**Acceptance:** the saturated-pool test passes (gate=`try_reserve`, `load()` advisory); `AdmissionPhase` never folded into `ResourcePhase`; the four `Unavailable` variants map to the documented `ErrorKind`s.

---

## Phase E — Async/fallible lifecycle (A4 / A5 / A6)

**Why:** 16/22 cases need `release`/`destroy`/`check` to be real async work, not `Drop` glue. Revises canon §11.4 (ADR in Phase H). Spec §5 1.0-must.

- [ ] **Step E1: A5 — `on_release` async + fallible + ordered.** `Topology::on_release(&self, slot: &mut Slot) -> Result<(), Error>`: `Ok` → `InstanceStore::return_slot` (under the fence); `Err` → evict + destroy; panic-in-guard → poison → evict. The `ResourceGuard` drop schedules `on_release` on the release task (already on `ReleaseQueue`), then the framework runs the fence + return/evict. Port the existing "dirty connection not re-pooled" test; add a "reset Err evicts" test and a "panic poisons" test.

- [ ] **Step E2: A6 — `Provider::destroy` gains a deadline.** `async fn destroy(&self, instance: Self::Instance, timeout: Duration) -> Result<(), Error>` (or carry the timeout via `DrainTimeoutPolicy` already present). Ordered-before-drop: flush/drain/close run before the instance is dropped. Test: a `destroy` that flushes sets a flag observed before drop; a `destroy` exceeding timeout is abandoned with a `tracing::warn` + a typed `Error`, not a hang.

- [ ] **Step E3: A4 — per-acquire session-init.** `Topology::acquire` may do I/O (the spec's session-init): for Pooled, run `prepare`-style init after checkout, before handing the lease (e.g. `SET search_path`); on init `Err`, evict + retry-or-fail. Test: a Pooled resource whose `acquire` sets per-acquire state proves the state is present on the lease and reset on release.

- [ ] **Step E4: Observability (DoD).** Each new error path ships a typed `Error`/`ErrorKind` variant (reuse existing where possible) + a `tracing` span (`resource.release`, `resource.destroy`) + a `debug_assert!` on the fence invariant. Emit `ResourceEvent` variants for evict/poison.

- [ ] **Step E5: Gates + commit.**
```bash
git -C . commit -m "feat(resource)!: async fallible ordered release/destroy + per-acquire session-init (A4/A5/A6); revises canon §11.4"
```

**Acceptance:** reset-Err-evicts, panic-poisons, destroy-flushes-before-drop, destroy-timeout-no-hang, and session-init-reset tests all pass; no teardown logic runs in `Drop` (only scheduling).

---

## Phase F — Restore `Bounded` (built-in topology, runtime cap)

**Why:** the one undisputed capability gap (gRPC-cap / serial-exclusive / license-seats). Spec §2.5, A1. It was deleted in commit `445854ce`; re-add as a built-in `impl Topology`, NOT the old const-generic `Capped<N>`.

- [ ] **Step F1: `Bounded` topology with runtime cap.** Create `crates/resource/src/topology/bounded.rs`: `pub struct Bounded { mode: BoundedMode, sem: Arc<Semaphore>, … }` where `pub enum BoundedMode { Capped(usize), Exclusive, Unbounded }`. `Capped(n)`/`Exclusive(n=1)` back the gate with a `tokio::Semaphore`; `Unbounded` is infallible `try_reserve`. `set_cap(n)` grows/shrinks via `add_permits`/`forget_permits`. Cap validated at registration as a typed `Error` (n≥1 for Capped/Exclusive), never a panic — mirror the old `PoolRuntime::try_new` fail-closed shape.
- [ ] **Step F2: `Exclusive` reset-ordering + poison.** `on_release` for `Exclusive` runs reset-before-reissue; reset `Err` poisons until recovery-gate clears. Port the old exclusive/transport tests' *intent* (they were deleted with Bounded) as fresh tests over the new `Bounded` impl — Capped<2> session count, Exclusive one-at-a-time, reset-Err-poisons, cap-shrink-mid-run.
- [ ] **Step F3: Registration + derive attr.** `#[resource(topology = bounded)]` (or `bounded(mode=…, cap=…)`) selects it; or author registers `Bounded::capped(n)` directly. Re-add the `TopologyTag::Bounded` discriminant + lib.rs exports.
- [ ] **Step F4: Gates + commit.**
```bash
git -C . commit -m "feat(resource): restore Bounded as a runtime-cap built-in Topology (Capped/Exclusive/Unbounded)"
```

**Acceptance:** Capped/Exclusive/Unbounded tests pass; cap is set from a runtime value (test constructs `Bounded::capped(read_from_config)`); `set_cap` grow/shrink test passes; no const-generic `Capped<N>`.

---

## Phase G — A10 dedup affinity, A11 cost-aware check, A12 parent-generation recovery

**Why:** correctness for stateful/disposable/multiplexed resources. Spec §5 1.0-must.

- [ ] **Step G1: A10 — dedup affinity / anti-share modes.** Extend the `(key, scope, SlotIdentity)` dedup with a per-resource `ShareMode { Shared, AffinityKey(fn), AntiDedup }` declared on `Provider` (default `Shared`). Framework-enforced at registry insert: `AntiDedup` never shares a row even at identical config; `AffinityKey` keys the row by an intrinsic id (session-id), not config fingerprint. Tests: two `AntiDedup` resources at identical config get distinct rows; an `AffinityKey` resource routes back to the same instance by id.
- [ ] **Step G2: A11 — cost-aware `check()` scheduling contract.** `check_cost()` (Phase D3) is consumed by the maintenance reaper to space expensive probes (Cheap ~10s, Expensive ~minutes). Test: a resource with `CheckCost::Expensive` is probed less often than a `Cheap` one over a fixed window (use `tokio::test(start_paused = true)`).
- [ ] **Step G3: A12 — parent-generation recovery.** Two-level recovery: a parent instance (AMQP connection, browser) death invalidates its child pool (channels, pages) atomically by parent generation. `BrokenCheck`/recovery-gate key on parent generation. Test: incrementing the parent generation invalidates all child leases.
- [ ] **Step G4: Gates + commit.**
```bash
git -C . commit -m "feat(resource): dedup affinity modes (A10), cost-aware check (A11), parent-generation recovery (A12)"
```

**Acceptance:** the three tests above pass; `AntiDedup`/`AffinityKey` are framework-enforced (not author discretion).

---

## Phase H — Canon ADR + standard/memory mirror + docs sweep

**Why:** the async-trait override and the §11.4 release revision are binding decisions that future sessions must not silently revert. Spec §2.7, §6 D4, §8.

- [ ] **Step H1: ADR for canon §11.4 revision.** Write `docs/adr/00NN-resource-release-is-async-work.md` (next number; check `docs/adr/README.md`): release/destroy/check are fallible async ordered work, not best-effort `Drop`; the in-process manager reconciles orphaned slots via the reaper; guaranteed-destroy for money/hardware is out-of-scope (external janitor, later spec). Update `docs/PRODUCT_CANON.md` §11.4 wording + add the ADR to `docs/adr/README.md`.
- [ ] **Step H2: ADR (or fold into H1) for the async-trait policy.** Record the deliberate override: `#[async_trait]` crate-wide for nebula-resource because lifecycle is I/O-bound, async-trait is trusted/ubiquitous, migration-off is trivial, dynosaur is a heavier bet. Reference §2.7.
- [ ] **Step H3: Mirror into the path-scoped standard + memory.** Update the project's Rust standard note that says "prefer native async-fn-in-trait" to carve out the nebula-resource I/O-bound + always-erased exception. Update the `feedback_idiom_currency` auto-memory (`C:\Users\vanya\.claude\projects\…\memory\feedback_idiom_currency.md`) with a one-line note: "nebula-resource deliberately uses #[async_trait] (I/O-bound, dyn-everywhere, trivial migration) — see ADR-00NN; do not 'modernize' it back."
- [ ] **Step H4: Docs sweep.** Update `crates/resource/README.md`, `crates/resource/CLAUDE.md`, `crates/resource/docs/*` to the final vocabulary (Provider/Instance/Topology open trait/ManagedHandle/Bounded-restored/admission surface). Rustdoc gate: `RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-resource --no-deps`.
- [ ] **Step H5: Commit.**
```bash
git -C . commit -m "docs(resource): ADR for async release + async-trait policy; mirror into standard/memory; README/CLAUDE sweep"
```

**Acceptance:** ADR(s) filed + indexed; canon §11.4 updated; the idiom-currency memory + path-standard carry the carve-out; rustdoc clean; READMEs/CLAUDE.md truthful.

---

## Self-review (plan vs spec)

- **Spec coverage:** §0 vocabulary→Phase A; §2.1 async/erasure→Phase B; §2.1–2.5 open Topology + InstanceStore→Phase C; §2.6 credentials preserved→Phase C (uniform fence in InstanceStore) + asserted in C5; §2.7 async policy + rename→Phases B+H; §3 admission→Phase D; §5 A1 Bounded→Phase F, A2 phases→Phase D, A3 load→Phase D, A4/A5/A6→Phase E, A10/A11/A12→Phase G; §6 D4→confirmed (Phase B/H), D1 batteries default Pooled/Resident/Bounded→Phases C/F (Multiplexed/Ephemeral deferred, not in this plan), D2 load-routing→Spec 2 (out of scope), D3 reattach-key→Spec 2/1.1 (guard reserves a detach hook only — note in Phase E if it costs nothing, else defer); §7 migration→the phase order; §8 risks→addressed (InstanceStore rule C, ticket TOCTOU D, canon §11.4 H). Gap check: A7/A8/A9/A13 are explicitly 1.1-deferred (spec §5) — correctly absent.
- **Placeholder scan:** structural tasks reference the spec's concrete target code (trait/type defs reproduced) + concrete tests + exact gates; the only "implement per the existing internals" is the `pool.rs` re-seat (C3), bounded by an explicit do-NOT-change-correctness rule — this is a refactor seam, not a placeholder.
- **Type consistency:** `Provider`/`Instance`/`Topology`/`ManagedHandle`/`ResourceDescriptor`/`ResourceActivator`/`InstanceStore`/`Ticket`/`Unavailable`/`AdmissionPhase`/`Load`/`Bounded`/`BoundedMode` used consistently across phases.

**Note for the executor:** Phases A and B are large mechanical sweeps over ~40–55 files — use Serena symbol-level edits or careful per-file `Edit`, **not** regex/Python bulk scripts (they break brackets across files; this exact failure happened during the prior registration-migration attempt). Verify each file compiles before moving on.
