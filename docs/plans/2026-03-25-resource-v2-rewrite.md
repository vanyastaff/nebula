# nebula-resource v2 Full Rewrite Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Rewrite nebula-resource from pool-only architecture to the full 7-topology design per `crates/resource/plans/resource-hld.md`.

**Architecture:** Big-bang phased rewrite. Delete all existing code (12k lines), rebuild from primitives up. Each phase compiles independently. The HLD defines 5 architecture layers: Primitive → Recovery → Runtime → Manager → Integration. We build bottom-up.

**Tech Stack:** Rust 1.93, tokio, arc-swap, dashmap, thiserror, serde, tokio-util (CancellationToken), smallvec

**Key rename map (current → new):**

| Current | New |
|---------|-----|
| `Resource` trait (create/is_reusable/is_broken/recycle/prepare/destroy) | `Resource` trait (create/check/shutdown/destroy + 5 assoc types) |
| `Guard<T, F>` | `ResourceHandle<R>` (Owned/Guarded/Shared) |
| `Manager` (pool registry) | `Manager` (topology-aware registry + recovery + resilience) |
| `Pool<R>` (only topology) | `TopologyRuntime<R>` (7 variants) |
| `Error` (11 variants, ErrorCategory) | `Error` (6 ErrorKind variants + ErrorScope) |
| `Context` (scope/execution/cancel) | `Ctx` trait (scope/execution_id/cancel/extensions) |
| N/A | `RecoveryGate`, `ReleaseQueue`, `ManagedResource<R>` |

**Consumer impact:** nebula-engine (blocked, not using resource yet), nebula-action (ResourceProvider trait). Both will need updates but are minimal since resource system wasn't wired into execution yet.

---

## Phase 1: Primitives (Tasks 1–8)

Foundation types that everything else depends on. After this phase: `cargo check -p nebula-resource` passes with the new type system.

### Task 1: Clean slate — delete old code, set up skeleton

**Files:**
- Delete: ALL files in `crates/resource/src/` except keep `lib.rs`
- Delete: ALL files in `crates/resource/tests/`
- Delete: ALL files in `crates/resource/examples/`
- Delete: ALL files in `crates/resource/benches/`
- Rewrite: `crates/resource/src/lib.rs` — empty skeleton with module declarations
- Keep: `crates/resource/Cargo.toml` (update deps as needed)
- Keep: `crates/resource/plans/` (HLD reference)

The new `lib.rs` should declare empty modules that will be filled in subsequent tasks. This establishes the module structure from the HLD.

After this task: crate compiles (all modules empty).

**Commit:** `refactor(resource): clean slate for v2 rewrite`

---

### Task 2: Error types — Error, ErrorKind, ErrorScope

**Files:**
- Create: `crates/resource/src/error.rs`

New error system with 6 ErrorKind variants (not 11):

```rust
pub enum ErrorKind {
    Transient,
    Permanent,
    Exhausted { retry_after: Option<Duration> },
    Backpressure,
    NotFound,
    Cancelled,
}

pub enum ErrorScope {
    Resource,
    Target { id: String },
}

pub struct Error {
    kind: ErrorKind,
    scope: ErrorScope,
    message: String,
    resource_key: Option<ResourceKey>,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}
```

Methods: `kind()`, `scope()`, `resource_key()`, `is_retryable()`, `retry_after()`.
Implement `std::error::Error`, `Display`, `From` helpers.

Add unit tests for each ErrorKind, Display, retryable logic.

**Commit:** `feat(resource): add Error with 6 ErrorKind variants and ErrorScope`

---

### Task 3: Ctx trait + BasicCtx

**Files:**
- Create: `crates/resource/src/ctx.rs`

```rust
pub trait Ctx: Send + Sync {
    fn scope(&self) -> &ScopeLevel;
    fn execution_id(&self) -> &ExecutionId;
    fn cancel_token(&self) -> &CancellationToken;
    fn ext<T: Send + Sync + 'static>(&self) -> Option<&T>;
}
```

