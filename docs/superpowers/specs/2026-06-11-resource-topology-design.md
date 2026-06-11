# Spec 1 — Open, Engine-Managed Resource Topology

**Status:** design, awaiting architect approval before planning.
**Layer:** Business (`nebula-resource`).
**Date:** 2026-06-11.
**Provenance:** distilled from a 12-agent research workflow (7 domain case-catalogs + Rust/orchestrator prior-art + adversarial critique + a 22-case stress-test). Raw inputs: `2026-06-11-resource-topology-design-RAW.md`, `2026-06-11-resource-case-fit-matrix.md` (generated artifacts).

---

## 0. Vocabulary (locked)

This spec uses a renamed vocabulary that supersedes the names on the current branch. The renames make each word mean exactly what it says and free the headline word `Resource` for the derive.

| Concept | This spec | Was (current branch) |
|---|---|---|
| Declare-a-resource derive | `#[derive(Resource)]` | `#[derive(ResourceSlots)]` |
| Lifecycle trait (create/check/destroy/rotation) | **`Provider`** | `Resource` (trait) |
| The live handle (PgPool, reqwest::Client, a process) | **`Instance`** (assoc type) | `Runtime` (assoc type) |
| Lease / concurrency / sharing policy | `Topology` (trait) | `Topology` (enum) |
| What a caller holds | `ResourceGuard` | `ResourceGuard` |
| Credential-epoch trait the derive emits | `HasCredentialSlots` | `HasCredentialSlots` |

"Resource" remains the **concept and namespace**: `nebula-resource`, `ResourceConfig`, `ResourceContext`, `ResourceGuard`, `ResourceMetadata`, `ResourceKey`, `ResourceEvent`. You **derive `Resource`** to declare one, **implement `Provider`** to say how its `Instance`s are created and managed, and the resource's **`Topology`** governs how callers lease those instances under concurrency.

Reads:

```rust
#[derive(Resource)]
#[resource(topology = Pooled)]              // select a built-in topology: zero topology code
struct Postgres { #[credential(key = "db")] auth: CredentialSlot<PgCred> }

impl Provider for Postgres {
    type Config   = PgConfig;
    type Instance = PgPool;                 // the live handle
    async fn create(&self, c: &PgConfig, ctx: &ResourceContext) -> Result<PgPool, Error> { /* ... */ }
}

impl Pooled for Postgres { /* override is_broken / recycle / prepare hooks (optional) */ }

let guard: ResourceGuard<Postgres> = mgr.acquire::<Postgres>(&ctx, &opts).await?;
```

**Topology selection (two paths):** a **built-in** topology (`Pooled` / `Resident` / `Bounded`) is selected by the `#[resource(topology = …)]` derive attribute — zero topology code; `impl Pooled`/`impl Resident` is *optional* and only customizes that built-in's hooks. A **custom** topology is `impl Topology for YourType` + registered as the row's topology (§4.2). The derive attribute and a hand-written `impl Topology` are mutually exclusive per resource.

---

## 1. Architecture overview

A **Resource** is the author's integration concept (a Postgres, a Telegram bot, an FFmpeg). The author **derives `Resource`** (slot plumbing) and **implements `Provider`** — two associated types (`Config`, `Instance`) plus lifecycle methods (`create` / `check` / `shutdown` / `destroy` / `on_credential_refresh` / `on_credential_revoke`). This half already ships today (renamed).

A **`Topology`** is the engine-facing lifecycle contract describing how `Instance`s are *leased* to callers: the concurrency gate, the acquire→guard→release mechanics, and a read-only availability surface. Today `Topology` is a **closed enum** (`Pooled`, `Resident`; `Bounded` was deleted). This spec promotes it to an **open trait** an author may implement in a plugin crate, with a closed set of audited batteries-included impls.

The structural rule that makes "open" safe — and that defuses every adversarial objection — is:

> **The Manager owns instance storage; a `Topology` owns only the lease/permit policy over storage the Manager hands it.** A `Topology` never holds its own connection cache, `static` map, or `Arc<Pool>`. It is handed a lifetime-bound `&InstanceStore<Slot>` it cannot retain past a call. It therefore *cannot* build a host-keyed cache that bypasses `SlotIdentity`, so the cross-tenant barrier is preserved by API shape, not by author discipline.

