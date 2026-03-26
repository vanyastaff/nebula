# Resource Architect Recommendations Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement 5 features accepted by architect review: remove dead credential param, remove unwired circuit breaker, add topology-erased acquire, add health_check, add missing acquire_default variants.

**Architecture:** Tasks 1-2 are pure deletion (breaking API change within crate). Task 3 adds a new `TopologyAcquire` trait with blanket impls per topology. Tasks 4-5 are additive methods on Manager.

**Tech Stack:** Rust 1.93, RPITIT, tokio

---

### Task 1: Remove dead `_credential` from `register()`

**Files:**
- Modify: `crates/resource/src/manager.rs` — remove param from `register()` and all convenience methods
- Modify: `crates/resource/tests/basic_integration.rs` — remove `()` credential arg from all register calls
- Modify: `crates/resource/tests/dx_audit.rs` — same
- Modify: `crates/resource/tests/dx_evaluation.rs` — same
- Modify: `crates/resource/src/lib.rs` — remove `AcquireCircuitBreakerPreset` from re-exports (prep for Task 2)

**Step 1: Remove `_credential` parameter from `register()`**

In `manager.rs`, change the `register()` signature from:
```rust
pub fn register<R: Resource>(
    &self,
    resource: R,
    config: R::Config,
    _credential: R::Credential,  // DELETE THIS
    scope: ScopeLevel,
    ...
```
to:
```rust
pub fn register<R: Resource>(
    &self,
    resource: R,
    config: R::Config,
    scope: ScopeLevel,
    ...
```

Remove the `#[allow(clippy::too_many_arguments)]` if it drops to 6 params.

**Step 2: Update all convenience methods**

In each `register_*` and `register_*_with` method, remove the `()` credential argument in the `self.register(...)` call. Example for `register_pooled`:
```rust
// Before:
self.register(resource, config, (), ScopeLevel::Global, ...)
// After:
self.register(resource, config, ScopeLevel::Global, ...)
```

Also remove `R: Resource<Credential = ()>` bound — it's no longer needed since we don't pass credential. Wait — actually keep it. The convenience methods are specifically for credential-free resources. The full `register()` still accepts any Resource. The bound `Credential = ()` documents that these shortcuts are for no-auth resources.

Actually — reconsider. Without the credential parameter, the convenience methods don't need to know about credentials at all. But keeping the bound prevents accidental registration of credentialed resources without proper credential handling. **Keep the `Credential = ()` bound on convenience methods** — it's a safety guardrail.

**Step 3: Update all test files**

Search and replace: remove the `()` argument (third positional) from every `.register(` call in tests. The pattern is:
```rust
// Before:
.register(resource, config, (), ScopeLevel::Global, topology, None, None)
// After:
.register(resource, config, ScopeLevel::Global, topology, None, None)
```

**Step 4: Run tests**

```bash
rtk cargo nextest run -p nebula-resource
```

**Step 5: Commit**