Plus `BasicCtx` struct implementing it, with `Extensions` map (type-map via `HashMap<TypeId, Box<dyn Any + Send + Sync>>`).

`ScopeLevel` enum: Global, Organization(id), Project(id), Workflow(id), Execution(id).

**Commit:** `feat(resource): add Ctx trait, BasicCtx, ScopeLevel, Extensions`

---

### Task 4: Resource trait + ResourceConfig + Credential

**Files:**
- Create: `crates/resource/src/resource.rs`

The core Resource trait with 5 associated types:

```rust
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Runtime: Send + Sync + 'static;
    type Lease: Send + Sync + 'static;
    type Error: std::error::Error + Send + Sync + Into<Error> + 'static;
    type Credential: Credential;
    const KEY: ResourceKey;

    fn create(&self, config: &Self::Config, credential: &Self::Credential, ctx: &dyn Ctx)
        -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;
    fn check(&self, runtime: &Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }
    fn shutdown(&self, runtime: &Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }
    fn destroy(&self, runtime: Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }
    fn metadata() -> ResourceMetadata { ResourceMetadata::from_key(&Self::KEY) }
}
```

Plus `ResourceConfig` trait (validate + fingerprint) and `Credential` trait (KIND const + impl for `()`).

`ResourceMetadata` struct with key, name, description, tags.

**Commit:** `feat(resource): add Resource trait with 5 associated types, ResourceConfig, Credential`

---

### Task 5: ResourceHandle — unified RAII handle

**Files:**
- Create: `crates/resource/src/handle.rs`

3-variant handle replacing the old `Guard<T, F>`:

```rust
pub struct ResourceHandle<R: Resource> { inner: HandleInner<R>, resource_key: ResourceKey, topology_tag: &'static str }

enum HandleInner<R: Resource> {
    Owned(R::Lease),
    Guarded { value: Option<R::Lease>, on_release: Option<Box<dyn FnOnce(R::Lease, bool) + Send>>, tainted: bool, acquired_at: Instant, generation: u64 },
    Shared { value: Arc<R::Lease>, on_release: Option<Box<dyn FnOnce(bool) + Send>>, tainted: bool, acquired_at: Instant, generation: u64 },
}
```

Implement: `Deref<Target = R::Lease>`, `Drop` (dispatches release), `taint()`, `detach()`, `hold_duration()`, `resource_key()`, `topology_tag()`.

Add tests for all 3 variants, Deref, Drop behavior, taint, detach.

**Commit:** `feat(resource): add ResourceHandle with Owned/Guarded/Shared variants`

---

### Task 6: Cell<T> — lock-free ArcSwap cell for Resident

**Files:**
- Create: `crates/resource/src/cell.rs`

Thin wrapper around `arc_swap::ArcSwapOption<T>`:

```rust
pub struct Cell<T>(ArcSwapOption<T>);
```

Methods: `store(Arc<T>)`, `load() -> Option<Arc<T>>`, `take() -> Option<Arc<T>>`, `is_some()`.

**Commit:** `feat(resource): add Cell<T> for lock-free Resident topology`

---

### Task 7: ReleaseQueue — async cleanup workers

**Files:**
- Create: `crates/resource/src/release_queue.rs`

N primary workers + 1 fallback worker. Each worker owns its receiver (no Mutex on hot path).

```rust
pub struct ReleaseQueue { senders: Vec<mpsc::Sender<ReleaseTask>>, fallback_tx: mpsc::Sender<ReleaseTask>, next: AtomicUsize }
pub struct ReleaseQueueHandle { join_handles: Vec<JoinHandle<()>> }
type ReleaseTask = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;
```

Methods: `new(worker_count)`, `submit(task)`, `shutdown()`.
30s timeout per task. Metrics: submitted, fallback_used, dropped, timed_out.

**Commit:** `feat(resource): add ReleaseQueue with N primary + 1 fallback workers`

---

### Task 8: ResourceStatus + ResourcePhase + AcquireOptions

**Files:**
- Create: `crates/resource/src/state.rs`
- Create: `crates/resource/src/options.rs`

