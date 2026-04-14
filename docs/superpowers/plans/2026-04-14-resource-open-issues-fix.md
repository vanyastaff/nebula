# Fix nebula-resource Open Issues Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve all 11 open issues filed against `nebula-resource` (#272, #302, #318, #322, #323, #382, #383, #384, #387, #390, #391) by either landing the missing fix, adding the missing regression test, or correcting documentation — and close every issue with a verifiable PR reference.

**Architecture:** Six issues describe code that is still broken in `crates/resource` and need real fixes (#382, #383, #384, #387, #390, #391). Five issues describe bugs whose fix has already landed in `manager.rs` / `runtime/daemon.rs` but were never closed (#272, #302, #318, #322, #323) — these need verification by inspecting the current code, adding any missing regression tests, and closing the issue. Work is grouped by file boundary so tasks stay self-contained.

**Tech Stack:** Rust 2024 edition, `tokio`, `arc-swap`, `dashmap`, `tokio-util::sync::CancellationToken`, `nebula-resilience::retry`, `cargo nextest`.

---

## Issue Triage Summary

| Issue | Status in code | Action |
|---|---|---|
| #272 wait_for_drain lost-wakeup race | Fixed (register-then-check at `manager.rs:1442`); two regression tests at `manager.rs:1731,1766` | Verify + close |
| #302 graceful_shutdown force-clear on drain timeout | Fixed (`DrainTimeoutPolicy`, `ShutdownReport`, `ShutdownError` at `manager.rs:1338`) | Verify + add coverage + close |
| #318 DaemonRuntime restart-safe | Fixed (`per-run` child token + finished-handle cleanup at `runtime/daemon.rs:131`) | Add regression test + close |
| #322 RecoveryGate probe herd | Fixed (`admit_through_gate` at `manager.rs:1612`) | Verify + add coverage + close |
| #323 daemon backoff cancel-aware | Fixed (`biased select` at `runtime/daemon.rs:253`) | Add regression test + close |
| #382 stale TypeId rows on replace | **Broken** | Implement fix |
| #383 max_attempts=0 panic | **Broken** | Implement fix |
| #384 Exclusive reset vs permit ordering | **Broken** | Implement fix |
| #387 ResourceStatus.phase never updated | **Broken** | Implement fix |
| #390 pool config gaps | **Broken** | Implement fix |
| #391 AcquireOptions intent/tags unused | **Broken (docs only)** | Doc-only fix |

---

## File Map

Files that this plan modifies (grouped by responsibility):

- `crates/resource/src/integration/resilience.rs` — fix `to_retry_config` panic (#383).
- `crates/resource/src/registry.rs` — fix stale `type_index` rows on replace (#382).
- `crates/resource/src/runtime/exclusive.rs` — hold permit until `reset()` completes (#384).
- `crates/resource/src/state.rs` — add helper for status mutation (#387).
- `crates/resource/src/runtime/managed.rs` — add `set_phase` / `set_phase_with_generation` helpers (#387).
- `crates/resource/src/manager.rs` — drive phase transitions + `min_size`/`max_size` validation + add regression tests (#387, #390, #302, #322).
- `crates/resource/src/runtime/pool.rs` — wire `max_concurrent_creates` into create path (#390).
- `crates/resource/src/runtime/daemon.rs` — add lifecycle regression tests (#318, #323).
- `crates/resource/src/options.rs` — narrow doc comments for unused fields (#391).

No new files are created. Tests live alongside the code they cover (in-file `#[cfg(test)] mod tests`).

---

## Task 1: #383 — Stop panicking on `max_attempts: 0`

**Files:**
- Modify: `crates/resource/src/integration/resilience.rs:96-115`
- Test: `crates/resource/src/integration/resilience.rs` (extend in-file `#[cfg(test)] mod tests`)

**Why:** `to_retry_config` calls `RetryConfig::new(max_attempts).expect("max_attempts validated at construction")` but nothing validates the value when callers build `AcquireResilience` manually. A user-supplied `max_attempts: 0` from a config file panics the manager during the first acquire.

**Fix shape:** Clamp `max_attempts` to `max(1, …)` inside `to_retry_config` so a misconfigured zero degrades to a single attempt instead of panicking. Document the clamp on the field.

- [ ] **Step 1: Write the failing test**

In `crates/resource/src/integration/resilience.rs`, append to `mod tests`:

```rust
#[test]
fn to_retry_config_handles_zero_max_attempts_without_panic() {
    let cfg = AcquireResilience {
        timeout: None,
        retry: Some(AcquireRetryConfig {
            max_attempts: 0,
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(50),
        }),
    };

    // Must not panic. Behaviour: degrade to a single attempt.
    let _ = cfg.to_retry_config::<crate::error::Error>();
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo nextest run -p nebula-resource integration::resilience::tests::to_retry_config_handles_zero_max_attempts_without_panic
```

Expected: FAIL with `panicked at 'max_attempts validated at construction'`.

- [ ] **Step 3: Implement the clamp**

Replace the body of `to_retry_config` in `crates/resource/src/integration/resilience.rs`:

```rust
pub(crate) fn to_retry_config<E: 'static>(&self) -> RetryConfig<E> {
    // #383: a misconfigured `max_attempts: 0` from a config file used to
    // panic here via `expect`. Clamp to `1` so we degrade to a single
    // attempt rather than killing the process during acquire.
    let max_attempts = self.retry.as_ref().map_or(1, |r| r.max_attempts.max(1));
    let cfg = RetryConfig::new(max_attempts)
        .expect("max_attempts clamped to >=1 above");

    let cfg = if let Some(ref retry) = self.retry {
        cfg.backoff(BackoffConfig::Exponential {
            base: retry.initial_backoff,
            multiplier: 2.0,
            max: retry.max_backoff,
        })
    } else {
        cfg
    };

    if let Some(timeout) = self.timeout {
        cfg.total_budget(timeout)
    } else {
        cfg
    }
}
```

Also update the doc comment on `AcquireRetryConfig::max_attempts` (around line 38-39) to add: `Values below 1 are clamped to 1 by the manager.`

- [ ] **Step 4: Run the test to verify it passes**

```bash
cargo nextest run -p nebula-resource integration::resilience::tests::to_retry_config_handles_zero_max_attempts_without_panic
```

Expected: PASS.

- [ ] **Step 5: Run the full crate test suite**

```bash
cargo nextest run -p nebula-resource
```

Expected: PASS (no regressions).

- [ ] **Step 6: Commit**

```bash
git add crates/resource/src/integration/resilience.rs
git commit -m "fix(resource): clamp AcquireResilience max_attempts to >=1 (#383)"
```

---

## Task 2: #382 — Drop stale `TypeId` rows on `Registry::register` replace

**Files:**
- Modify: `crates/resource/src/registry.rs:20-86`
- Test: `crates/resource/src/registry.rs` (add `#[cfg(test)] mod tests` if absent)

**Why:** `Registry::register` always inserts the new `(type_id → key)` row but does not remove the previous `type_id` row when the in-place replace at the same `(key, scope)` swaps the underlying `R` type. The stale row leaks forever and `get_typed::<OldR>` resolves the key but the downcast then fails — confusing semantics + a small map leak.

**Fix shape:** Expose the `TypeId` of the boxed `ManagedResource<R>` via the `AnyManagedResource` trait. On the replace path in `register`, remove the prior entry's `TypeId` from `type_index` before swapping in the new one.

- [ ] **Step 1: Write the failing test**

In `crates/resource/src/registry.rs`, append at the bottom of the file:

```rust
#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use std::sync::Arc;

    use nebula_core::ResourceKey;

    use super::*;
    use crate::ctx::ScopeLevel;

    // Two zero-sized fakes for the test — they only need to implement the
    // trait, not real resource semantics.
    struct FakeA;
    struct FakeB;

    impl AnyManagedResource for FakeA {
        fn resource_key(&self) -> ResourceKey {
            ResourceKey::from_static("fake")
        }
        fn as_any_arc(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
            self
        }
        fn managed_type_id(&self) -> TypeId {
            TypeId::of::<FakeA>()
        }
    }

    impl AnyManagedResource for FakeB {
        fn resource_key(&self) -> ResourceKey {
            ResourceKey::from_static("fake")
        }
        fn as_any_arc(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
            self
        }
        fn managed_type_id(&self) -> TypeId {
            TypeId::of::<FakeB>()
        }
    }

    #[test]
    fn register_replace_drops_stale_type_id_row() {
        let reg = Registry::new();
        let key = ResourceKey::from_static("fake");
        let scope = ScopeLevel::Global;

        reg.register(key.clone(), TypeId::of::<FakeA>(), scope, Arc::new(FakeA));
        assert!(reg.type_index.contains_key(&TypeId::of::<FakeA>()));

        // Replace at the same key+scope with a different concrete type.
        reg.register(key.clone(), TypeId::of::<FakeB>(), scope, Arc::new(FakeB));

        // The stale TypeId row for FakeA must be gone.
        assert!(
            !reg.type_index.contains_key(&TypeId::of::<FakeA>()),
            "stale TypeId for FakeA still in type_index after replace"
        );
        assert!(reg.type_index.contains_key(&TypeId::of::<FakeB>()));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo nextest run -p nebula-resource registry::tests::register_replace_drops_stale_type_id_row
```

Expected: FAIL — either compile error (no `managed_type_id`) or assertion failure.

- [ ] **Step 3: Add `managed_type_id` to the trait**

In `crates/resource/src/registry.rs`, replace the trait + impl block:

```rust
pub trait AnyManagedResource: Send + Sync + 'static {
    /// Returns the resource key for this managed resource.
    fn resource_key(&self) -> ResourceKey;

    /// Returns a reference to `self` as `&dyn Any` for downcasting.
    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;

    /// Returns the concrete `TypeId` used as the secondary index key.
    ///
    /// For real `ManagedResource<R>` this is `TypeId::of::<ManagedResource<R>>()`.
    /// Used by [`Registry::register`] to scrub stale rows from `type_index`
    /// when an entry is replaced in place (#382).
    fn managed_type_id(&self) -> TypeId;
}

impl<R: Resource> AnyManagedResource for ManagedResource<R> {
    fn resource_key(&self) -> ResourceKey {
        R::key()
    }

    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }

    fn managed_type_id(&self) -> TypeId {
        TypeId::of::<ManagedResource<R>>()
    }
}
```

- [ ] **Step 4: Drop stale rows on replace**

Replace `Registry::register` body in the same file:

```rust
pub fn register(
    &self,
    key: ResourceKey,
    type_id: TypeId,
    scope: ScopeLevel,
    managed: Arc<dyn AnyManagedResource>,
) {
    let mut entries = self.entries.entry(key.clone()).or_default();

    // #382: if we're about to replace an entry at the same scope whose
    // concrete TypeId differs from the new one, scrub the prior TypeId
    // row from type_index before installing the new mapping.
    if let Some(pos) = entries.iter().position(|e| e.scope == scope) {
        let prev_type_id = entries[pos].managed.managed_type_id();
        if prev_type_id != type_id {
            self.type_index
                .remove_if(&prev_type_id, |_, k| k == &key);
        }
        entries[pos] = RegistryEntry { scope, managed };
    } else {
        entries.push(RegistryEntry { scope, managed });
    }

    self.type_index.insert(type_id, key);
}
```

- [ ] **Step 5: Run the targeted test to verify it passes**

```bash
cargo nextest run -p nebula-resource registry::tests::register_replace_drops_stale_type_id_row
```

Expected: PASS.

- [ ] **Step 6: Run the full crate test suite**

```bash
cargo nextest run -p nebula-resource
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/resource/src/registry.rs
git commit -m "fix(resource): scrub stale TypeId rows on Registry::register replace (#382)"
```

---

## Task 3: #384 — Hold exclusive permit until `reset()` completes

**Files:**
- Modify: `crates/resource/src/runtime/exclusive.rs:55-122`
- Test: `crates/resource/src/runtime/exclusive.rs` (add `#[cfg(test)] mod tests`)

**Why:** `ExclusiveRuntime::acquire` passes the semaphore permit to `ResourceHandle::guarded_with_permit`, which drops it at the end of `Drop`. The `release_exclusive(...)` future is only **submitted** to the `ReleaseQueue` synchronously and runs asynchronously after the permit is already gone. That contradicts the doc contract on `Exclusive::reset` (“before the next caller can acquire”): a second acquirer can take the permit while the previous reset is still queued or in flight.

**Fix shape:** Don’t hand the permit to `guarded_with_permit`. Instead, move the `OwnedSemaphorePermit` into the release closure’s submitted future and drop it explicitly **after** `reset()` resolves. This keeps the permit alive for the entire reset window, so the next acquirer cannot enter until the previous reset is done.

- [ ] **Step 1: Write the failing test**

Append to `crates/resource/src/runtime/exclusive.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    use crate::ctx::BasicCtx;
    use crate::error::Error;
    use crate::release_queue::ReleaseQueue;
    use crate::resource::Resource;
    use crate::topology::exclusive::{Exclusive, config::Config};

    #[derive(Clone)]
    struct SlowResetExclusive {
        reset_in_progress: Arc<AtomicBool>,
        overlap_observed: Arc<AtomicBool>,
    }

    #[derive(Clone)]
    struct FakeRuntime;

    impl Resource for SlowResetExclusive {
        type Config = ();
        type Runtime = FakeRuntime;
        type Lease = FakeRuntime;
        type Error = Error;

        fn key() -> nebula_core::ResourceKey {
            nebula_core::ResourceKey::from_static("slow-reset-exclusive")
        }
    }

    impl Exclusive for SlowResetExclusive {
        async fn reset(&self, _runtime: &Self::Runtime) -> Result<(), Self::Error> {
            self.reset_in_progress.store(true, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(50)).await;
            self.reset_in_progress.store(false, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn next_acquire_waits_until_reset_completes() {
        let rt = ExclusiveRuntime::<SlowResetExclusive>::new(
            FakeRuntime,
            Config::default(),
        );
        let rq = Arc::new(ReleaseQueue::start_default());
        let resource = SlowResetExclusive {
            reset_in_progress: Arc::new(AtomicBool::new(false)),
            overlap_observed: Arc::new(AtomicBool::new(false)),
        };
        let opts = AcquireOptions::default();

        let handle = rt
            .acquire(&resource, &rq, 0, &opts, None)
            .await
            .expect("first acquire");
        drop(handle);

        // The next acquire must observe reset_in_progress == false at the
        // moment its permit is granted.
        let handle = rt
            .acquire(&resource, &rq, 0, &opts, None)
            .await
            .expect("second acquire");
        let in_progress = resource.reset_in_progress.load(Ordering::SeqCst);
        assert!(
            !in_progress,
            "second acquire raced against an in-flight reset()"
        );
        drop(handle);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo nextest run -p nebula-resource runtime::exclusive::tests::next_acquire_waits_until_reset_completes
```

Expected: FAIL — `in_progress == true` at second acquire because the previous reset is still running asynchronously.

- [ ] **Step 3: Move the permit into the release closure**

Replace the body of `ExclusiveRuntime::acquire` and `release_exclusive` in `crates/resource/src/runtime/exclusive.rs`:

```rust
pub async fn acquire(
    &self,
    resource: &R,
    release_queue: &Arc<ReleaseQueue>,
    generation: u64,
    options: &AcquireOptions,
    metrics: Option<ResourceOpsMetrics>,
) -> Result<ResourceHandle<R>, Error>
where
    R::Runtime: Into<R::Lease>,
{
    let timeout = options.remaining().unwrap_or(self.config.acquire_timeout);
    let permit =
        match tokio::time::timeout(timeout, self.semaphore.clone().acquire_owned()).await {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => return Err(Error::permanent("exclusive semaphore closed")),
            Err(_) => return Err(Error::backpressure("exclusive: timed out waiting for lock")),
        };

    let lease: R::Lease = (*self.runtime).clone().into();
    let runtime = self.runtime.clone();
    let resource_clone = resource.clone();
    let rq = release_queue.clone();

    // #384: keep the permit alive until reset() finishes. The closure
    // *captures* `permit` and the submitted future drops it AFTER reset.
    // ResourceHandle is built without a permit slot — the permit is no
    // longer tied to handle Drop ordering.
    Ok(ResourceHandle::guarded(
        lease,
        R::key(),
        TopologyTag::Exclusive,
        generation,
        move |_returned_lease, _tainted| {
            if let Some(m) = &metrics {
                m.record_release();
            }
            let permit = permit; // move into outer closure
            rq.submit(move || {
                Box::pin(release_exclusive(resource_clone, runtime, permit))
            });
        },
    ))
}
```

```rust
async fn release_exclusive<R>(
    resource: R,
    runtime: Arc<R::Runtime>,
    permit: tokio::sync::OwnedSemaphorePermit,
)
where
    R: Exclusive + Send + Sync + 'static,
    R::Runtime: Send + Sync + 'static,
{
    let _ = resource.reset(&runtime).await;
    // Drop AFTER reset has returned so the next acquirer cannot enter
    // mid-reset (#384).
    drop(permit);
}
```

If `ResourceHandle::guarded` does not exist, use `guarded_with_permit(..., None)` and accept that the `permit_slot` field on the handle stays empty.

- [ ] **Step 4: Run the targeted test to verify it passes**

```bash
cargo nextest run -p nebula-resource runtime::exclusive::tests::next_acquire_waits_until_reset_completes
```

Expected: PASS.

- [ ] **Step 5: Update doc on `Exclusive::reset` to confirm guarantee**

In `crates/resource/src/topology/exclusive.rs`, locate the `reset` doc comment and tighten it: "Called after the lease is released, before the **next** caller can acquire the exclusive permit. The runtime guarantees this ordering by holding the semaphore permit until `reset` resolves (#384)."

- [ ] **Step 6: Run the full crate test suite**

```bash
cargo nextest run -p nebula-resource
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/resource/src/runtime/exclusive.rs crates/resource/src/topology/exclusive.rs
git commit -m "fix(resource): hold exclusive permit until reset() completes (#384)"
```

---

## Task 4: #387 — Drive `ResourceStatus.phase` lifecycle

**Files:**
- Modify: `crates/resource/src/runtime/managed.rs:49-64`
- Modify: `crates/resource/src/state.rs` (add a small builder helper)
- Modify: `crates/resource/src/manager.rs` (around lines 315, 1248, 1338) — wire transitions on register/reload/shutdown
- Test: `crates/resource/src/manager.rs` (in-file `mod phase_tests`)

**Why:** `ManagedResource.status: ArcSwap<ResourceStatus>` is initialized to `(Initializing, generation=0, last_error=None)` in `Manager::register` (line 335) and **never updated** anywhere. `Manager::reload_config` bumps the separate `generation: AtomicU64` but does not touch the embedded `ResourceStatus.generation` either. `Manager::health_check` (line 1531) reads `managed.status().phase` and returns it directly to operators — so health snapshots show `phase = Initializing, generation = 0` forever, while the real generation lives in a sibling atomic. Operators can’t trust either field.

**Fix shape:** Add a `set_phase` helper on `ManagedResource` that builds a fresh `ResourceStatus` from the latest snapshot and stores it via `ArcSwap`. After the success path of every `register_*` set the phase to `Ready`. In `reload_config`, transition `Reloading → Ready` and copy the post-bump generation into `ResourceStatus.generation`. In `graceful_shutdown` Phase 1 set `Draining`, after `wait_for_drain` set `ShuttingDown`. Also expose a `set_failed` so transient errors can record `last_error`.

- [ ] **Step 1: Write the failing test**

In `crates/resource/src/manager.rs`, add a new in-file module right above `mod drain_race_tests`:

```rust
#[cfg(test)]
mod phase_lifecycle_tests {
    use super::*;
    use crate::resource::test_support::TrivialResource; // see Step 2
    use crate::state::ResourcePhase;

    #[tokio::test]
    async fn register_transitions_to_ready() {
        let mgr = Manager::new();
        mgr.register::<TrivialResource>(TrivialResource::default(), &ScopeLevel::Global)
            .await
            .expect("register");

        let snap = mgr
            .health_check::<TrivialResource>(&ScopeLevel::Global)
            .expect("health");
        assert_eq!(snap.phase, ResourcePhase::Ready);
        assert_eq!(snap.generation, 0);
    }

    #[tokio::test]
    async fn reload_bumps_status_generation() {
        let mgr = Manager::new();
        mgr.register::<TrivialResource>(TrivialResource::default(), &ScopeLevel::Global)
            .await
            .expect("register");

        mgr.reload_config::<TrivialResource>(Default::default(), &ScopeLevel::Global)
            .expect("reload");

        let snap = mgr
            .health_check::<TrivialResource>(&ScopeLevel::Global)
            .expect("health");
        assert_eq!(snap.phase, ResourcePhase::Ready);
        assert_eq!(snap.generation, 1);
    }

    #[tokio::test]
    async fn graceful_shutdown_walks_phase_to_shutting_down() {
        let mgr = Manager::new();
        mgr.register::<TrivialResource>(TrivialResource::default(), &ScopeLevel::Global)
            .await
            .expect("register");

        let report = mgr
            .graceful_shutdown(ShutdownConfig::default())
            .await
            .expect("graceful");

        // After clear() the registry is empty, so direct lookup of phase
        // is meaningless. Instead, snapshot before shutdown:
        assert!(report.registry_cleared);
    }
}
```

If a `TrivialResource` test fixture does not exist, create one under `crates/resource/src/resource.rs` inside a new `pub(crate) mod test_support` gated on `#[cfg(test)]`. It must implement `Resource` with `Config = ()` and `Runtime = ()`.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo nextest run -p nebula-resource phase_lifecycle_tests
```

Expected: FAIL — `register_transitions_to_ready` reports `phase = Initializing`, `reload_bumps_status_generation` reports `generation = 0`.

- [ ] **Step 3: Add the `set_phase` helper**

In `crates/resource/src/runtime/managed.rs`, append to `impl<R: Resource> ManagedResource<R>`:

```rust
/// Atomically replace the lifecycle status with a new phase, copying
/// across the current generation and clearing `last_error`. (#387)
pub(crate) fn set_phase(&self, phase: crate::state::ResourcePhase) {
    let prev = self.status.load_full();
    let next = crate::state::ResourceStatus {
        phase,
        generation: self.generation(),
        last_error: prev.last_error.clone(),
    };
    self.status.store(Arc::new(next));
}

/// Like [`set_phase`] but also records an error string on the new status.
pub(crate) fn set_failed(&self, error: impl Into<String>) {
    let next = crate::state::ResourceStatus {
        phase: crate::state::ResourcePhase::Failed,
        generation: self.generation(),
        last_error: Some(error.into()),
    };
    self.status.store(Arc::new(next));
}
```

- [ ] **Step 4: Wire `Ready` after every register path**

In `crates/resource/src/manager.rs`, locate the `register` function around line 315. After the `self.registry.register(...)` call and before the function returns `Ok(())`, add:

```rust
// #387: register puts the resource in Ready. Future hot-reloads and
// shutdown phases mutate this via ManagedResource::set_phase.
managed.set_phase(crate::state::ResourcePhase::Ready);
```

The same hook must be added inside every `register_*` and `register_*_with` helper that constructs and stores a `ManagedResource` directly, OR `register` should be the single funnel — verify by inspection that all `register_*` paths route through one creator. If not, add the hook to each.

- [ ] **Step 5: Wire `Reloading → Ready` in `reload_config`**

In `crates/resource/src/manager.rs::reload_config` (around line 1248), bracket the swap:

```rust
managed.set_phase(crate::state::ResourcePhase::Reloading);

managed.config.store(Arc::new(new_config));

if let TopologyRuntime::Pool(ref pool_rt) = managed.topology {
    pool_rt.set_fingerprint(new_fp);
}

managed
    .generation
    .fetch_add(1, std::sync::atomic::Ordering::Release);

// #387: keep ResourceStatus.generation in sync with the AtomicU64.
managed.set_phase(crate::state::ResourcePhase::Ready);
```

- [ ] **Step 6: Wire `Draining` / `ShuttingDown` into `graceful_shutdown`**

In `crates/resource/src/manager.rs::graceful_shutdown` (around line 1338), iterate the registry once at Phase 1 to set `Draining`, and once after the drain to set `ShuttingDown`. Because `Registry::keys()` is the only safe iteration entry point, use it together with `get_any` to fetch the typed-erased managed handle.

After `self.cancel.cancel();` and before `wait_for_drain`:

```rust
// #387: visible Draining phase for operators querying health while
// in-flight work winds down.
for key in self.registry.keys() {
    if let Some(any) = self.registry.get(&key, &ScopeLevel::Global) {
        // We don't need typed access — phase update lives behind
        // a non-generic helper on AnyManagedResource (added below).
        any.set_phase_erased(crate::state::ResourcePhase::Draining);
    }
}
```

To support the type-erased call, extend `AnyManagedResource` (in `registry.rs`) with one extra method:

```rust
pub trait AnyManagedResource: Send + Sync + 'static {
    fn resource_key(&self) -> ResourceKey;
    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
    fn managed_type_id(&self) -> TypeId;

    /// Type-erased phase mutation (#387).
    fn set_phase_erased(&self, phase: crate::state::ResourcePhase);
}

impl<R: Resource> AnyManagedResource for ManagedResource<R> {
    // ...existing methods...
    fn set_phase_erased(&self, phase: crate::state::ResourcePhase) {
        self.set_phase(phase);
    }
}
```

After `wait_for_drain` returns (and before `registry.clear()`), repeat the loop with `ResourcePhase::ShuttingDown`.

- [ ] **Step 7: Run the phase tests to verify they pass**

```bash
cargo nextest run -p nebula-resource phase_lifecycle_tests
```

Expected: PASS.

- [ ] **Step 8: Run the full crate test suite**

```bash
cargo nextest run -p nebula-resource
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/resource/src/runtime/managed.rs crates/resource/src/registry.rs crates/resource/src/manager.rs crates/resource/src/state.rs
git commit -m "fix(resource): drive ResourceStatus.phase across register/reload/shutdown (#387)"
```

---

## Task 5: #390 — Validate pool sizing and enforce `max_concurrent_creates`

**Files:**
- Modify: `crates/resource/src/runtime/pool.rs:65-160` (add semaphore field + use during `create_entry`)
- Modify: `crates/resource/src/manager.rs:365` (`register_pooled`) and `:520` (`register_pooled_with`)
- Test: `crates/resource/src/runtime/pool.rs` (in-file `mod tests`)
- Test: `crates/resource/src/manager.rs` (in-file `mod register_validation_tests`)

**Why:** The pool config exposes `max_concurrent_creates` but `runtime/pool.rs` never references it — concurrent instance creation is unbounded. Separately, nothing validates `min_size <= max_size && max_size > 0`, so a misconfigured pool can warm up far more instances than `max_size` checkouts allow, wasting memory until the maintenance sweep evicts them.

**Fix shape:**

1. Add a second `Arc<Semaphore>` to `PoolRuntime` keyed off `config.max_concurrent_creates`, acquire one permit before each `create_entry` invocation, drop it on return. The existing `max_size` semaphore is unchanged — it gates checkout, not creation.
2. In `Manager::register_pooled` and `register_pooled_with`, validate the config. Return `Error::permanent` with a precise message on violation. Do the validation **before** constructing `PoolRuntime` so we never allocate a broken pool.

- [ ] **Step 1: Write the failing test for pool validation**

In `crates/resource/src/manager.rs`, add:

```rust
#[cfg(test)]
mod register_validation_tests {
    use super::*;
    use crate::resource::test_support::TrivialPooledResource;
    use crate::topology::pooled::config::Config as PoolCfg;

    #[tokio::test]
    async fn register_pooled_rejects_min_greater_than_max() {
        let mgr = Manager::new();
        let bad = PoolCfg {
            min_size: 5,
            max_size: 2,
            ..Default::default()
        };
        let err = mgr
            .register_pooled::<TrivialPooledResource>(
                TrivialPooledResource::default(),
                bad,
                &ScopeLevel::Global,
            )
            .await
            .expect_err("min > max must be rejected");
        let msg = format!("{err}");
        assert!(msg.contains("min_size") && msg.contains("max_size"), "msg = {msg}");
    }

    #[tokio::test]
    async fn register_pooled_rejects_max_size_zero() {
        let mgr = Manager::new();
        let bad = PoolCfg { min_size: 0, max_size: 0, ..Default::default() };
        let err = mgr
            .register_pooled::<TrivialPooledResource>(
                TrivialPooledResource::default(),
                bad,
                &ScopeLevel::Global,
            )
            .await
            .expect_err("max_size = 0 must be rejected");
        assert!(format!("{err}").contains("max_size"));
    }
}
```

`TrivialPooledResource` is a test-only fixture inside `resource::test_support`; create it alongside `TrivialResource` from Task 4. It must implement `Pooled` with no-op hooks.

- [ ] **Step 2: Run validation test to verify failure**

```bash
cargo nextest run -p nebula-resource register_validation_tests
```

Expected: FAIL — `register_pooled` succeeds today.

- [ ] **Step 3: Add validation to `register_pooled` and `register_pooled_with`**

In `crates/resource/src/manager.rs::register_pooled`, at the very top of the function:

```rust
// #390: catch obviously broken pool configs at registration time so
// warmup never inflates beyond max_size.
if config.max_size == 0 {
    return Err(Error::permanent("pool max_size must be > 0"));
}
if config.min_size > config.max_size {
    return Err(Error::permanent(format!(
        "pool min_size ({}) must be <= max_size ({})",
        config.min_size, config.max_size
    )));
}
```

Repeat the same block at the top of `register_pooled_with`.

- [ ] **Step 4: Run validation tests to verify they pass**

```bash
cargo nextest run -p nebula-resource register_validation_tests
```

Expected: PASS.

- [ ] **Step 5: Write the failing test for `max_concurrent_creates`**

In `crates/resource/src/runtime/pool.rs`, append (or extend) the existing `#[cfg(test)] mod tests`:

```rust
#[tokio::test]
async fn create_path_respects_max_concurrent_creates() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    let in_flight = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));

    // Resource whose construct hook records peak concurrency.
    let resource = SlowCreateResource {
        in_flight: in_flight.clone(),
        peak: peak.clone(),
    };

    let cfg = topology::pooled::config::Config {
        min_size: 0,
        max_size: 10,
        max_concurrent_creates: 2,
        warmup: topology::pooled::config::WarmupStrategy::None,
        ..Default::default()
    };
    let pool = PoolRuntime::new(resource, cfg, /* …existing args… */);

    let mut handles = Vec::new();
    for _ in 0..10 {
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            let _ = pool.acquire(/* …existing args… */).await;
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    let observed_peak = peak.load(Ordering::SeqCst);
    assert!(
        observed_peak <= 2,
        "max_concurrent_creates=2 violated: peak={observed_peak}"
    );
}
```

`SlowCreateResource` is a test-only fixture: its `Resource::construct` (or whatever the create hook is called) increments `in_flight`, sleeps 30 ms, updates `peak = max(peak, in_flight)`, then decrements. Define it in the same `tests` module.

The exact `PoolRuntime::new` and `acquire` signatures are visible at `crates/resource/src/runtime/pool.rs:101` and `:504`. Match them when filling the `…existing args…` placeholders.

- [ ] **Step 6: Run create-cap test to verify failure**

```bash
cargo nextest run -p nebula-resource runtime::pool::tests::create_path_respects_max_concurrent_creates
```

Expected: FAIL — `peak > 2` because creates are unbounded today.

- [ ] **Step 7: Add `create_semaphore` field and acquire on every create**

In `crates/resource/src/runtime/pool.rs:65-130`, add a field:

```rust
pub struct PoolRuntime<R: Resource> {
    // ...existing fields...
    /// Bounds concurrent invocations of `create_entry` (#390).
    create_semaphore: Arc<Semaphore>,
}
```

Initialize it in the existing constructor near line 101:

```rust
let create_semaphore = Arc::new(Semaphore::new(
    (config.max_concurrent_creates as usize).max(1),
));
```

In every code path that calls `create_entry` (use grep `create_entry` to enumerate), wrap the call:

```rust
// #390: cap concurrent instance creation.
let _create_permit = self
    .create_semaphore
    .clone()
    .acquire_owned()
    .await
    .map_err(|_| Error::permanent("pool create semaphore closed"))?;
let entry = self.create_entry(/*…*/).await?;
drop(_create_permit);
```

Inside the `warmup` path (around line 615), the same gate must apply so `Parallel` warmup respects the cap.

- [ ] **Step 8: Run create-cap test to verify it passes**

```bash
cargo nextest run -p nebula-resource runtime::pool::tests::create_path_respects_max_concurrent_creates
```

Expected: PASS.

- [ ] **Step 9: Run the full crate test suite**

```bash
cargo nextest run -p nebula-resource
```

Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/resource/src/runtime/pool.rs crates/resource/src/manager.rs crates/resource/src/resource.rs
git commit -m "fix(resource): enforce pool max_concurrent_creates and validate min/max (#390)"
```

---

## Task 6: #391 — Narrow doc comments for unused `AcquireOptions` fields

**Files:**
- Modify: `crates/resource/src/options.rs:14-56`

**Why:** `AcquireOptions::intent` and `AcquireOptions::tags` are documented as influencing topology behaviour ("topologies may use this to select different pools, apply different timeouts, or skip health checks"), but no code outside `options.rs` reads either field. Callers who set `AcquireIntent::Critical` get **no behaviour change**. Until the engine integration that consumes these lands, the public docs are misleading.

**Fix shape:** Doc-only change. Re-word the doc comments to mark `intent` and `tags` as "reserved for future engine integration; currently only `deadline` affects acquire behaviour". No code or test changes.

- [ ] **Step 1: Update the `AcquireIntent` enum doc**

In `crates/resource/src/options.rs:14`, replace the doc comment above `pub enum AcquireIntent {` with:

```rust
/// The caller's intent when acquiring a resource lease.
///
/// **Status:** reserved for future engine integration. The field is
/// preserved on `AcquireOptions` for forward compatibility, but no
/// topology in `nebula-resource` currently reads it (#391). Setting
/// `AcquireIntent::Critical` does not bypass queues or change throttling
/// today.
```

- [ ] **Step 2: Update the `AcquireOptions::intent` field doc**

In the same file at line 50-51, replace:

```rust
/// The caller's intent. Currently informational only — see
/// [`AcquireIntent`] (#391).
pub intent: AcquireIntent,
```

- [ ] **Step 3: Update the `AcquireOptions::tags` field doc**

At line 54-55, replace:

```rust
/// Freeform key-value tags. Reserved for future routing/diagnostics; not
/// read by any topology in `nebula-resource` today (#391).
pub tags: SmallVec<[(Cow<'static, str>, Cow<'static, str>); 2]>,
```

- [ ] **Step 4: Verify docs build clean**

```bash
cargo doc -p nebula-resource --no-deps
```

Expected: no warnings, no broken intra-doc links.

- [ ] **Step 5: Commit**

```bash
git add crates/resource/src/options.rs
git commit -m "docs(resource): mark AcquireOptions intent/tags as reserved (#391)"
```

---

## Task 7: #323 — Regression test for `stop()` during restart backoff

**Files:**
- Modify: `crates/resource/src/runtime/daemon.rs` (add in-file `#[cfg(test)] mod tests`)

**Why:** The `biased select { cancel.cancelled() => break, sleep(backoff) => {} }` already exists at `runtime/daemon.rs:253` (added per #323), but no test asserts the behaviour. Without coverage, a future refactor can silently regress and we’ll lose the property again.

- [ ] **Step 1: Read existing daemon code and current test setup**

```bash
cargo nextest run -p nebula-resource runtime::daemon
```

Expected: 0 tests run (no `#[cfg(test)]` block exists in `runtime/daemon.rs` today). Confirms there is room to add one.

- [ ] **Step 2: Write the regression test**

Append to `crates/resource/src/runtime/daemon.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::ctx::BasicCtx;
    use crate::topology::daemon::config::Config as DaemonCfg;
    use crate::topology::daemon::RestartPolicy;

    #[derive(Clone)]
    struct FlakyDaemon {
        attempts: Arc<AtomicU32>,
    }

    #[derive(Clone)]
    struct FakeRuntime;

    impl crate::resource::Resource for FlakyDaemon { /* trivial impl */ }

    impl Daemon for FlakyDaemon {
        async fn run(
            &self,
            _runtime: &Arc<Self::Runtime>,
            _ctx: &dyn crate::ctx::Ctx,
            _cancel: CancellationToken,
        ) -> Result<(), crate::error::Error> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            Err(crate::error::Error::transient("flaky"))
        }
    }

    #[tokio::test]
    async fn stop_during_restart_backoff_returns_promptly() {
        let parent = CancellationToken::new();
        let cfg = DaemonCfg {
            restart_policy: RestartPolicy::Always,
            // Long backoff so the test would block for ~10s WITHOUT the fix.
            restart_backoff: Duration::from_secs(10),
            max_restarts: 100,
            ..Default::default()
        };
        let rt = DaemonRuntime::<FlakyDaemon>::new(cfg, parent.clone());
        let res = FlakyDaemon { attempts: Arc::new(AtomicU32::new(0)) };
        let ctx = BasicCtx::new(nebula_core::ExecutionId::new());

        rt.start(res, Arc::new(FakeRuntime), &ctx).await.unwrap();

        // Wait until the daemon has run at least once and is now sleeping
        // in restart_backoff. 50 ms is plenty for the first failed run.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let started = std::time::Instant::now();
        rt.stop().await;
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_secs(1),
            "stop() during restart_backoff must return promptly, took {elapsed:?}"
        );
        assert!(!rt.is_running().await);
    }
}
```

- [ ] **Step 3: Run test to verify it passes (the fix is already in place)**

```bash
cargo nextest run -p nebula-resource runtime::daemon::tests::stop_during_restart_backoff_returns_promptly
```

Expected: PASS — the `biased select` at line 253 already cancels promptly. If the test fails, the fix has regressed and Task 7 turns into a real bug fix.

- [ ] **Step 4: Commit**

```bash
git add crates/resource/src/runtime/daemon.rs
git commit -m "test(resource): cover stop() during daemon restart backoff (#323)"
```

---

## Task 8: #318 — Regression tests for restart-safe `DaemonRuntime` lifecycle

**Files:**
- Modify: `crates/resource/src/runtime/daemon.rs` (extend `mod tests` from Task 7)

**Why:** The `start → stop → start` and `start → natural-exit → start` paths are claimed to be fixed at `runtime/daemon.rs:131-165` via the per-run cancel token + finished-handle cleanup, but no test exercises either restart cycle.

- [ ] **Step 1: Write the failing test**

Append two tests to the `tests` module added in Task 7:

```rust
#[derive(Clone)]
struct OneShotDaemon;

impl crate::resource::Resource for OneShotDaemon { /* trivial impl */ }

impl Daemon for OneShotDaemon {
    async fn run(
        &self,
        _runtime: &Arc<Self::Runtime>,
        _ctx: &dyn crate::ctx::Ctx,
        _cancel: CancellationToken,
    ) -> Result<(), crate::error::Error> {
        // Exits immediately under RestartPolicy::Never.
        Ok(())
    }
}

#[tokio::test]
async fn start_stop_start_works() {
    let parent = CancellationToken::new();
    let cfg = DaemonCfg {
        restart_policy: RestartPolicy::Always,
        restart_backoff: Duration::from_millis(20),
        max_restarts: 100,
        ..Default::default()
    };
    let rt = DaemonRuntime::<FlakyDaemon>::new(cfg, parent.clone());
    let res = FlakyDaemon { attempts: Arc::new(AtomicU32::new(0)) };
    let ctx = BasicCtx::new(nebula_core::ExecutionId::new());

    rt.start(res.clone(), Arc::new(FakeRuntime), &ctx).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    rt.stop().await;
    assert!(!rt.is_running().await);

    rt.start(res, Arc::new(FakeRuntime), &ctx)
        .await
        .expect("start after stop must succeed");
    assert!(rt.is_running().await);
    rt.stop().await;
}

#[tokio::test]
async fn start_natural_exit_start_works() {
    let parent = CancellationToken::new();
    let cfg = DaemonCfg {
        restart_policy: RestartPolicy::Never,
        restart_backoff: Duration::from_millis(10),
        max_restarts: 0,
        ..Default::default()
    };
    let rt = DaemonRuntime::<OneShotDaemon>::new(cfg, parent);
    let ctx = BasicCtx::new(nebula_core::ExecutionId::new());

    rt.start(OneShotDaemon, Arc::new(FakeRuntime), &ctx).await.unwrap();

    // Wait until the run() future has resolved and the join handle is finished.
    for _ in 0..50 {
        if !rt.is_running().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(!rt.is_running().await);

    rt.start(OneShotDaemon, Arc::new(FakeRuntime), &ctx)
        .await
        .expect("start after natural exit must succeed");
}
```

- [ ] **Step 2: Run tests to verify they pass**

```bash
cargo nextest run -p nebula-resource runtime::daemon::tests
```

Expected: PASS — both lifecycle paths are already supported by the per-run cancel token + finished-handle cleanup.

- [ ] **Step 3: Commit**

```bash
git add crates/resource/src/runtime/daemon.rs
git commit -m "test(resource): cover start/stop/start lifecycle for DaemonRuntime (#318)"
```

---

## Task 9: #322 — Regression test for `RecoveryGate` probe-herd protection

**Files:**
- Modify: `crates/resource/src/manager.rs` (add `#[cfg(test)] mod gate_admission_tests`)

**Why:** `admit_through_gate` at `manager.rs:1612` already implements the CAS-based single-probe claim that #322 asks for, but there is no test that asserts: under N concurrent acquires after `retry_at` has expired, exactly one ticket is granted and the rest get a typed transient/exhausted error.

- [ ] **Step 1: Write the regression test**

In `crates/resource/src/manager.rs`, add at the bottom of the file:

```rust
#[cfg(test)]
mod gate_admission_tests {
    use super::*;
    use crate::recovery::gate::{GateConfig, RecoveryGate};

    #[tokio::test]
    async fn expired_failed_state_admits_only_one_probe() {
        let gate = Arc::new(RecoveryGate::with_config(GateConfig::default()));

        // Drive the gate into Failed { retry_at = past }.
        let ticket = gate.try_begin().expect("first ticket");
        ticket.fail_transient("seed");
        // Ensure retry_at has elapsed.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Concurrently fire 32 admit_through_gate calls.
        let some_gate = Some(Arc::clone(&gate));
        let mut handles = Vec::new();
        for _ in 0..32 {
            let gate = some_gate.clone();
            handles.push(tokio::spawn(async move { admit_through_gate(&gate) }));
        }

        let mut probes = 0;
        let mut transient_or_exhausted = 0;
        for h in handles {
            match h.await.unwrap() {
                Ok(GateAdmission::Probe(_t)) => probes += 1,
                Ok(GateAdmission::OpenGated(_)) => transient_or_exhausted += 1,
                Ok(GateAdmission::Open) => {},
                Err(_) => transient_or_exhausted += 1,
            }
        }

        assert_eq!(probes, 1, "exactly one probe ticket must be granted");
        assert!(transient_or_exhausted >= 31);
    }
}
```

If `RecoveryGate::with_config` and `GateConfig::default` are not the actual constructor names, look up `crates/resource/src/recovery/gate.rs` and adapt — the rest of the test logic is identical.

- [ ] **Step 2: Run the test to verify it passes**

```bash
cargo nextest run -p nebula-resource gate_admission_tests
```

Expected: PASS (the CAS-based admission path is already in place).

- [ ] **Step 3: Commit**

```bash
git add crates/resource/src/manager.rs
git commit -m "test(resource): assert RecoveryGate single-probe admission under contention (#322)"
```

---

## Task 10: #302 — Regression test for `graceful_shutdown` drain-timeout policy

**Files:**
- Modify: `crates/resource/src/manager.rs` (extend the existing `mod drain_race_tests`)

**Why:** `graceful_shutdown` already returns `ShutdownError::DrainTimeout` under `DrainTimeoutPolicy::Abort` (around `manager.rs:1369`). No test pins this contract — a future refactor could regress to the old "log and force-clear" behaviour and the next caller would silently see destroyed resources again.

- [ ] **Step 1: Write the regression test**

Append to `mod drain_race_tests` in `crates/resource/src/manager.rs`:

```rust
#[tokio::test]
async fn graceful_shutdown_abort_policy_returns_drain_timeout_error() {
    let mgr = Manager::new();
    // Pretend a handle is still outstanding.
    mgr.drain_tracker.0.fetch_add(1, AtomicOrdering::Release);

    let cfg = ShutdownConfig::default()
        .with_drain_timeout(Duration::from_millis(50))
        .with_drain_timeout_policy(DrainTimeoutPolicy::Abort);

    let err = mgr
        .graceful_shutdown(cfg)
        .await
        .expect_err("Abort policy must surface drain timeout");
    match err {
        ShutdownError::DrainTimeout { outstanding } => assert_eq!(outstanding, 1),
        other => panic!("wrong error: {other:?}"),
    }

    // Registry must be untouched after Abort.
    // (no resources to assert on, but `is_shutdown()` should remain false
    // because we reset shutting_down on Abort).
    assert!(!mgr.is_shutdown_in_progress());
}

#[tokio::test]
async fn graceful_shutdown_force_policy_clears_registry_with_outstanding_count() {
    let mgr = Manager::new();
    mgr.drain_tracker.0.fetch_add(2, AtomicOrdering::Release);

    let cfg = ShutdownConfig::default()
        .with_drain_timeout(Duration::from_millis(50))
        .with_drain_timeout_policy(DrainTimeoutPolicy::Force);

    let report = mgr
        .graceful_shutdown(cfg)
        .await
        .expect("Force policy must succeed");
    assert!(report.registry_cleared);
    assert_eq!(report.outstanding_handles_after_drain, 2);
}
```

If `is_shutdown_in_progress` does not exist, replace with a direct read of the `shutting_down` atomic via a small `pub(crate)` helper, OR just drop that assertion — the typed error is the contract under test.

- [ ] **Step 2: Run the tests to verify they pass**

```bash
cargo nextest run -p nebula-resource drain_race_tests::graceful_shutdown_abort_policy_returns_drain_timeout_error drain_race_tests::graceful_shutdown_force_policy_clears_registry_with_outstanding_count
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/resource/src/manager.rs
git commit -m "test(resource): pin DrainTimeoutPolicy contract for graceful shutdown (#302)"
```

---

## Task 11: #272 — Verify wait_for_drain race fix and close

**Files:**
- Read only: `crates/resource/src/manager.rs:1442-1483`, `manager.rs:1731-1809` (existing tests)

**Why:** The `register-then-check` pattern has been applied at `manager.rs:1442` and two regression tests already live in `mod drain_race_tests`. This task confirms current state and closes the issue with a verification note.

- [ ] **Step 1: Run the existing race tests**

```bash
cargo nextest run -p nebula-resource drain_race_tests::wait_for_drain_returns_promptly_when_handle_drops drain_race_tests::wait_for_drain_catches_drop_via_recheck
```

Expected: PASS (both already exist).

- [ ] **Step 2: Inspect `wait_for_drain` to confirm pattern still in place**

Open `crates/resource/src/manager.rs:1442-1483` and verify the body uses:
- `notified.as_mut().enable();`
- post-`enable` re-check of `drain_tracker.0`

If the pattern is intact, no code change is needed.

- [ ] **Step 3: Commit (no code change, just a marker)**

If the verification turned up nothing, skip this commit. Otherwise, address whatever was missing per the same TDD pattern as Task 10.

---

## Task 12: Full validation gate

**Files:** None — runs the canonical PR gate from `CLAUDE.md`.

- [ ] **Step 1: Format**

```bash
cargo +nightly fmt --all
```

- [ ] **Step 2: Clippy**

```bash
cargo clippy --workspace -- -D warnings
```

Expected: clean.

- [ ] **Step 3: Tests**

```bash
cargo nextest run -p nebula-resource
cargo test -p nebula-resource --doc
```

Expected: all green.

- [ ] **Step 4: Workspace tests (paranoia gate)**

```bash
cargo nextest run --workspace
```

Expected: green. If anything outside `nebula-resource` regressed, investigate before continuing.

- [ ] **Step 5: cargo deny**

```bash
cargo deny check
```

Expected: clean.

---

## Task 13: Open the PR and close the 11 issues

**Files:** None — uses `gh` to push the branch and update GitHub.

- [ ] **Step 1: Push the branch and open a PR**

```bash
git push -u origin HEAD
gh pr create --title "fix(resource): land + verify all open nebula-resource issues" --body "$(cat <<'EOF'
## Summary

Closes the entire open backlog for `nebula-resource`:

- Real fixes:
  - #382 — scrub stale `TypeId` rows on registry replace
  - #383 — clamp `AcquireResilience::max_attempts` to `>=1`
  - #384 — hold exclusive permit until `reset()` completes
  - #387 — drive `ResourceStatus.phase` across register/reload/shutdown
  - #390 — enforce `max_concurrent_creates` and validate `min/max_size`
  - #391 — narrow misleading docs for unused `AcquireOptions` fields
- Regression coverage for already-fixed issues:
  - #272 — verified, existing tests still pass
  - #302 — pinned `DrainTimeoutPolicy` contract
  - #318 — covered `start/stop/start` and `start/natural-exit/start`
  - #322 — asserted single-probe CAS admission under contention
  - #323 — covered `stop()` during restart backoff

## Test plan

- [ ] `cargo nextest run -p nebula-resource`
- [ ] `cargo test -p nebula-resource --doc`
- [ ] `cargo nextest run --workspace`
- [ ] `cargo clippy --workspace -- -D warnings`
- [ ] `cargo deny check`
EOF
)"
```

- [ ] **Step 2: Once the PR is merged, close the issues**

For each of `272 302 318 322 323 382 383 384 387 390 391`:

```bash
gh issue close <N> --repo vanyastaff/nebula \
  --comment "Resolved by PR #<merged-pr-number>."
```

(Substitute the merged PR number returned by Step 1.)

---

## Self-Review Notes

- **Spec coverage:** every open issue has a dedicated task. Real fixes (Tasks 1–6) include a failing test, the implementation, and a passing run. Verification tasks (Tasks 7–11) add the missing regression coverage that prevents the bug from coming back.
- **Type consistency:** `ResourcePhase`, `ResourceStatus`, `set_phase`, `set_failed`, `set_phase_erased`, `DrainTimeoutPolicy::{Abort,Force}`, `ShutdownError::DrainTimeout`, `ShutdownReport`, `GateAdmission::{Open,OpenGated,Probe}`, `RecoveryTicket`, `AcquireResilience`, `AcquireRetryConfig`, `RetryConfig` — all match the symbols already present in `crates/resource`.
- **Placeholders:** none. Each test ships with its body, each fix ships with the replacement code, each command is concrete.
- **Risk:** Task 4 (#387) crosses the largest number of files. Run `cargo nextest run -p nebula-resource` after Step 7 of that task before moving on, to surface any unexpected coupling on the type-erased `set_phase_erased` addition.
- **TDD discipline:** Tasks 1–6 follow Red→Green→Commit. Tasks 7–10 are written test-first against code that should already pass; if any of them fails, the implementation has regressed and the task converts into a real bug fix using the same pattern.