```
author crate                      nebula-resource  (framework-owned, UNIFORM)              engine (Spec 2)
┌──────────────┐  derive Resource ┌──────────────────────────────────────────────┐
│  Resource    │ ───────────────► │  Manager                                     │
│  (#[derive], │  impl Provider   │   • dedup by (Key, Scope, SlotIdentity)+fp    │
│   slot decl) │ ───────────────► │   • scope-hierarchy lookup        [UNIFORM]  │
└──────────────┘                  │   • revoke-epoch fence            [UNIFORM]  │
       ▲                          │   • rotation fan-out              [UNIFORM]  │
       │ impl Topology (optional) │   • recovery-gate + maintenance   [UNIFORM]  │
       │  (lease policy +         │   • two-phase drain / quiesce     [UNIFORM]  │
       │   availability surface)  │   • metrics / events / spans      [UNIFORM]  │
       │                          │   • InstanceStore<Slot>  (framework-held)    │
       │                          │   ┌────────── per-Topology (small) ──────┐   │
       └──────────────────────────┼──►│ try_reserve(&store) → Ticket  (gate)  │   │
                                  │   │ acquire(Ticket,&store) → Lease<Slot>  │   │
                                  │   │ on_release(&mut Slot)  reset (async)  │   │
                                  │   │ phase(&store) / load(&store)  (read)  │   │
                                  │   └───────────────────────────────────────┘   │
                                  │   erased acquire: Arc<dyn Fn(..) ->          │
                                  │     BoxFuture<Result<ResourceGuard>>>         │
                                  └────────────────────┬──────────────────────────┘
                                                       │ ResourceGuard (Deref→Instance,
                                                       │  RAII release → on_release → store)
                                                       ▼
                                           ┌──────────────────────────┐
                                           │ admission surface (read) │◄── Spec 2 reads:
                                           │  try_reserve → Ticket     │     gate = try_reserve
                                           │  phase() : Copy snapshot  │     route/UI = phase/load
                                           │  load()  : Option<Load>   │     (advisory only)
                                           └──────────────────────────┘
```

**Uniform-in-Manager (one audited implementation, never author-touchable):** dedup by `(ResourceKey, ScopeLevel, SlotIdentity)` + config fingerprint; scope-hierarchy resolution; the revoke-epoch fence (snapshot epoch at create-start, re-check under lock on every return-to-store path, destroy-not-recycle on stale); rotation fan-out (`on_credential_refresh`/`on_credential_revoke` walked over all live instances of a row); recovery-gate + maintenance reaper; two-phase drain/quiesce; instance storage; metrics/spans/events.

**Per-Topology (author writes, small):** the concurrency gate (`try_reserve` → ticket), how a ticket becomes a leased `Instance` over framework-held storage (`acquire`), the release-time reset (`on_release`), and the read-only availability snapshot (`phase`/`load`).

This split is the whole design: **authors express *lease policy*; the framework keeps *every credential / tenant / drain / recovery invariant*.**

---

## 2. The open `Topology` contract (the core)

### 2.1 The two traits (author RPITIT + framework erasure)

Authoring uses an RPITIT trait (ergonomic, no `async_trait`). The Manager consumes a boxed-future erasure of it (object-safe) — exactly mirroring the `Provider`/`AnyManagedResource` split already shipped. Authors implement the RPITIT trait; the framework's blanket impl produces the erased form. **Authors never write `Box<dyn Any>`** — the erasure boundary is closed by making `Topology` generic over its own `Slot`, not over `Any`.

```rust
/// Author-facing lease policy for a resource's instances. Object-safe via a
/// separate framework `ErasedTopology` (NOT the sealed `AnyManagedResource`).
/// `forbid(unsafe_code)`, thiserror errors.
pub trait Topology: Send + Sync + 'static {
    /// The leasable unit the framework stores. Pooled: one connection.
    /// Resident: the shared handle. Bounded: `()` (permit-only, no stored instance).
    type Slot: Send + Sync;

    // ---- concurrency gate: admission is a value, not a bool (resolves TOCTOU) ----

    /// Non-blocking. Returns a permit-bearing `Ticket` or a typed `Unavailable`.
    /// The Ticket IS the reservation — holding it guarantees a slot; dropping it
    /// releases it. There is no separate "is there room?" boolean to lie.
    fn try_reserve(&self, store: &InstanceStore<Self::Slot>)
        -> Result<Ticket<Self::Slot>, Unavailable>;

    /// Async, fallible. Consumes a `Ticket`; produces the live slot to lease.
    /// May do network I/O (channel-open, page-open, post-checkout `SET`) — this
    /// is the per-acquire session-init hook (A4). The framework wraps the result
    /// in a `ResourceGuard` whose release runs `on_release` then returns the slot.
    fn acquire(&self, ticket: Ticket<Self::Slot>, store: &InstanceStore<Self::Slot>)
        -> impl Future<Output = Result<Lease<Self::Slot>, Error>> + Send;

    /// Async, fallible, ordered-before-reissue. Reset the slot to a clean
    /// baseline (rollback txn, reset PRAGMAs, UNSUBSCRIBE, cancel transients).
    /// `Err`  => framework EVICTS the slot (destroy, do not return to store).
    /// `Ok`   => framework returns the slot to store for the next acquirer.
    /// Panic-in-guard => framework treats the slot as POISONED => evict (A5).
    fn on_release(&self, slot: &mut Self::Slot)
        -> impl Future<Output = Result<(), Error>> + Send { async { Ok(()) } }

    // ---- availability surface (sync, Copy, advisory — see §3) ----

    fn phase(&self, store: &InstanceStore<Self::Slot>) -> AdmissionPhase;
    fn load(&self, store: &InstanceStore<Self::Slot>) -> Option<Load> { None }
}
```