ResourcePhase (6 states): Initializing, Ready, Reloading, Draining, ShuttingDown, Failed.
ResourceStatus: phase + generation (u64) + last_error.

AcquireOptions: intent (Standard/LongRunning/Streaming/Prefetch/Critical), deadline, tags.

**Commit:** `feat(resource): add ResourceStatus, ResourcePhase, AcquireOptions`

---

### Task 9: Wire lib.rs — Phase 1 compilation check

**Files:**
- Rewrite: `crates/resource/src/lib.rs`

Wire all modules from tasks 2–8. Set up re-exports and prelude. Run:
- `cargo check -p nebula-resource`
- `cargo clippy -p nebula-resource -- -D warnings`

Fix any compilation errors.

**Commit:** `feat(resource): wire Phase 1 modules, crate compiles`

---

## Phase 2: Topology Traits (Tasks 10–16)

Define the 7 topology traits. These are trait definitions only — no runtime implementations yet.

### Task 10: Pooled trait

**Files:**
- Create: `crates/resource/src/topology/mod.rs`
- Create: `crates/resource/src/topology/pooled.rs`

```rust
pub trait Pooled: Resource {
    fn is_broken(&self, runtime: &Self::Runtime) -> BrokenCheck { BrokenCheck::Healthy }
    fn recycle(&self, runtime: &Self::Runtime, metrics: &InstanceMetrics) -> impl Future<Output = Result<RecycleDecision, Self::Error>> + Send { async { Ok(RecycleDecision::Keep) } }
    fn prepare(&self, runtime: &Self::Runtime, ctx: &dyn Ctx) -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }
}
```

Plus `BrokenCheck` (Healthy/Broken), `RecycleDecision` (Keep/Drop), `InstanceMetrics`.
Pool config types: `pool::Config` with min_size, max_size, idle_timeout, max_lifetime, strategy, warmup, test_on_checkout, maintenance_interval, create_timeout.

**Commit:** `feat(resource): add Pooled trait with BrokenCheck, RecycleDecision, pool::Config`

---

### Task 11: Resident trait

**Files:**
- Create: `crates/resource/src/topology/resident.rs`

```rust
pub trait Resident: Resource where Self::Lease: Clone {
    fn is_alive_sync(&self, _runtime: &Self::Runtime) -> bool { true }
    fn stale_after(&self) -> Option<Duration> { None }
}
```

Plus `resident::Config` (check_interval, recreate_on_failure).

**Commit:** `feat(resource): add Resident trait`

---

### Task 12: Service trait

**Files:**
- Create: `crates/resource/src/topology/service.rs`

```rust
pub trait Service: Resource {
    const TOKEN_MODE: TokenMode = TokenMode::Cloned;
    fn acquire_token(&self, runtime: &Self::Runtime, ctx: &dyn Ctx) -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;
    fn release_token(&self, runtime: &Self::Runtime, token: Self::Lease) -> impl Future<Output = Result<(), Self::Error>> + Send { ... }
}
```

Plus `TokenMode` (Cloned/Tracked), `service::Config`.

**Commit:** `feat(resource): add Service trait with TokenMode`

---

### Task 13: Transport trait

**Files:**
- Create: `crates/resource/src/topology/transport.rs`

```rust
pub trait Transport: Resource {
    fn open_session(&self, transport: &Self::Runtime, ctx: &dyn Ctx) -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;
    fn close_session(&self, transport: &Self::Runtime, session: Self::Lease, healthy: bool) -> impl Future<Output = Result<(), Self::Error>> + Send { ... }
    fn keepalive(&self, transport: &Self::Runtime) -> impl Future<Output = Result<(), Self::Error>> + Send { ... }
}
```

Plus `transport::Config` (max_sessions, keepalive_interval).

**Commit:** `feat(resource): add Transport trait`

---

### Task 14: Exclusive trait

**Files:**
- Create: `crates/resource/src/topology/exclusive.rs`

```rust
pub trait Exclusive: Resource {
    fn reset(&self, runtime: &Self::Runtime) -> impl Future<Output = Result<(), Self::Error>> + Send { ... }
}
```