```bash
git commit -m "fix(resource): remove dead _credential parameter from register()

The credential parameter was accepted but silently discarded — never
stored in ManagedResource. Credentials are passed at acquire time,
not registration time. Removing prevents silent data loss.

BREAKING: register() now takes 6 params instead of 7.
Convenience methods unchanged (they hardcoded () internally).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Remove unwired `circuit_breaker` from AcquireResilience

**Files:**
- Modify: `crates/resource/src/integration/resilience.rs` — delete enum, remove field
- Modify: `crates/resource/src/integration/mod.rs` — remove re-export
- Modify: `crates/resource/src/lib.rs` — remove `AcquireCircuitBreakerPreset` from re-exports
- Modify: all test files and docs that reference `circuit_breaker`

**Step 1: Delete `AcquireCircuitBreakerPreset` enum and field**

In `resilience.rs`:
- Delete the `AcquireCircuitBreakerPreset` enum (lines 46-57)
- Remove `pub circuit_breaker: Option<AcquireCircuitBreakerPreset>` from `AcquireResilience`
- Update module doc: change "timeout, retry, and circuit-breaker" to "timeout and retry"
- Update struct doc similarly
- Remove `circuit_breaker` from all 4 preset constructors
- Update tests to remove `circuit_breaker` assertions

**Step 2: Update lib.rs re-export**

Change:
```rust
pub use integration::{AcquireCircuitBreakerPreset, AcquireResilience, AcquireRetryConfig};
```
to:
```rust
pub use integration::{AcquireResilience, AcquireRetryConfig};
```

**Step 3: Update integration/mod.rs re-export**

Remove `AcquireCircuitBreakerPreset` from `pub use resilience::...`.

**Step 4: Update all test files and docs that reference circuit_breaker**

Search for `circuit_breaker` in all resource files and remove references.

**Step 5: Run tests + clippy**

```bash
rtk cargo fmt && rtk cargo clippy -p nebula-resource -- -D warnings && rtk cargo nextest run -p nebula-resource
```

**Step 6: Commit**

```bash
git commit -m "fix(resource): remove unwired circuit_breaker from AcquireResilience

AcquireCircuitBreakerPreset was defined and stored but never read by
execute_with_resilience. Users configuring circuit breakers got false
safety. RecoveryGate already provides the circuit-breaker pattern
(fail-fast after N errors, backoff, probe). Removing the dead field
prevents confusion.

BREAKING: AcquireCircuitBreakerPreset removed.
AcquireResilience no longer has circuit_breaker field.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Add `TopologyAcquire` trait + `Manager::acquire()` / `acquire_default()`

**Files:**
- Create: `crates/resource/src/topology_acquire.rs`
- Modify: `crates/resource/src/lib.rs` — add module + re-export
- Modify: `crates/resource/src/manager.rs` — add `acquire()` and `acquire_default()`
- Modify: `crates/resource/src/runtime/mod.rs` — add `TopologyRuntime::tag()` method
- Modify: `crates/resource/tests/basic_integration.rs` — add test

**Step 1: Add `tag()` method to `TopologyRuntime`**

In `crates/resource/src/runtime/mod.rs`, add:
```rust
use crate::topology_tag::TopologyTag;

impl<R: Resource> TopologyRuntime<R> {
    /// Returns the topology tag for this runtime variant.
    pub fn tag(&self) -> TopologyTag {
        match self {
            Self::Pool(_) => TopologyTag::Pool,
            Self::Resident(_) => TopologyTag::Resident,
            Self::Service(_) => TopologyTag::Service,
            Self::Transport(_) => TopologyTag::Transport,
            Self::Exclusive(_) => TopologyTag::Exclusive,
            Self::EventSource(_) => TopologyTag::EventSource,
            Self::Daemon(_) => TopologyTag::Daemon,
        }
    }
}
```

**Step 2: Create `topology_acquire.rs`**

Create `crates/resource/src/topology_acquire.rs` with the trait and blanket impls.

The trait:
```rust
//! Topology-erased acquire dispatch.
//!
//! [`TopologyAcquire`] allows callers to acquire a resource handle without
//! knowing or naming the topology at the call site. The topology was already
//! declared at registration time — callers just say `manager.acquire::<R>()`.

use std::future::Future;
use std::sync::Arc;

use crate::ctx::Ctx;
use crate::error::Error;
use crate::handle::ResourceHandle;
use crate::options::AcquireOptions;
use crate::resource::Resource;
use crate::runtime::TopologyRuntime;
use crate::runtime::managed::ManagedResource;

/// Trait for resources that can be acquired topology-agnostically.
///
/// Implemented automatically for resources that implement a topology trait
/// (`Pooled`, `Resident`, `Service`, `Transport`, `Exclusive`). Callers
/// use [`Manager::acquire`](crate::Manager::acquire) instead of topology-specific methods.
pub trait TopologyAcquire: Resource {
    /// Acquires a handle by dispatching to the registered topology runtime.
    fn topology_acquire(
        managed: &Arc<ManagedResource<Self>>,
        credential: &Self::Credential,
        ctx: &dyn Ctx,
        options: &AcquireOptions,
    ) -> impl Future<Output = Result<ResourceHandle<Self>, Error>> + Send;
}
```

