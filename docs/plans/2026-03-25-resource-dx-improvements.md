# Resource DX Improvements Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Improve nebula-resource developer experience based on DX Tester and SDK User feedback: add Debug, re-export macro, document trait bounds, add convenience register methods, rewrite all 7 stale v1 docs for v2 API.

**Architecture:** Phase 1 (quick wins) modifies existing types with minimal blast radius. Phase 2 adds convenience methods on Manager that delegate to the existing `register()`. Phase 3 rewrites all docs in `crates/resource/docs/` using the actual v2 source as ground truth.

**Tech Stack:** Rust 1.93, RPITIT traits, tokio

---

### Task 1: Add Debug impl for ResourceHandle

**Files:**
- Modify: `crates/resource/src/handle.rs`

**Step 1: Add the Debug impl**

After the `Drop` impl (around line 312), add:

```rust
impl<R: Resource> std::fmt::Debug for ResourceHandle<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mode = match &self.inner {
            HandleInner::Owned(_) => "Owned",
            HandleInner::Guarded { .. } => "Guarded",
            HandleInner::Shared { .. } => "Shared",
        };
        f.debug_struct("ResourceHandle")
            .field("resource_key", &self.resource_key)
            .field("topology_tag", &self.topology_tag)
            .field("mode", &mode)
            .finish()
    }
}
```

**Step 2: Run tests**

```bash
rtk cargo nextest run -p nebula-resource
```

**Step 3: Commit**