Plus `exclusive::Config`.

**Commit:** `feat(resource): add Exclusive trait`

---

### Task 15: EventSource trait (secondary)

**Files:**
- Create: `crates/resource/src/topology/event_source.rs`

```rust
pub trait EventSource: Resource {
    type Event: Send + Clone + 'static;
    type Subscription: Send + 'static;
    fn subscribe(&self, runtime: &Self::Runtime, ctx: &dyn Ctx) -> impl Future<Output = Result<Self::Subscription, Self::Error>> + Send;
    fn recv(&self, subscription: &mut Self::Subscription) -> impl Future<Output = Result<Self::Event, Self::Error>> + Send;
}
```

Plus `event_source::Config`.

**Commit:** `feat(resource): add EventSource trait`

---

### Task 16: Daemon trait (secondary)

**Files:**
- Create: `crates/resource/src/topology/daemon.rs`

```rust
pub trait Daemon: Resource {
    fn run(&self, runtime: &Self::Runtime, ctx: &dyn Ctx, cancel: CancellationToken) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
```

Plus `daemon::Config` (restart_policy: Never/OnFailure/Always, recreate_budget).

Wire all topology modules in `topology/mod.rs` and update `lib.rs`.

**Commit:** `feat(resource): add Daemon trait, wire all 7 topology traits`

---

## Phase 3: Recovery Layer (Tasks 17–19)

### Task 17: RecoveryGate — CAS state machine

**Files:**
- Create: `crates/resource/src/recovery/mod.rs`
- Create: `crates/resource/src/recovery/gate.rs`

CAS-based state machine: Idle → InProgress → Failed/PermanentlyFailed.
Uses `ArcSwap::compare_and_swap`.

Key types: `GateState`, `RecoveryTicket` (Drop guard auto-fails), `RecoveryWaiter` (Notify).

Methods: `try_begin()` → `Ok(RecoveryTicket)` or `Err(RecoveryWaiter)`.
`RecoveryTicket::resolve()` → Idle. `RecoveryTicket::fail_transient()` → Failed with backoff.

Tests: concurrent access, ticket drop auto-fail, backoff escalation, permanent failure.

**Commit:** `feat(resource): add RecoveryGate with CAS state machine`

---

### Task 18: RecoveryGroup — shared gates per backend

**Files:**
- Create: `crates/resource/src/recovery/group.rs`

```rust
pub struct RecoveryGroupRegistry { groups: DashMap<RecoveryGroupKey, Arc<RecoveryGate>> }
```

Multiple resources on same backend share one gate. Register via `RecoveryGroupKey`.

**Commit:** `feat(resource): add RecoveryGroup for shared backend recovery`

---

### Task 19: AcquireResilience — timeout/retry/circuit-breaker wrapper

**Files:**
- Create: `crates/resource/src/integration/mod.rs`
- Create: `crates/resource/src/integration/resilience.rs`

```rust
pub struct AcquireResilience {
    pub timeout: Option<Duration>,
    pub retry: Option<AcquireRetryConfig>,
    pub circuit_breaker: Option<AcquireCircuitBreakerPreset>,
}
```

Presets: Standard (5 failures/30s), Fast (3/10s), Slow (10/60s).
Wraps acquire with nebula-resilience pipeline.

**Commit:** `feat(resource): add AcquireResilience with presets`

---

## Phase 4: Topology Runtimes (Tasks 20–27)

Implement the runtime for each topology — the actual acquire/release logic.

### Task 20: TopologyRuntime enum + ManagedResource shell

**Files:**
- Create: `crates/resource/src/runtime/mod.rs`
- Create: `crates/resource/src/runtime/managed.rs`

TopologyRuntime<R> enum with 7 variants (stubs initially).
ManagedResource<R> struct: resource + config (ArcSwap) + topology + recovery_gate + release_queue + metrics + cancel + generation + status.

**Commit:** `feat(resource): add TopologyRuntime enum and ManagedResource shell`