Then blanket impls for each topology. Example for Pooled:
```rust
impl<R> TopologyAcquire for R
where
    R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
    R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
    R::Lease: Into<R::Runtime> + Send + 'static,
{
    fn topology_acquire(
        managed: &Arc<ManagedResource<Self>>,
        credential: &Self::Credential,
        ctx: &dyn Ctx,
        options: &AcquireOptions,
    ) -> impl Future<Output = Result<ResourceHandle<Self>, Error>> + Send {
        async move {
            match &managed.topology {
                TopologyRuntime::Pool(rt) => {
                    let generation = managed.generation();
                    let config = managed.config();
                    rt.acquire(
                        &managed.resource, &config, credential, ctx,
                        &managed.release_queue, generation, options,
                        Arc::clone(&managed.metrics),
                    ).await
                }
                other => Err(Error::permanent(format!(
                    "{}: registered as {}, but acquire expected Pool",
                    Self::key(), other.tag()
                ))),
            }
        }
    }
}
```

**IMPORTANT:** Blanket impls for Pooled, Resident, Service, Transport, Exclusive will OVERLAP if a type implements multiple topology traits (e.g., `PoolTestResource` in tests implements both `Pooled` and `Exclusive`). This causes `E0119: conflicting implementations`.

**Solution:** Don't use blanket impls. Instead, use a single function inside `topology_acquire.rs` that requires the caller to pick a topology via a marker, OR make `TopologyAcquire` require manual impl.

**Better solution:** Instead of a trait, add a single method on `Manager` that does runtime dispatch. This avoids the overlapping impls problem entirely:

```rust
// In manager.rs:
/// Acquires a resource handle with topology-erased dispatch.
///
/// The topology was declared at registration time — callers don't need
/// to know which `acquire_*` method to call. The correct topology is
/// dispatched at runtime based on the stored `TopologyRuntime`.
///
/// # When to use
///
/// Use this when you don't care about the topology or want to decouple
/// call sites from topology choice. Use the topology-specific methods
/// (`acquire_pooled`, `acquire_resident`, etc.) when you need
/// topology-specific trait bounds checked at compile time.
///
/// # Errors
///
/// - [`ErrorKind::NotFound`] if no resource of type `R` is registered.
/// - [`ErrorKind::Cancelled`] if the manager is shutting down.
/// - [`ErrorKind::Permanent`] if the registered topology does not support
///   the required trait bounds (e.g., pool requires `R::Lease: Into<R::Runtime>`).
/// - Propagates topology-specific acquire errors.
pub async fn acquire<R>(
    &self,
    credential: &R::Credential,
    ctx: &dyn Ctx,
    options: &AcquireOptions,
) -> Result<crate::handle::ResourceHandle<R>, Error>
where
    R: Resource + Clone + Send + Sync + 'static,
    R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
    R::Lease: Clone + Into<R::Runtime> + Send + 'static,
{
    let started = Instant::now();
    let managed = self.lookup::<R>(ctx.scope())?;
    check_recovery_gate(&managed.recovery_gate)?;
    let resilience = managed.resilience.clone();

    let result = execute_with_resilience(&resilience, || {
        let generation = managed.generation();
        let config = managed.config();
        let managed = Arc::clone(&managed);
        async move {
            match &managed.topology {
                TopologyRuntime::Pool(rt) => {
                    rt.acquire(
                        &managed.resource, &config, credential, ctx,
                        &managed.release_queue, generation, options,
                        Arc::clone(&managed.metrics),
                    ).await
                }
                TopologyRuntime::Resident(rt) => {
                    rt.acquire(&managed.resource, &config, credential, ctx, options).await
                }
                TopologyRuntime::Service(rt) => {
                    rt.acquire(
                        &managed.resource, ctx, &managed.release_queue,
                        generation, options, Arc::clone(&managed.metrics),
                    ).await
                }
                TopologyRuntime::Transport(rt) => {
                    rt.acquire(
                        &managed.resource, ctx, &managed.release_queue,
                        generation, options, Arc::clone(&managed.metrics),
                    ).await
                }
                TopologyRuntime::Exclusive(rt) => {
                    rt.acquire(
                        &managed.resource, &config, credential, ctx,
                        &managed.release_queue, generation, options,
                        Arc::clone(&managed.metrics),
                    ).await
                }
                _ => Err(Error::permanent(format!(
                    "{}: topology {} does not support acquire",
                    R::key(), managed.topology.tag()
                ))),
            }
        }
    }).await;

    if let Err(e) = &result {
        trigger_recovery_on_failure(&managed.recovery_gate, e);
    }
    self.record_acquire_result(&managed, &result, started);
    result.map(|h| h.with_drain_tracker(self.drain_tracker.clone()))
}

/// Acquires a resource handle without credentials.
///
/// Shorthand for [`acquire`](Self::acquire) with `credential = &()`.
pub async fn acquire_default<R>(
    &self,
    ctx: &dyn Ctx,
    options: &AcquireOptions,
) -> Result<crate::handle::ResourceHandle<R>, Error>
where
    R: Resource<Credential = ()> + Clone + Send + Sync + 'static,
    R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
    R::Lease: Clone + Into<R::Runtime> + Send + 'static,
{
    self.acquire::<R>(&(), ctx, options).await
}
```