### 2.2 Why `Ticket` (not a boolean) resolves check-then-acquire TOCTOU

The tower `poll_ready` lesson (issues #408/#412/#431) and tokio #898 both prove: a readiness *boolean* is a lie the instant after it returns, and a readiness *reservation held in `&mut self`* gets stranded. The fix both projects converged on is a **droppable capability token**. `try_reserve` returns exactly that: the `Ticket` carries an `OwnedSemaphorePermit` (Bounded/Pooled) or is zero-cost (Resident). The engine's admission decision *is* `try_reserve` — success means a slot is genuinely held; failure returns a typed `Unavailable` to park on. `acquire` consumes the ticket, so you cannot acquire without first reserving, and you cannot reserve without holding capacity. There is no second, weaker signal to race.

```rust
pub enum Unavailable {
    Saturated { retry_after: Option<Duration> }, // capacity full now — park & reschedule
    Warming,                                       // instance not query-ready (index build, cold start)
    Recovering,                                    // mid-reconnect / mid-reset / rebalance
    Tainted,                                       // creds revoked / poisoned — until recovery-gate clears
}
```

### 2.3 `ResourceGuard` and release semantics

`acquire` yields a `Lease<Slot>`; the Manager wraps it in a `ResourceGuard` that `Deref`s to the resource's `Instance`. On guard drop the framework runs the release path **as real async work, scheduled on a release task — not in `Drop`** (this revises canon §11.4 for server-side-stateful resources, see §5): `on_release(slot)` runs; `Ok` returns the slot to `InstanceStore` *under the revoke-epoch fence*; `Err` or panic evicts and destroys. The guard never lets an author re-pool a dirty or credential-stale slot — that decision is the Manager's, keyed on the fence, not the author's `on_release`.

### 2.4 A custom author topology in ~30 lines (FFmpeg pool, case #7)

```rust
pub struct FfmpegPool { sem: Arc<Semaphore>, cap: usize }   // permit-only; process spawned in action body

impl Topology for FfmpegPool {
    type Slot = ();                              // permit-only Instance (case #7)

    fn try_reserve(&self, _s: &InstanceStore<()>) -> Result<Ticket<()>, Unavailable> {
        match self.sem.clone().try_acquire_owned() {
            Ok(permit) => Ok(Ticket::permit(permit)),    // the permit IS the reservation
            Err(_)     => Err(Unavailable::Saturated { retry_after: None }),
        }
    }
    async fn acquire(&self, ticket: Ticket<()>, _s: &InstanceStore<()>) -> Result<Lease<()>, Error> {
        Ok(Lease::from_permit(ticket))           // hand back the permit-guard; the action spawns ffmpeg
    }
    // on_release: default (permit drop frees capacity); nothing to reset

    fn phase(&self, _s: &InstanceStore<()>) -> AdmissionPhase {
        if self.sem.available_permits() == 0 { AdmissionPhase::Saturated } else { AdmissionPhase::Ready }
    }
    fn load(&self, _s: &InstanceStore<()>) -> Option<Load> {
        Some(Load::permits(self.cap - self.sem.available_permits(), self.cap))   // used/total, advisory
    }
}
```

For free — because the Manager owns everything else — this FFmpeg pool gets: dedup by `(binary-path+hwaccel-fp, scope, SlotIdentity)`; rotation fan-out if it ever grows a credential slot; recovery-gate + reaper; two-phase drain on shutdown; acquire/wait tracing spans; saturation/recovery `ResourceEvent`s; and the engine admission read. The author wrote lease policy and a load gauge; the framework supplied the moat.

### 2.5 Built-ins that ship

Closed, audited, framework-owned — these carry the revoke-fence proof so authors rarely implement from scratch:

- **`Pooled`** — N interchangeable, checkout/recycle. `Slot` = the connection. `on_release` runs reset-before-repool (A5 applies to Pooled, not only Exclusive). Cases #1, #5, #19-channels.
- **`Resident`** — one shared handle, clone-on-acquire, `on_release` no-op. `try_reserve` is infallible (unbounded logical concurrency). Cases #2, #9, #14, #18, #21, most cloud SDKs.
- **`Bounded` (RESTORED)** — runtime-configurable cap, three modes:
  - `Capped(n)` where `n` is read from config/JSON at registration (NOT a const generic), validated as a typed `Error` at registration, never a panic — mirrors the existing `PoolRuntime::try_new` fail-closed pattern. Cases #7, #22-license, permit governors.
  - `Exclusive` (`n == 1`, with reset-ordering before re-issue + poison-on-panic). Cases #11 serial-port, #22-device.
  - `Unbounded` (token-gated, no concurrency cap; backpressure is logical-inflight). A degenerate `Resident`-with-a-gauge.

  Bounded's cap is **runtime-set-and-queryable** because the unanimous case evidence is that *no resource in any catalog has a compile-time-fixed N* (license seats, `num_cpus`, MIG slices, peer-negotiated `MAX_CONCURRENT_STREAMS`, broker channel-max). The cap may even change at runtime — `Bounded` exposes `set_cap(n)` that grows/shrinks the semaphore via `add_permits` / `forget_permits`.

The open trait does **not** reuse the sealed `AnyManagedResource`; it introduces a *separate* object-safe `ErasedTopology`, and the revoke/rotation/drain invariants stay in Manager code over `InstanceStore`, never delegated to author code.

### 2.6 Credentials & authorization — outside the `Topology` boundary (preserved by construction)

The active credential lifecycle (slots, create-time resolution, OAuth refresh / rotation / revocation, per-tenant owner scoping, the revoke-epoch fence) is **the product moat and is not changed by this spec**. Every credential mechanism lives on the **`Provider`** side or is **UNIFORM-in-Manager** — explicitly *outside* the `Topology` trait. A `Topology` (built-in or custom) never sees a credential, a `CredentialGuard`, or a `SlotIdentity`.

| Credential mechanism | Where it lives | Changed? |
|---|---|---|
| `#[credential(key)]` slot fields, `CredentialSlot<C>`, `HasCredentialSlots` epoch | `#[derive(Resource)]` + the resource struct (Provider side) | **No** — derive renamed, plumbing identical |
| Slot resolution before `create` (engine binds `CredentialGuard` into the cell) | engine → `Provider::create` | **No** — `create` still runs after resolution |
| Rotation hooks `on_credential_refresh` / `on_credential_revoke` | `Provider`, called by the Manager rotation fan-out | **No** — fan-out is UNIFORM-in-Manager, walks `InstanceStore` |
| Revoke-epoch fence (`credential_slot_epoch`, create-vs-rotate race) | Manager, every return-to-store path | **Strengthened** — becomes a uniform fence over `InstanceStore` (today a Pool-only 2-arm `match`), so it now covers custom topologies too |
| `SlotIdentity` cross-tenant barrier, `(key, scope, SlotIdentity)` dedup | Manager registry | **No** — the `InstanceStore` rule (§1) exists *precisely* to keep it intact for custom topologies |
| Two-phase revoke (sync taint + epoch bump, then drain + hook) | Manager | **No** |

**Why a custom (author-written) topology cannot break authorization.** A `Topology` is handed a lifetime-bound `&InstanceStore<Slot>` it cannot retain and cannot populate with its own credential-bearing instances — instances are created by `Provider::create` (which the framework drives *after* credential resolution) and stored by the framework. The Topology only decides *which already-built, already-authorized instance* to lease and how many at once. Credential resolution, rotation fan-out, the revoke-epoch fence, and the `SlotIdentity` barrier all run in Manager code over the framework-owned store, regardless of what the topology does. An author topology with a wrong `on_release` can leak its own resource's *non-credential* state (its own bug); it cannot resurrect a revoked credential (the fence evicts it) nor cross a tenant boundary (it never owns instances).

**Net: client authorization is preserved by construction. The open topology widens *lease policy*, never the credential/tenant seam.** The one credential-adjacent *improvement* the spec lands is that the revoke-epoch fence stops being a Pool-only branch and becomes uniform over `InstanceStore`, so a rotated/revoked credential is fenced on *every* topology — built-in or third-party — not just the pool.

---

## 3. The availability / admission surface (the seam for Spec 2)

> **This is the seam Spec 2 (engine resource-aware scheduling) consumes. Spec 1 designs the hooks; Spec 1 does NOT design the scheduler.**

Three layers, ordered cheap→expensive, deliberately minimal and TOCTOU-honest.

### 3.1 Authoritative layer — `try_reserve` (gate = grant)

The **only** signal the engine may *gate* on is `try_reserve`, because it is the only one that cannot lie: success hands back a held `Ticket`. The scheduler's admission loop is **try-reserve-and-park, never read-and-decide** (`stats()` is documented non-atomic; any pre-acquire gauge is TOCTOU-by-construction). Spec 2 dispatch sketch:

```text
on step needing resource R:
    match topology.try_reserve(&store):
        Ok(ticket)                     => dispatch worker; worker calls acquire(ticket)
        Err(Saturated { retry_after }) => park step, reschedule after retry_after (priority-weighted)
        Err(Warming | Recovering)      => park step, reschedule on next phase-change event
        Err(Tainted)                   => fail step or route to recovery-gate
```

### 3.2 Advisory layer — `phase()` (cheap cached `Copy` snapshot)

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AdmissionPhase { Ready, Warming, Recovering, Saturated, Tainted }
```

This is an **orthogonal admission axis**, NOT folded into the lifecycle `ResourcePhase` (`Initializing`/`Ready`/`Reloading`/`Draining`/`ShuttingDown`/`Failed`). 12 cases need `Warming` (index build / cold start / model load — distinct from "Initializing = being constructed"), `Recovering` (mid-reconnect / rebalance), and `Tainted` (post-failover suspect / creds revoked). `phase()` is for **prioritization and UI** (which resources to *prefer*, what to show operators), updated by the periodic active check (§3.3). **`Saturated`-as-phase is advisory only** — gating on it is a TOCTOU lie, so the *gate* is always `try_reserve`; `phase() == Saturated` only tells the scheduler "don't bother try_reserving this one first."

### 3.3 Active layer — `check()` cost class + `load()`

Active health stays on the existing `Provider::check` (author-side) + `RecoveryGate`. Spec 1's addition is a **cost class** so Spec 2 schedules expensive probes less often (`SELECT 1` is cheap; `PRAGMA quick_check` / `DescribeTable` / quota-consuming health are not):

```rust
pub enum CheckCost { Cheap, Moderate, Expensive }    // engine: Cheap ~10s, Expensive ~minutes
fn check_cost(&self) -> CheckCost { CheckCost::Cheap }   // on Provider, default cheap
```

`load()` returns an **optional, author-supplied, generic snapshot** — never a fixed `(open, max)`. The metric shape varies in every catalog (pool occupancy, inflight-pipeline depth, CMAP wait-queue, server `max_concurrent_queries`, PEL/lag, RCU/WCU headroom, VRAM, byte-budget), so it is one `PartialOrd` scalar plus an opaque detail, modeled on tower `Load`:

```rust
#[derive(Clone, Copy)]
pub struct Load {
    pub saturation: f32,             // 0.0..=1.0, PartialOrd — least-loaded routing in Spec 2
    pub est_wait: Option<Duration>,  // None when meaningless (FFmpeg variance, case #7)
    pub detail: LoadDetail,          // Permits{used,total} | Inflight(u32) | Lag(u64) | ByteBudget{used,max} | None
}
```

`load()` is **diagnostics + routing-preference only, never a gate**. Spec 2 may use `saturation` to *rank* candidates (power-of-two-choices) among those that *already returned `Ok` from `try_reserve`*; it may not use it to decide admission. `None` is a first-class answer (shared singletons have no honest local load).

**Boundary stated explicitly:** rate-limit / token-budget (LLM/Telegram/cloud temporal budgets) is **enforced in `nebula-resilience`, surfaced read-only here**. The limiter *drives* `phase()` (sets `Saturated`/`Recovering` from 429/Retry-After headers) and may populate `load().saturation`; the topology owns no rate accounting. Temporal budgets refill on a clock, not on release, so they are never `Bounded` permits.

---

## 4. DX walkthrough

### 4.1 Minimal case — one shared HTTP client (case #2), ~11 lines

```rust
#[derive(Resource)]
#[resource(topology = Resident)]              // the entire topology choice: one attribute
struct HttpClient;

impl Provider for HttpClient {
    type Config   = HttpConfig;
    type Instance = reqwest::Client;
    async fn create(&self, cfg: &HttpConfig, _ctx: &ResourceContext) -> Result<reqwest::Client, Error> {
        Ok(reqwest::Client::builder().timeout(cfg.timeout).build()?)
    }
    // check / shutdown / destroy: derive defaults. No topology code at all.
}
```

The author writes **zero topology code** and gets dedup, rotation, recovery, drain, metrics, and a `phase() == Ready` admission read. `load()` defaults to `None` (honest — reqwest's pool is not introspectable).

### 4.2 Exotic case — gRPC N-streams (case #9), author ships a custom topology

The cap is *peer-negotiated* (`MAX_CONCURRENT_STREAMS`, mutates mid-connection), so `load()` must pull-from-live-resource, not a stored const — the one thing the closed enum can't pre-name. The author still writes only the lease policy:

```rust
struct GrpcStreams { channel: Channel }       // Slot = the shared HTTP/2 channel

impl Topology for GrpcStreams {
    type Slot = Channel;
    fn try_reserve(&self, s: &InstanceStore<Channel>) -> Result<Ticket<Channel>, Unavailable> {
        match self.channel.connectivity_state() {            // pull live, not cached
            Ready             => Ok(Ticket::shared(s.shared())),
            Idle | Connecting => Err(Unavailable::Warming),
            TransientFailure  => Err(Unavailable::Recovering),
            Shutdown          => Err(Unavailable::Tainted),
        }
    }
    async fn acquire(&self, t: Ticket<Channel>, _s: &InstanceStore<Channel>) -> Result<Lease<Channel>, Error> {
        Ok(Lease::shared(t))                                 // open the stream over this channel in the action
    }
    fn phase(&self, _s: &InstanceStore<Channel>) -> AdmissionPhase { /* map connectivity_state */ }
    fn load(&self, _s: &InstanceStore<Channel>) -> Option<Load> {
        let used = self.channel.active_streams();
        let max  = self.channel.max_concurrent_streams();    // peer-negotiated, read live
        Some(Load { saturation: used as f32 / max as f32, est_wait: None, detail: LoadDetail::Inflight(used) })
    }
}
```

**Line / concept comparison:**

| Model | Minimal HTTP client | Exotic (gRPC streams / FFmpeg) | Concepts to learn |
|---|---|---|---|
| Dagster `ConfigurableResource` | ~6 lines | not expressible (no topology taxonomy; pooling is hand-rolled, no engine admission) | typed object + setup/teardown |
| Current nebula (closed `{Pooled, Resident}`) | 1 attribute | **impossible** — no variant; author abuses `Resident` with a load-correctness hole | topology enum |
| **This spec** | 1 attribute, 0 topology code | ~20 lines, no friction, full admission + moat for free | `Provider` + (optional) `Topology` |

Nebula matches Dagster's minimal-case ergonomics (one attribute) while delivering what Dagster structurally cannot: an engine that knows and manages the lifecycle (the moat). The exotic case costs ~20 author lines versus *impossible* in both Dagster and current nebula.

---

## 5. Capability scope: 1.0 / 1.1 / CUT

### 1.0 MUST-HAVE

| Item | Why (one line) | Forced by |
|---|---|---|
| A1 `Bounded` restored, runtime cap, typed-Error validation | The one undisputed gap; unanimous across every domain and the adversary. | #7,#11,#22 |
| A14 Open `Topology` trait (framework-storage-only) | Owner-confirmed; gRPC/SSH/browser/disposable shapes can't be pre-named; made safe by the `InstanceStore` rule. | #4,#8,#10,#17 |
| A2 Admission phase axis (`Warming`/`Recovering`/`Tainted`) | 12 cases; orthogonal to lifecycle `ResourcePhase`. | 12 cases |
| Ticket-based `try_reserve` | The admission signal must not lie; gate = grant. | tower #412, adversary §2 |
| A3 Optional `load()`, diagnostics+routing only | Admission = try-reserve-and-park, never read-and-decide. | adversary §2/§4 |
| A4 per-acquire session-init + A5 fallible async reset-on-release + A6 graceful `destroy(timeout)` | Correctness bug, not nicety — session state leaks across tenants; missed teardown leaks server-side quota. Revises canon §11.4. | #1,#8,#11,#17,#19,#20 |
| A10 Dedup affinity / anti-share modes (share / affinity-key / anti-dedup), framework-enforced | Stateful sessions must NOT share same config; disposables never share. Framework-enforced so author caches can't reopen bleed. | #6,#11,#16,#17 |
| A11 Cost-aware semantic `check()` | Socket-liveness is a false-green for every multiplexed/streaming shape. | 7 cases |
| A12 Parent-generation recovery | Conn death must atomically invalidate the channel pool. | #8,#17,#19,#20 |

### 1.1 DEFER

| Item | Why deferred |
|---|---|
| A7 generic renew-while-held + fencing token | Scope only the lock-lease family to 1.0; generalize after a real second consumer. Genuinely outside acquire→guard→release. |
| A8 suspend/passivation across durable wait | Biggest structural break, but durability belongs to the **engine/storage tier** (journal), not the topology trait. 1.0 only ensures the guard *can* detach and carries a reattach key. Templates: Trigger.dev onWait/onResume, DBOS config_name registry. |
| A9 streaming handles | Possibly engine-daemon land (the retired EventSource shape). 1.0 only "does not forbid" a guard outliving one call. |
| A13 first-class supervised background task | Fits-by-convention today (JoinHandle in `Instance`, abort in `shutdown`); low case-count. |
| Built-in `Multiplexed` / `ExclusiveStatefulOwner` / `Ephemeral` batteries | Authors use the open trait now; promote to audited built-ins when a real integration targets them (see decision D1). |

### CUT (speculative — heed the adversary)

| Item | Why cut |
|---|---|
| `Saturated`-as-a-gating-phase | Load dressed as phase; TOCTOU. Kept advisory-only; the gate is `try_reserve`. |
| Engine-scheduled health *poller* as a new surface | `Provider::check` + RecoveryGate + reaper already exist; no consumer for a new poller. |
| Graded load as an admission *input* | Demoted to diagnostics-only; never feeds the scheduler's gate. |
| Lease-renewal as a general contract (1.0) | No second consumer; invites half-renewed leases racing revoke. (The lock-family slice is A7-deferred, not cut.) |

---

## 6. Decision points (architect-confirm on this spec)

These three were flagged by the design lead; the spec defaults them to the recommended answers. Confirm or override during review.

- **D1 — Which batteries ship in 1.0?** *Default: `Pooled` + `Resident` + `Bounded` only.* `Multiplexed` / `ExclusiveStatefulOwner` / `Ephemeral` ship as authored open-trait topologies until a concrete 1.0 integration targets them, then graduate to audited built-ins. Rationale: YAGNI — the open trait makes them cheap to add later, and each built-in carries an audit cost now.
- **D2 — Does `load().saturation` feed Spec 2 routing?** *Default: yes* — power-of-two-choices over a single `f32` among already-reserved candidates (tower-validated minimal coupling). Override to "admit-or-park only, `load()` purely diagnostic" if scheduler↔resource coupling must be zero in 1.0. (This is really a Spec 2 decision; recorded here because it constrains the `Load` shape.)
- **D3 — Where does the A8 durable-passivation reattach-key live?** *Default: the journal (engine/storage tier).* `nebula-resource` only exposes a `reattach_key()` and a detachable guard; durability is owned above. Confirm the tier boundary now so Spec 1's guard type reserves the detach hook without implementing it.

---

## 7. Decomposition + migration

### Spec dependency order

```
Spec 1 (THIS): open Topology contract + restored Bounded + admission surface
   │  produces: try_reserve(Ticket) / phase() / load() / check_cost()   ← the seam
   ▼
Spec 2: engine resource-aware scheduling   (consumes the seam: try-reserve-and-park,
   │                                         priority-weighted park queue, p2c routing on load())
   ▼
Deferred items 5–7 from the prior plan (local to nebula-resource; may interleave with Spec 1):
   5. typed Registration (replace the RegistrationSpec struct with typed builders)
   6. error seam (topology errors → engine backpressure taxonomy)
   7. surface amputation (delete the legacy phase-only projection once the admission axis lands)
   8. (A8) durable passivation — engine/storage tier; depends on Spec 2's park/reschedule model
```

### What changes vs the 4 landed commits

- **Rename** `Resource` (trait) → `Provider`, `Runtime` (assoc) → `Instance`, `#[derive(ResourceSlots)]` → `#[derive(Resource)]`. Pure rename; the slot-plumbing / 2-assoc-type / fingerprint work already landed stays.
- **Restore `Bounded`** as a built-in `Topology` impl (it was deleted). Runtime cap, three modes, `try_new`-style fail-closed validation.
- **Promote `Topology` from closed enum to open trait** + introduce `ErasedTopology` (boxed-future object-safe form) alongside the existing sealed `AnyManagedResource` (do **not** widen the sealed trait — topology gets its own erasure so the semver seal holds).
- **Add `InstanceStore<Slot>`** as the framework-owned storage handle topologies borrow but never own. The existing `PoolRuntime` / `ResidentRuntime` internal fields become `InstanceStore` specializations; the 2-arm `match` in the revoke fence / slot-hook dispatch becomes a uniform fence over `InstanceStore`.
- **Add the admission axis**: `AdmissionPhase` (new enum, orthogonal to `ResourcePhase`), `Load`, `LoadDetail`, `CheckCost`, `Ticket`, `Unavailable`. The legacy phase-only engine projection stays until Spec 2 lands the admission read, then is amputated (deferred #7).
- **Lifecycle methods go async/fallible/ordered** (A4/A5/A6): `on_release`, `destroy(timeout)`, per-acquire session-init. Revises canon §11.4 — a follow-up canon edit accompanies this.
- **No shims.** Hard break: consumers re-dispatch through the `Topology` trait. Pre-1.0, acceptable per the constraints.

---

## 8. Risks + open questions

### Risks

1. **Erasure leak onto authors (mitigated, not eliminated).** The RPITIT `Topology` is generic over `Slot`, so authors never write `Box<dyn Any>` or `Pin<Box<dyn Future>>` — the framework's blanket impl erases. Residual risk: the `Lease<Slot>` / `Ticket<Slot>` types must be ergonomic enough that authors don't reach around them. Mitigation: ship the batteries as worked examples; consider a derive-macro path for the common cases.
2. **TOCTOU (resolved by construction).** `try_reserve → Ticket → acquire(ticket)` makes the admission grant a held value; you cannot acquire without a ticket, cannot ticket without capacity. The only gauge (`load()`) is explicitly non-gating.
3. **Scheduler coupling to topology internals (mitigated by the closed vocabulary).** Spec 2 couples only to `Ticket` / `Unavailable` (closed) + `AdmissionPhase` (closed) + `Load.saturation` (a single `f32`, opaque `detail` ignored). It never reads `available_permits` / `open` / `max`. A new topology with no semaphore still returns a `saturation` and a `Ticket` — no ripple into engine code.
4. **Author implements topology wrong → cross-tenant bleed (mitigated structurally).** The adversary's sharpest attack. Mitigation is the `InstanceStore` rule, enforced by API shape: a `Topology` has no field for instance storage and is handed `&InstanceStore<Slot>` it cannot retain past a call. It physically cannot build a host-keyed `static` cache that bypasses `SlotIdentity`. The revoke-epoch fence runs in Manager code on every return-to-store path regardless of the author's `on_release`. An author can write a *slow* or *incorrect-reset* topology (their own correctness), but cannot reopen the tenant barrier or skip the revoke fence.
5. **Reset/destroy now async + fallible (revises canon §11.4).** Moving release off `Drop` onto a release task means a crashed worker can still skip teardown. Mitigation: the framework's drain/reaper reconciles orphaned slots (poison-on-missing-release → evict); cost/safety-critical guaranteed-destroy (money, hardware) is explicitly out-of-scope for the in-process manager and flagged for an external janitor in a later spec.

### Open questions

The three decision points (D1/D2/D3) in §6 plus: does the canon §11.4 revision (release is real async work, not best-effort `Drop`) need its own ADR, or ride this spec's PR? *Recommendation: a short ADR, because it changes a binding durability invariant.*

---

## 9. Forward-compatibility with planned resource features

The redesign was checked against every resource feature already on the books — ADR-0089 `ResourceTools` (agent tools advertised by a resource), and the 1.1 line `InfraProvider` / `ConnectionAware` / `ResourceGroup` / `Authenticate<C>`, plus M12.4 bind-population and M12.4.2 frontier cleanup. **None conflicts**, because `Topology` widens *only* "how an already-built, already-authorized instance is leased under concurrency." Every other feature sits on the `Provider`/Manager side of the `InstanceStore` line — the same line that keeps the credential/tenant seam safe (§2.6). Several are *enabled* by the new contract.

| Planned feature | Where it lives | Interaction with this spec |
|---|---|---|
| **`ResourceTools`** (ADR-0089) | a separate `#[async_trait] ResourceTools` on the resource + a `tools()` accessor on the sealed `AnyManagedResource` | **Orthogonal.** A *capability* read off the guard; `Topology` is *lease policy*. `ResourceGuard` is kept (derefs to `Instance`), so `invoke(&self, …)`-borrows-the-guard works unchanged. `AnyManagedResource` gains `tools()` (a framework-side method on the sealed trait — exactly what sealing permits); the open topology uses a *separate* `ErasedTopology`, so the two erasures don't collide. Mechanical only: the bound `R: Resource + ResourceTools` becomes `R: Provider + ResourceTools`. |
| **`InfraProvider`** (resource-on-resource dependency) | `Provider` + Manager dependency resolution (the `#[credential]` pattern, for a resource dep) | **Orthogonal, same pattern as credentials (§2.6).** Resource deps resolve before `create`, outside `Topology`; the dependency's own topology is independent. |
| **`ConnectionAware`** (disconnect detection) | `Provider::check` + the admission axis | **Enabled.** Maps directly onto `AdmissionPhase::Recovering`/`Tainted`, `check_cost` (A11), and parent-generation recovery (A12). |
| **`ResourceGroup`** (multi-resource atomic acquire / txn) | a composition *over* `acquire` / `try_reserve` | **Enabled by the ticket model.** All-or-nothing acquire is `try_reserve` over N resources; if any returns `Unavailable`, drop the held `Ticket`s (release) and park — a clean two-phase group reservation the boolean readiness model could not express. |
| **`Authenticate<C>`** | credential / `Provider` side | **Orthogonal** — credential seam (§2.6). |
| **M12.4 bind-population** (production credential→slot resolver) | engine + Manager | **Orthogonal** — resolution feeds `create`; `Topology` never sees it. |
| **M12.4.2 frontier per-branch cleanup** | engine scope teardown + guard drop | **Composes** with scoped release; the async/fallible release path (A5/A6) makes branch cleanup truthful. |

**The load-bearing reason none conflicts:** a resource can be `Pooled` **and** advertise `ResourceTools` **and** depend on another resource via `InfraProvider` **and** be `ConnectionAware` — four independent axes that never touch the topology trait. Opening `Topology` does not foreclose, complicate, or reroute any of them.

---

**Net:** `Topology` becomes an **open trait** made safe by the **framework-owns-storage** rule that defuses the adversary's bleed/erasure/revoke objections; the admission seam is **ticket-based `try_reserve`** (gate) + advisory `phase()`/`load()` (route/diagnose), resolving TOCTOU by construction; **`Bounded` is restored** with a runtime cap; durability/streaming/renewal are deferred to the engine tier or 1.1. The contract additions cluster around one truth the catalogs forced: **`on_release`/`destroy`/`check` are real async work, not `Drop` glue**, and **admission is an orthogonal axis fed by try-reserve outcomes, not a gauge**. Vocabulary: **Resource → Provider → Instance → Topology → ResourceGuard**.