---

### Task 21: Pool runtime — idle queue, create, checkout, recycle, maintenance

**Files:**
- Create: `crates/resource/src/runtime/pool.rs`

The most complex topology. Implements:
- Idle queue (VecDeque with InstanceMetadata)
- Acquire: checkout from idle → is_broken? → test_on_checkout? → prepare(ctx) → return Guarded handle. If no idle → create() → prepare(ctx). If full → wait.
- Release: via ReleaseQueue → is_broken? → stale fingerprint? → max_lifetime? → recycle() → Keep/Drop
- Maintenance: background task trimming idle connections, checking health
- Warmup: Sequential/Parallel/Staggered strategies

Tests: acquire/release cycle, pool exhaustion, broken detection, recycle, maintenance.

**Commit:** `feat(resource): implement Pool runtime with idle queue, acquire, release, maintenance`

---

### Task 22: Resident runtime — Cell + Clone acquire

**Files:**
- Create: `crates/resource/src/runtime/resident.rs`

Simple: Cell<R::Runtime> + create on first acquire, clone on subsequent.
is_alive_sync() checked on acquire. If dead → destroy + recreate.
stale_after() → background check interval.

**Commit:** `feat(resource): implement Resident runtime with Cell + Clone`

---

### Task 23: Service runtime — token acquire/release

**Files:**
- Create: `crates/resource/src/runtime/service.rs`

Long-lived runtime. acquire_token() → Owned (Cloned) or Guarded (Tracked) handle.
Reload: new runtime, old drains via Arc refcount.

**Commit:** `feat(resource): implement Service runtime with token modes`

---

### Task 24: Transport runtime — session multiplexing

**Files:**
- Create: `crates/resource/src/runtime/transport.rs`

Shared connection + open_session/close_session. Bounded by max_sessions semaphore.
keepalive() on background interval.

**Commit:** `feat(resource): implement Transport runtime with session multiplexing`

---

### Task 25: Exclusive runtime — semaphore + reset

**Files:**
- Create: `crates/resource/src/runtime/exclusive.rs`

Semaphore(1). Acquire → permit + Arc<Lease> → Shared handle.
Release → reset() + drop permit.

**Commit:** `feat(resource): implement Exclusive runtime with semaphore`

---

### Task 26: EventSource runtime — subscribe/recv

**Files:**
- Create: `crates/resource/src/runtime/event_source.rs`

Secondary topology. subscribe() returns Subscription, recv() pulls events.

**Commit:** `feat(resource): implement EventSource runtime`

---

### Task 27: Daemon runtime — background run loop with restart

**Files:**
- Create: `crates/resource/src/runtime/daemon.rs`

Background run() loop with CancellationToken. RestartPolicy: Never/OnFailure/Always.
Two-level restart: retry run() with same runtime (Level 1), recreate runtime (Level 2, budget-limited).

**Commit:** `feat(resource): implement Daemon runtime with restart policy`

---

## Phase 5: Manager + Registry (Tasks 28–33)

### Task 28: Registry — type-erased, scope-aware storage

**Files:**
- Create: `crates/resource/src/registry/mod.rs`
- Create: `crates/resource/src/registry/lookup.rs`

DashMap-based registry with two indices:
- `by_type: DashMap<(TypeId, ResourceId), SmallVec<[ScopedRuntime; 4]>>`
- `by_key: DashMap<ResourceKey, TypeId>`

Scope-aware lookup: find most-specific compatible scope.

**Commit:** `feat(resource): add Registry with scope-aware lookup`

---

### Task 29: RegistrationBuilder — typestate

**Files:**
- Create: `crates/resource/src/manager/mod.rs`
- Create: `crates/resource/src/manager/builder.rs`

Typestate: NeedsConfig → NeedsId → NeedsTopology → Ready.
.pool() available only if R: Pooled. .resident() only if R: Resident + Lease: Clone.
.also_event_source() / .also_daemon() only on Ready if R impls those traits.

**Commit:** `feat(resource): add RegistrationBuilder with typestate`

---