```bash
git add crates/resource/src/handle.rs
git commit -m "feat(resource): add Debug impl for ResourceHandle

Shows resource_key, topology_tag, and mode (Owned/Guarded/Shared)
without requiring R::Lease: Debug. Unblocks .unwrap_err() and
.expect_err() in test code.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Re-export resource_key! macro and document trait bounds

**Files:**
- Modify: `crates/resource/src/lib.rs`
- Modify: `crates/resource/src/topology/pooled.rs`
- Modify: `crates/resource/src/topology/resident.rs`
- Modify: `crates/resource/src/topology/service.rs`
- Modify: `crates/resource/src/topology/transport.rs`
- Modify: `crates/resource/src/topology/exclusive.rs`
- Modify: `crates/resource/src/runtime/pool.rs`

**Step 1: Re-export the macro in lib.rs**

In `crates/resource/src/lib.rs`, change line 75:

```rust
pub use nebula_core::{ExecutionId, ResourceKey, WorkflowId};
```

to:

```rust
pub use nebula_core::{ExecutionId, ResourceKey, WorkflowId, resource_key};
```

**Step 2: Add acquire bounds docs to Pooled trait**

In `crates/resource/src/topology/pooled.rs`, update the `Pooled` trait doc comment:

```rust
/// Pool topology — N interchangeable stateful instances with
/// checkout/recycle/destroy.
///
/// Implementors extend [`Resource`] with pool-aware lifecycle hooks:
/// a sync broken check (for the `Drop` path), an async recycle step,
/// and an optional per-checkout prepare step.
///
/// # Acquire bounds
///
/// [`Manager::acquire_pooled`](crate::Manager::acquire_pooled) requires:
/// - `R: Clone + Send + Sync + 'static`
/// - `R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static`
/// - `R::Lease: Into<R::Runtime> + Send + 'static`
///
/// If `Runtime` and `Lease` are the same type, the blanket `impl<T> From<T> for T`
/// satisfies both conversion bounds automatically.
pub trait Pooled: Resource {
```

**Step 3: Add acquire bounds docs to Resident trait**

In `crates/resource/src/topology/resident.rs`:

```rust
/// Resident topology — one shared instance, clone on acquire.
///
/// The runtime is created once and shared across all callers via `Clone`.
/// Suitable for stateless or internally-pooled clients (e.g., `reqwest::Client`).
///
/// # Acquire bounds
///
/// [`Manager::acquire_resident`](crate::Manager::acquire_resident) requires:
/// - `R: Send + Sync + 'static`
/// - `R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static`
/// - `R::Lease: Clone + Send + 'static`
///
/// If `Runtime` and `Lease` are the same type, the blanket `impl<T> From<T> for T`
/// satisfies the conversion bound automatically.
pub trait Resident: Resource
```

**Step 4: Add acquire bounds docs to Service, Transport, Exclusive traits**

In `crates/resource/src/topology/service.rs`:

```rust
/// Service topology — long-lived runtime, short-lived tokens for callers.
///
/// The runtime lives for the duration of the resource, and callers acquire
/// lightweight tokens (e.g., API keys, session handles) scoped to their
/// execution context.
///
/// # Acquire bounds
///
/// [`Manager::acquire_service`](crate::Manager::acquire_service) requires:
/// - `R: Clone + Send + Sync + 'static`
/// - `R::Runtime: Send + Sync + 'static`
/// - `R::Lease: Send + 'static`
pub trait Service: Resource {
```

In `crates/resource/src/topology/transport.rs`:

```rust
/// Transport topology — shared connection, multiplexed sessions.
///
/// A single long-lived transport (e.g., HTTP/2, gRPC channel, AMQP connection)
/// multiplexes many short-lived sessions (streams, channels) for callers.
///
/// # Acquire bounds
///
/// [`Manager::acquire_transport`](crate::Manager::acquire_transport) requires:
/// - `R: Clone + Send + Sync + 'static`
/// - `R::Runtime: Send + Sync + 'static`
/// - `R::Lease: Send + 'static`
pub trait Transport: Resource {
```

In `crates/resource/src/topology/exclusive.rs`:

```rust
/// Exclusive topology — one caller at a time via semaphore.
///
/// The runtime is protected by a semaphore permit. Only one caller can
/// hold the lease at a time. Suitable for resources that are not
/// concurrency-safe (e.g., serial ports, single-writer databases).
///
/// # Acquire bounds
///
/// [`Manager::acquire_exclusive`](crate::Manager::acquire_exclusive) requires:
/// - `R: Send + Sync + 'static`
/// - `R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static`
/// - `R::Lease: Send + 'static`
pub trait Exclusive: Resource {
```

**Step 5: Document fingerprint param on PoolRuntime::new**

In `crates/resource/src/runtime/pool.rs`, update the `new` method doc:

```rust
    /// Creates a new pool runtime with the given configuration.
    ///
    /// The `fingerprint` is a config-change detection token. When
    /// [`Manager::reload_config`](crate::Manager::reload_config) is called,
    /// idle instances whose fingerprint differs from the current one are
    /// evicted. Use `0` as the initial value; the manager updates it
    /// automatically on reload. Implement [`ResourceConfig::fingerprint()`]
    /// on your config type to enable change detection.
    pub fn new(config: Config, fingerprint: u64) -> Self {
```

**Step 6: Run tests**

```bash
rtk cargo nextest run -p nebula-resource && rtk cargo test --doc -p nebula-resource
```

**Step 7: Commit**

```bash
git add crates/resource/src/lib.rs crates/resource/src/topology/ crates/resource/src/runtime/pool.rs
git commit -m "docs(resource): re-export resource_key! macro, document acquire bounds

- Re-export resource_key! from nebula_resource (was only in nebula_core)
- Add '# Acquire bounds' section to all 5 topology traits documenting
  the where-clause bounds required by Manager::acquire_*
- Document PoolRuntime::new fingerprint parameter

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Add convenience register methods

**Files:**
- Modify: `crates/resource/src/manager.rs`

**Step 1: Add register_pooled**

Add after the existing `register` method:

```rust
    /// Registers a pooled resource with sensible defaults.
    ///
    /// Shorthand for [`register`](Self::register) with `credential = ()`,
    /// `scope = Global`, `resilience = None`, `recovery_gate = None`.
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_resource::{Manager, PoolConfig, PoolRuntime};
    /// # fn example(manager: &Manager) {
    /// // manager.register_pooled(my_resource, my_config, PoolConfig::default());
    /// # }
    /// ```
    pub fn register_pooled<R>(
        &self,
        resource: R,
        config: R::Config,
        pool_config: crate::topology::pooled::config::Config,
    ) -> Result<(), Error>
    where
        R: Resource<Credential = ()>,
    {
        let fingerprint = config.fingerprint();
        self.register(
            resource,
            config,
            (),
            ScopeLevel::Global,
            TopologyRuntime::Pool(crate::runtime::pool::PoolRuntime::<R>::new(pool_config, fingerprint)),
            None,
            None,
        )
    }
```

**Step 2: Add register_resident**

```rust
    /// Registers a resident resource with sensible defaults.
    ///
    /// Shorthand for [`register`](Self::register) with `credential = ()`,
    /// `scope = Global`, `resilience = None`, `recovery_gate = None`.
    pub fn register_resident<R>(
        &self,
        resource: R,
        config: R::Config,
        resident_config: crate::topology::resident::config::Config,
    ) -> Result<(), Error>
    where
        R: Resource<Credential = ()>,
    {
        self.register(
            resource,
            config,
            (),
            ScopeLevel::Global,
            TopologyRuntime::Resident(crate::runtime::resident::ResidentRuntime::<R>::new(resident_config)),
            None,
            None,
        )
    }
```

**Step 3: Add register_service, register_exclusive**

```rust
    /// Registers a service resource with sensible defaults.
    pub fn register_service<R>(
        &self,
        resource: R,
        config: R::Config,
        service_config: crate::topology::service::config::Config,
    ) -> Result<(), Error>
    where
        R: Resource<Credential = ()>,
    {
        self.register(
            resource,
            config,
            (),
            ScopeLevel::Global,
            TopologyRuntime::Service(crate::runtime::service::ServiceRuntime::<R>::new(service_config)),
            None,
            None,
        )
    }

    /// Registers an exclusive resource with sensible defaults.
    pub fn register_exclusive<R>(
        &self,
        resource: R,
        config: R::Config,
        exclusive_config: crate::topology::exclusive::config::Config,
    ) -> Result<(), Error>
    where
        R: Resource<Credential = ()>,
    {
        self.register(
            resource,
            config,
            (),
            ScopeLevel::Global,
            TopologyRuntime::Exclusive(crate::runtime::exclusive::ExclusiveRuntime::<R>::new(exclusive_config)),
            None,
            None,
        )
    }
```

**Step 4: Run tests**

```bash
rtk cargo nextest run -p nebula-resource && rtk cargo clippy -p nebula-resource -- -D warnings
```

**Step 5: Commit**

```bash
git add crates/resource/src/manager.rs
git commit -m "feat(resource): add convenience register_pooled/resident/service/exclusive

Shorthands that default credential=(), scope=Global, resilience=None,
recovery_gate=None. Reduces the 7-parameter register() ceremony to
3 parameters for the 90% case.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Rewrite README.md for v2 API

**Files:**
- Rewrite: `crates/resource/docs/README.md`

**Step 1: Write the new README**

The README must cover: overview, topology table, quick start (v2 API with actual compiling examples), feature matrix, crate layout. Use ONLY types that exist in the current `lib.rs` re-exports.

Key types to reference:
- `Resource` trait (5 associated types: Config, Runtime, Lease, Error, Credential)
- `Pooled`, `Resident`, `Service`, `Transport`, `Exclusive` topology traits
- `Manager` with `register_pooled()` / `acquire_pooled()` convenience
- `ResourceHandle<R>` with RAII, `taint()`, `detach()`
- `BasicCtx`, `ScopeLevel`
- `ErrorKind` (Transient/Permanent/Exhausted/Backpressure/NotFound/Cancelled)
- `AcquireResilience` presets
- `RecoveryGate`
- `ResourceEvent` + `subscribe_events()`
- `ClassifyError` derive macro

Quick Start example pattern (from integration tests):
```rust
use nebula_resource::{
    resource_key, Resource, ResourceConfig, ResourceMetadata,
    Manager, PoolConfig, AcquireOptions, BasicCtx, ScopeLevel,
};
```

**Step 2: Verify all referenced types exist**

```bash
rtk cargo test --doc -p nebula-resource
```

**Step 3: Commit**

```bash
git add crates/resource/docs/README.md
git commit -m "docs(resource): rewrite README.md for v2 API

Complete rewrite replacing stale v1 docs with current v2 types:
Resource trait, topology system, Manager convenience methods,
ResourceHandle, error model, resilience presets.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Rewrite architecture.md for v2

**Files:**
- Rewrite: `crates/resource/docs/architecture.md`

Cover: design decisions (topology-per-trait, RPITIT, type erasure at Manager, RAII handles with separate permit), module map (current files), data flow (acquire path per topology, release path via ReleaseQueue), dependency graph (nebula-core, nebula-resource-macros only), invariants.

**Commit message:** `docs(resource): rewrite architecture.md for v2 topology system`

---

### Task 6: Rewrite api-reference.md for v2

**Files:**
- Rewrite: `crates/resource/docs/api-reference.md`

Cover ALL public types from `lib.rs` re-exports with signatures. Group by: Core traits (Resource, ResourceConfig, Credential), Topology traits (Pooled, Resident, Service, Transport, Exclusive, EventSource, Daemon), Handle (ResourceHandle), Manager, Error model, Context (Ctx, BasicCtx, ScopeLevel), Options (AcquireOptions, AcquireIntent), Resilience, Recovery, Events, Metrics, Runtime types, State.

**Commit message:** `docs(resource): rewrite api-reference.md for v2 public API`

---

### Task 7: Rewrite pooling.md for v2

**Files:**
- Rewrite: `crates/resource/docs/pooling.md`

Cover: Pooled trait, PoolConfig fields, PoolStrategy (LIFO/FIFO), WarmupStrategy, idle timeout/max lifetime, fingerprint-based eviction, test_on_checkout, semaphore-based max_size, acquire flow (semaphore → idle check → create → prepare), release flow (via ReleaseQueue: taint? → fingerprint? → lifetime? → recycle), maintenance sweep, cancel-safety guards.

**Commit message:** `docs(resource): rewrite pooling.md for v2 pool topology`

---

### Task 8: Rename and rewrite health-and-quarantine.md → recovery.md

**Files:**
- Delete: `crates/resource/docs/health-and-quarantine.md`
- Create: `crates/resource/docs/recovery.md`

Cover: RecoveryGate (CAS state machine: Idle → InProgress → Failed/PermanentlyFailed), RecoveryTicket (RAII with resolve/fail_transient/fail_permanent), RecoveryWaiter, RecoveryGateConfig, exponential backoff, integration with Manager (check_recovery_gate + trigger_recovery_on_failure), WatchdogHandle/WatchdogConfig, RecoveryGroupRegistry.

Note: v1 HealthChecker, QuarantineManager, HealthPipeline no longer exist. Resource::check() is the only health check mechanism.

**Commit message:** `docs(resource): replace health-and-quarantine.md with recovery.md for v2`

---

### Task 9: Rename and rewrite events-and-hooks.md → events.md

**Files:**
- Delete: `crates/resource/docs/events-and-hooks.md`
- Create: `crates/resource/docs/events.md`

Cover: ResourceEvent enum (7 variants: Registered, Removed, AcquireSuccess, AcquireFailed, Released, HealthChanged, ConfigReloaded), Manager::subscribe_events() returning broadcast::Receiver, usage patterns.

Note: v1 EventBus, HookRegistry, ResourceHook no longer exist.

**Commit message:** `docs(resource): replace events-and-hooks.md with events.md for v2`

---

### Task 10: Rewrite adapters.md for v2

**Files:**
- Rewrite: `crates/resource/docs/adapters.md`

Cover: step-by-step guide to implementing a resource adapter crate for v2:
1. Define config implementing `ResourceConfig` (with `validate()` and `fingerprint()`)
2. Define error type + `ClassifyError` derive (or manual `From<E> for Error`)
3. Implement `Resource` trait (5 associated types + `key()` + `create()`)
4. Pick and implement topology trait (`Pooled`, `Resident`, etc.)
5. Register with Manager using convenience method
6. Integration tests using `BasicCtx` and `AcquireOptions::default()`

Include a complete, compilable example of a Postgres pool adapter.

**Commit message:** `docs(resource): rewrite adapters.md for v2 Resource trait and topologies`

---

### Task 11: Update README crate layout and doc table

**Files:**
- Modify: `crates/resource/docs/README.md` (if not already done in Task 4)

Update the crate layout tree and documentation table to reflect renamed files (recovery.md, events.md instead of old names).

**Commit message:** `docs(resource): update doc table for renamed files`

---

### Final: Full validation

```bash
rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run --workspace && rtk cargo test --doc -p nebula-resource
```

Update `.claude/crates/resource.md` with any new traps or conventions.