Note: The bounds are the UNION of all topology bounds. This means callers need `R::Lease: Clone + Into<R::Runtime>` even for Service/Transport where it's not needed. This is the cost of topology erasure. For cases where this is too restrictive, the topology-specific methods remain available.

**IMPORTANT:** Check that each topology runtime's `acquire` method signature matches what we're passing. Read each runtime's acquire signature to verify.

**Step 3: Add test**

In `basic_integration.rs`, add:
```rust
#[tokio::test]
async fn acquire_default_dispatches_to_correct_topology() {
    let manager = Manager::new();
    let resource = PoolTestResource::new();
    manager.register_pooled(resource, test_config(), PoolConfig::default()).unwrap();

    let ctx = test_ctx();
    let handle = manager.acquire_default::<PoolTestResource>(&ctx, &AcquireOptions::default()).await.unwrap();
    assert_eq!(handle.topology_tag(), nebula_resource::TopologyTag::Pool);
}
```

**Step 4: Run tests**

```bash
rtk cargo nextest run -p nebula-resource
```

**Step 5: Commit**

```bash
git commit -m "feat(resource): add Manager::acquire() / acquire_default() — topology-erased dispatch

Callers no longer need to know which topology a resource uses. The
correct acquire path is dispatched at runtime based on the registered
TopologyRuntime. Also adds TopologyRuntime::tag() for better error
messages ('registered as Pool, expected Resident').

Existing per-topology acquire methods remain for callers who need
topology-specific trait bounds checked at compile time.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Add `Manager::health_check()` with `ResourceHealthSnapshot`

**Files:**
- Modify: `crates/resource/src/manager.rs` — add struct + method
- Modify: `crates/resource/src/lib.rs` — re-export
- Modify: `crates/resource/tests/basic_integration.rs` — add test

**Step 1: Add `ResourceHealthSnapshot` and `health_check()`**

In `manager.rs`, add the struct near `ShutdownConfig`:

```rust
/// Snapshot of a resource's health and operational state.
#[derive(Debug, Clone)]
pub struct ResourceHealthSnapshot {
    /// The resource's unique key.
    pub key: ResourceKey,
    /// Current lifecycle phase.
    pub phase: crate::state::ResourcePhase,
    /// Recovery gate state (if a gate is attached).
    pub gate_state: Option<crate::recovery::gate::GateState>,
    /// Aggregate operation counters.
    pub metrics: crate::metrics::MetricsSnapshot,
    /// Config generation counter.
    pub generation: u64,
}
```

Add the method on `Manager`:
```rust
/// Returns a health snapshot for a registered resource.
///
/// # Errors
///
/// Returns [`ErrorKind::NotFound`] if the resource is not registered.
pub fn health_check<R: Resource>(
    &self,
    scope: &ScopeLevel,
) -> Result<ResourceHealthSnapshot, Error> {
    let managed = self.lookup::<R>(scope)?;
    Ok(ResourceHealthSnapshot {
        key: R::key(),
        phase: managed.status().phase,
        gate_state: managed.recovery_gate.as_ref().map(|g| g.state()),
        metrics: managed.metrics.snapshot(),
        generation: managed.generation(),
    })
}
```

Re-export in `lib.rs`:
```rust
pub use manager::{Manager, ManagerConfig, RegisterOptions, ResourceHealthSnapshot, ShutdownConfig};
```

**Step 2: Add test**

```rust
#[tokio::test]
async fn health_check_returns_snapshot() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    manager.register_resident(resource, test_config(), ResidentConfig::default()).unwrap();

    let snapshot = manager.health_check::<ResidentTestResource>(&ScopeLevel::Global).unwrap();
    assert_eq!(snapshot.key, resource_key!("test-resident"));
    assert_eq!(snapshot.generation, 0);
    assert!(snapshot.gate_state.is_none());
}
```

**Step 3: Commit**

```bash
git commit -m "feat(resource): add Manager::health_check() with ResourceHealthSnapshot