### Task 30: Manager — register + acquire dispatch

**Files:**
- Create: `crates/resource/src/manager/acquire.rs`
- Modify: `crates/resource/src/manager/mod.rs`

Manager struct with registry, recovery_groups, cancel token.
`register()` → builder. `acquire::<R>(ctx)` → recovery gate check → resilience → topology dispatch → ResourceHandle.

**Commit:** `feat(resource): implement Manager register + acquire`

---

### Task 31: ShutdownOrchestrator

**Files:**
- Create: `crates/resource/src/manager/shutdown.rs`

Phased shutdown: cancel → reverse registration order → drain release queues.
Timeout budgets per phase.

**Commit:** `feat(resource): add ShutdownOrchestrator with phased drain`

---

### Task 32: Events + Metrics integration

**Files:**
- Create: `crates/resource/src/events.rs`
- Create: `crates/resource/src/metrics.rs`

ResourceEvent enum: Registered, Removed, AcquireSuccess, AcquireFailed, Released, HealthChanged, ConfigReloaded.
ResourceMetrics: counters (acquire_total, error_total) + histograms (acquire_duration, hold_duration).

**Commit:** `feat(resource): add ResourceEvent and ResourceMetrics`

---

### Task 33: Wire Phase 5 — full compilation + integration tests

**Files:**
- Rewrite: `crates/resource/src/lib.rs` — wire all modules, re-exports, prelude

Run full check:
- `cargo check -p nebula-resource`
- `cargo clippy -p nebula-resource -- -D warnings`
- `cargo nextest run -p nebula-resource`

Write integration tests:
- Register + acquire Pool resource → use → release
- Register Resident resource → acquire (clone) → use
- Manager shutdown ordering
- RecoveryGate concurrent access

**Commit:** `feat(resource): wire all modules, add integration tests`

---

## Phase 6: Testing + Docs + Consumers (Tasks 34–38)

### Task 34: Example resources — TestPool, TestResident, TestService

**Files:**
- Create: `crates/resource/examples/basic_pool.rs`
- Create: `crates/resource/examples/basic_resident.rs`
- Create: `crates/resource/examples/basic_service.rs`

Minimal working examples for the 3 most common topologies.

**Commit:** `docs(resource): add example resources for Pool, Resident, Service`

---

### Task 35: README.md

**Files:**
- Create: `crates/resource/README.md`

Cover: what it does, 7 topologies, action author view, resource author view, registration builder, error classification, verify locally.

**Commit:** `docs(resource): add README.md`

---

### Task 36: Update consumers — nebula-action, nebula-engine

**Files:**
- Modify: consumer crates that reference nebula-resource types

Update ResourceProvider trait, any integration points.

**Commit:** `refactor: update consumers for resource v2 API`

---

### Task 37: Full workspace verification

Run:
- `cargo fmt`
- `cargo clippy --workspace -- -D warnings`
- `cargo nextest run --workspace`
- `cargo test --workspace --doc`

**Commit:** `chore: fix workspace issues after resource v2 rewrite`

---

### Task 38: Update context files

**Files:**
- Modify: `.claude/crates/resource.md`
- Modify: `.claude/active-work.md`

**Commit:** `docs(claude): update resource context file for v2`

---

## Summary

| Phase | Tasks | Description |
|-------|-------|-------------|
| 1 Primitives | 1–9 | Error, Ctx, Resource trait, Handle, Cell, ReleaseQueue, Status |
| 2 Topology Traits | 10–16 | Pooled, Resident, Service, Transport, Exclusive, EventSource, Daemon |
| 3 Recovery | 17–19 | RecoveryGate, RecoveryGroup, AcquireResilience |
| 4 Runtimes | 20–27 | Pool, Resident, Service, Transport, Exclusive, EventSource, Daemon runtimes |
| 5 Manager | 28–33 | Registry, Builder, Manager, Shutdown, Events, Metrics |
| 6 Docs/Tests | 34–38 | Examples, README, consumers, workspace check, context files |

**Total: 38 tasks across 6 phases.**