Returns phase, gate state, metrics snapshot, and generation for a
registered resource. Enables health dashboards and diagnostics
without assembling data from multiple sources.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Add missing `_default` acquire variants

**Files:**
- Modify: `crates/resource/src/manager.rs`

**Step 1: Add the three missing methods**

After the existing `acquire_service` method:
```rust
/// Acquires a service resource handle without credentials.
pub async fn acquire_service_default<R>(
    &self,
    ctx: &dyn Ctx,
    options: &AcquireOptions,
) -> Result<crate::handle::ResourceHandle<R>, Error>
where
    R: crate::topology::service::Service<Credential = ()> + Clone + Send + Sync + 'static,
    R::Runtime: Send + Sync + 'static,
    R::Lease: Send + 'static,
{
    self.acquire_service::<R>(&(), ctx, options).await
}
```

After `acquire_transport`:
```rust
/// Acquires a transport resource handle without credentials.
pub async fn acquire_transport_default<R>(
    &self,
    ctx: &dyn Ctx,
    options: &AcquireOptions,
) -> Result<crate::handle::ResourceHandle<R>, Error>
where
    R: crate::topology::transport::Transport<Credential = ()> + Clone + Send + Sync + 'static,
    R::Runtime: Send + Sync + 'static,
    R::Lease: Send + 'static,
{
    self.acquire_transport::<R>(&(), ctx, options).await
}
```

After `acquire_exclusive`:
```rust
/// Acquires an exclusive resource handle without credentials.
pub async fn acquire_exclusive_default<R>(
    &self,
    ctx: &dyn Ctx,
    options: &AcquireOptions,
) -> Result<crate::handle::ResourceHandle<R>, Error>
where
    R: crate::topology::exclusive::Exclusive<Credential = ()> + Clone + Send + Sync + 'static,
    R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
    R::Lease: Send + 'static,
{
    self.acquire_exclusive::<R>(&(), ctx, options).await
}
```

**Step 2: Run tests + clippy**

```bash
rtk cargo fmt && rtk cargo clippy -p nebula-resource -- -D warnings && rtk cargo nextest run -p nebula-resource
```

**Step 3: Commit**

```bash
git commit -m "feat(resource): add acquire_service/transport/exclusive_default helpers

Complete the _default variant coverage for all topology acquire
methods. Each passes &() as credential for Credential = () resources.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Final: Full workspace validation + context update

```bash
rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run --workspace
```

Update `.claude/crates/resource.md` and docs as needed.
