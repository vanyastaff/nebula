//! Bounded per-key plugin-process pool with a type-enforced RAII lease.
//!
//! A long-lived out-of-process plugin connection ([`ProcessSandbox`]) is
//! expensive to establish (spawn + dial + handshake) and **stateful**: its
//! request/response stream must stay in lock-step. Two correctness hazards
//! follow from that and are the entire reason this module exists:
//!
//! 1. **Permit accounting must be exact.** Capacity is enforced by a
//!    [`Semaphore`]; if a permit could leak on a cancellation or panic the
//!    pool would slowly wedge (every future `acquire` on that key blocks
//!    forever). The permit therefore lives *inside* the [`Lease`] and the
//!    only release path is the lease's `Drop` — there is no manual
//!    `add_permits`, so every exit (`return`, `?`, panic-unwind,
//!    await-cancellation, early drop) releases exactly once.
//! 2. **A desynced connection must never be re-pooled.** If a caller
//!    observed a transport error mid-round-trip the stream position is
//!    undefined. Returning that connection to the idle set and handing it
//!    to a *different* caller would misattribute one execution's response
//!    to another — a silent cross-execution data leak. The lease is
//!    poison-gated: [`Lease::poison`] marks the connection unusable and
//!    `Drop` then drops it (the spawned child is `kill_on_drop`, so the OS
//!    SIGKILLs the plugin) instead of returning it to idle.
//!
//! The pool is generic over the pooled connection type ([`PooledConn`]) so
//! the invariants can be unit-tested with a lightweight fake — constructing
//! a real [`ProcessSandbox`] would require spawning a plugin binary. The
//! production instantiation is `PluginPool<ProcessSandbox>`.
//!
//! Scope note: this module is the *mechanism* only. It does not spawn
//! connections itself (the caller supplies a spawn closure), does not know
//! how a [`PoolKey`] is derived from a workflow node, and is wired to no
//! dispatch path here. Capacity policy is a constructor parameter, not a
//! baked-in default.
#![allow(
    dead_code,
    reason = "self-contained pool mechanism landed ahead of its dispatch consumer; \
              the API surface (PoolKey/PluginPool/Lease/PoolRegistry) is exercised by \
              the in-module invariant tests and consumed by the dispatch wiring in a \
              follow-up — the consumer is deliberately a separate change"
)]

use std::{path::PathBuf, sync::Arc};

use dashmap::DashMap;
use nebula_sandbox::{ProcessSandbox, ScopeHash};
use parking_lot::Mutex;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// A connection that can be pooled and reused.
///
/// The pool cannot introspect an opaque transport's health, so the pooled
/// type self-reports via [`is_healthy`](PooledConn::is_healthy). For
/// [`ProcessSandbox`] this is conservatively `true`: the sandbox clears its
/// own cached handle internally on any transport error and respawns on the
/// next call, so a *returned* (non-[`poison`](Lease::poison)ed) connection
/// is safe to reuse. The lease's explicit poison flag is the primary,
/// type-enforced guard against re-pooling a connection a caller saw fail;
/// `is_healthy` is the secondary check.
pub(crate) trait PooledConn: Send + 'static {
    /// `true` if this connection may be returned to the idle set for reuse
    /// by a subsequent acquirer. A `false` here causes [`Lease::drop`] to
    /// discard the connection (for [`ProcessSandbox`] that means the child
    /// is SIGKILLed via `kill_on_drop`).
    fn is_healthy(&self) -> bool;
}

impl PooledConn for ProcessSandbox {
    fn is_healthy(&self) -> bool {
        // `ProcessSandbox` owns its own defense-in-depth poisoning: any
        // transport error clears its cached handle and the next call
        // respawns. A connection that is *returned* to the pool (i.e. the
        // caller did not `lease.poison()` it) is therefore reusable. The
        // lease poison flag — not this method — is the guard against the
        // dangerous case (a caller that saw a desync re-pooling it).
        true
    }
}

/// Identity that buckets connections into independent pools.
///
/// Two invocations with the same binary but a different bound
/// credential-slot set ([`ScopeHash`]) MUST NOT share a process (ADR-0025
/// §2 isolation), hence the scope is part of the key, not just the binary
/// path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PoolKey {
    /// Plugin binary path.
    binary: PathBuf,
    /// Credential-scope identity (ADR-0025 §2 per-process isolation key).
    scope: ScopeHash,
}

impl PoolKey {
    /// Construct a pool key from a plugin binary path and its
    /// credential-scope identity.
    pub(crate) fn new(binary: PathBuf, scope: ScopeHash) -> Self {
        Self { binary, scope }
    }
}

/// Bounded pool of reusable connections for a single [`PoolKey`].
///
/// Capacity is `max_per_key` permits on a [`Semaphore`]; the idle set never
/// exceeds that because a connection only exists while a permit is held and
/// is only returned to idle as its lease (hence its permit) drops. The
/// `Mutex<Vec<_>>` idle stack is `parking_lot` (already a workspace dep) —
/// guarded sections are O(1) push/pop with no `.await` held across the
/// lock.
pub(crate) struct PluginPool<T: PooledConn> {
    /// Idle (warm, reusable) connections. LIFO: a just-returned connection
    /// is the most likely to still be warm.
    idle: Mutex<Vec<T>>,
    /// Capacity gate. `max_per_key` permits; an [`OwnedSemaphorePermit`]
    /// lives in every outstanding [`Lease`].
    sem: Arc<Semaphore>,
}

impl<T: PooledConn> PluginPool<T> {
    /// Create an empty pool with `max_per_key` concurrent-connection
    /// capacity.
    ///
    /// `max_per_key` is the mechanism's only policy input — the caller (the
    /// lead's wiring, out of scope here) decides the value. A zero capacity
    /// would deadlock every `acquire`; callers must pass a positive value.
    pub(crate) fn new(max_per_key: usize) -> Arc<Self> {
        Arc::new(Self {
            idle: Mutex::new(Vec::with_capacity(max_per_key)),
            sem: Arc::new(Semaphore::new(max_per_key)),
        })
    }

    /// Acquire a leased connection, waiting for a free permit if the pool
    /// is at capacity.
    ///
    /// Order is deliberate: the permit is awaited **first**, so an idle
    /// connection is only popped (or `spawn` invoked) once capacity is
    /// secured — never spawn beyond `max_per_key`. `spawn` is called only
    /// when no warm connection is available; the pool does not assume how a
    /// `T` is built (the real spawn+dial wiring is the lead's).
    ///
    /// `spawn` is **fallible**: spawning a child plugin (fork/exec, dial,
    /// handshake) can fail (`ENOEXEC`, `EMFILE`, handshake timeout). On a
    /// spawn failure no [`Lease`] is created and the just-acquired permit is
    /// dropped here — released exactly once, so a spawn failure surfaces as
    /// a per-call error and never leaks a permit or wedges the pool for the
    /// key. The returned [`Lease`] owns the permit; dropping it is the sole
    /// release on the success path.
    pub(crate) async fn acquire<E>(
        self: &Arc<Self>,
        spawn: impl FnOnce() -> Result<T, E>,
    ) -> Result<Lease<T>, E> {
        // `acquire_owned` ties the permit's lifetime to the `Lease`, not to
        // a borrow of the pool — the lease is `'static` and the permit is
        // released purely by its `Drop`. `Semaphore::close` is never called
        // on this semaphore, so `acquire_owned` cannot return `Err`; the
        // `Result` is collapsed without `unwrap`/`expect` so a future code
        // change that *does* close it degrades to a fresh spawn rather than
        // panicking inside the pool.
        let permit = Arc::clone(&self.sem)
            .acquire_owned()
            .await
            .map_err(|_| ())
            .ok();

        let conn = {
            let mut idle = self.idle.lock();
            idle.pop()
        };
        // `spawn()?` on the no-idle path: an early return here drops `permit`
        // (released exactly once), creates no `Lease`, and propagates the
        // spawn error to the caller — a spawn failure is a per-call error,
        // not a wedged pool or a panicked dispatch task.
        let conn = match conn {
            Some(c) => c,
            None => spawn()?,
        };

        Ok(Lease {
            conn: Some(conn),
            poisoned: false,
            _permit: permit,
            pool: Arc::clone(self),
        })
    }

    /// Current idle-connection count. Test-only: production code never
    /// inspects pool internals (it acquires and drops leases).
    #[cfg(test)]
    fn idle_len(&self) -> usize {
        self.idle.lock().len()
    }

    /// Currently-available permits. Test-only.
    #[cfg(test)]
    fn available_permits(&self) -> usize {
        self.sem.available_permits()
    }

    /// Return a connection to the idle set. Called only from
    /// [`Lease::drop`] and only for a healthy, non-poisoned connection.
    fn push_idle(&self, conn: T) {
        self.idle.lock().push(conn);
    }
}

/// RAII handle to a pooled connection.
///
/// Two invariants are enforced structurally by this type, not by
/// call-site discipline:
///
/// - **Permit released exactly once.** The [`OwnedSemaphorePermit`] is a
///   field; its destructor runs on every drop path (normal return, `?`,
///   panic unwind, future cancellation, early `drop`). There is no other
///   release path, so the count cannot leak or double-release.
/// - **Poison-gated return.** A caller that observed a transport error
///   calls [`poison`](Self::poison); `Drop` then discards the connection
///   (SIGKILL via `kill_on_drop`) instead of returning it. A desynced
///   connection therefore can never reach a different caller.
///
/// The permit field is intentionally read only by `Drop`; the
/// `#[allow(dead_code)]` documents that the *value's lifetime*, not any
/// method call, is what enforces invariant 1.
pub(crate) struct Lease<T: PooledConn> {
    /// The leased connection. `Option` so `Drop` can move it out to either
    /// return it to the pool or drop it (poison/unhealthy). `Some` for the
    /// entire normal lifetime of the lease.
    conn: Option<T>,
    /// Set by [`poison`](Self::poison) when the holder saw a transport
    /// failure. Gates the return-to-idle decision in `Drop`.
    poisoned: bool,
    /// Capacity permit. Held for the lease's lifetime; released solely by
    /// this field's `Drop`. `Option` because `acquire_owned` is collapsed
    /// to `Option` (see `PluginPool::acquire`); a `None` only occurs if the
    /// semaphore were closed, which this module never does.
    #[allow(
        dead_code,
        reason = "value lifetime — not a method call — releases the permit on every drop path"
    )]
    _permit: Option<OwnedSemaphorePermit>,
    /// Owning pool, for the return path in `Drop`.
    pool: Arc<PluginPool<T>>,
}

impl<T: PooledConn> Lease<T> {
    /// Mark the leased connection as unusable.
    ///
    /// Call this when a transport error, timeout, cancellation, or any
    /// other event leaves the connection's request/response stream in an
    /// undefined position. After `poison`, `Drop` discards the connection
    /// rather than returning it to the idle set — the next acquirer on this
    /// key spawns a fresh connection instead of inheriting a desynced one.
    ///
    /// Idempotent and infallible: poisoning is a one-way latch.
    pub(crate) fn poison(&mut self) {
        self.poisoned = true;
    }

    /// Shared access to the leased connection.
    ///
    /// `Some` for the lease's whole lifetime; the `Option` exists only so
    /// `Drop` can move the connection out.
    pub(crate) fn get(&self) -> Option<&T> {
        self.conn.as_ref()
    }
}

impl<T: PooledConn> Drop for Lease<T> {
    fn drop(&mut self) {
        // Move the connection out so we either re-pool it or drop it here.
        if let Some(conn) = self.conn.take() {
            // Return to idle ONLY if the holder did not poison it AND the
            // connection self-reports healthy. Either gate failing means
            // the connection is discarded — `T`'s own `Drop` runs (for
            // `ProcessSandbox` that SIGKILLs the child via `kill_on_drop`),
            // so a desynced connection is destroyed, never handed to the
            // next, different caller.
            if !self.poisoned && conn.is_healthy() {
                self.pool.push_idle(conn);
            } else {
                tracing::debug!(
                    poisoned = self.poisoned,
                    "plugin pool lease dropped a connection instead of re-pooling \
                     it (poisoned or unhealthy); connection destroyed"
                );
                drop(conn);
            }
        }
        // `_permit` (if `Some`) is dropped here automatically, releasing
        // exactly one semaphore permit. This is the only release path.
    }
}

/// Registry of per-key pools.
///
/// `DashMap` (already a workspace + engine dependency) gives lock-free
/// sharded reads; one `Arc<PluginPool<T>>` per distinct [`PoolKey`] so
/// distinct keys have fully independent capacity and idle sets. The
/// `RwLock<HashMap>` fallback noted in the task is unnecessary because
/// `dashmap` is already a workspace dependency.
pub(crate) struct PoolRegistry<T: PooledConn> {
    pools: DashMap<PoolKey, Arc<PluginPool<T>>>,
    /// Per-key capacity applied to pools created on demand.
    max_per_key: usize,
}

impl<T: PooledConn> PoolRegistry<T> {
    /// Create a registry whose on-demand pools each get `max_per_key`
    /// capacity.
    pub(crate) fn new(max_per_key: usize) -> Self {
        Self {
            pools: DashMap::new(),
            max_per_key,
        }
    }

    /// Get (creating if absent) the pool for `key`.
    ///
    /// Distinct keys map to distinct `Arc<PluginPool<T>>` values, so a
    /// different binary OR a different [`ScopeHash`] yields an independent
    /// pool with its own capacity — the ADR-0025 §2 isolation boundary.
    pub(crate) fn pool_for(&self, key: &PoolKey) -> Arc<PluginPool<T>> {
        if let Some(existing) = self.pools.get(key) {
            return Arc::clone(existing.value());
        }
        // Two racing inserts for the same key: `entry` serializes them so
        // exactly one pool is created and shared.
        Arc::clone(
            self.pools
                .entry(key.clone())
                .or_insert_with(|| PluginPool::new(self.max_per_key))
                .value(),
        )
    }

    /// Number of distinct pools currently registered. Test-only.
    #[cfg(test)]
    fn pool_count(&self) -> usize {
        self.pools.len()
    }
}

/// Compile-time guarantee that `Lease<ProcessSandbox>` — the production
/// instantiation — actually satisfies the pool's bounds. If a future
/// `ProcessSandbox` change broke `PooledConn`, this fails the build rather
/// than only failing once a consumer (the lead's wiring) is added.
const _: () = {
    fn assert_pooled<T: PooledConn>() {}
    fn assert_process_sandbox_is_pooled() {
        assert_pooled::<ProcessSandbox>();
    }
    let _ = assert_process_sandbox_is_pooled;
};

#[cfg(test)]
mod tests {
    //! Proof of the three structural invariants — permit-exactly-once,
    //! poison-gated return, key isolation — plus the concurrency bound.
    //!
    //! Tests use a fake `PooledConn` rather than a real `ProcessSandbox`:
    //! constructing the latter would require spawning a plugin binary, and
    //! the invariants under test (permit accounting, poison gating, key
    //! isolation, concurrency) are transport-agnostic and fully exercised
    //! by a fake whose health is controllable and whose `Drop` is
    //! observable.

    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use nebula_sandbox::scope_hash;

    use super::*;

    /// Per-test shared counters: how many fakes were spawned and how many
    /// were dropped (destroyed, i.e. NOT returned to idle).
    #[derive(Default)]
    struct Counters {
        spawned: AtomicUsize,
        dropped: AtomicUsize,
    }

    /// Fake pooled connection. `healthy` models a connection that self-
    /// reports unusable even without an explicit `poison()` call.
    struct FakeConn {
        healthy: bool,
        counters: Arc<Counters>,
    }

    impl FakeConn {
        fn spawn(counters: &Arc<Counters>) -> Result<Self, &'static str> {
            counters.spawned.fetch_add(1, Ordering::SeqCst);
            Ok(Self {
                healthy: true,
                counters: Arc::clone(counters),
            })
        }

        fn spawn_unhealthy(counters: &Arc<Counters>) -> Result<Self, &'static str> {
            counters.spawned.fetch_add(1, Ordering::SeqCst);
            Ok(Self {
                healthy: false,
                counters: Arc::clone(counters),
            })
        }
    }

    impl PooledConn for FakeConn {
        fn is_healthy(&self) -> bool {
            self.healthy
        }
    }

    impl Drop for FakeConn {
        fn drop(&mut self) {
            self.counters.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn key(binary: &str, slots: &[&str]) -> PoolKey {
        PoolKey::new(PathBuf::from(binary), scope_hash(slots))
    }

    // ---- invariant 1: permit released exactly once ------------------

    #[tokio::test]
    async fn permit_released_on_normal_drop_allows_next_acquire() {
        let counters = Arc::new(Counters::default());
        let pool = PluginPool::new(1);

        let lease = pool
            .acquire(|| FakeConn::spawn(&counters))
            .await
            .expect("fake spawn is infallible");
        assert_eq!(pool.available_permits(), 0, "permit held while leased");

        drop(lease);
        assert_eq!(
            pool.available_permits(),
            1,
            "exactly one permit released on normal drop"
        );

        // Capacity is genuinely free again: a second acquire proceeds
        // without blocking (it reuses the healthy returned conn).
        let _lease2 = pool
            .acquire(|| FakeConn::spawn(&counters))
            .await
            .expect("fake spawn is infallible");
        assert_eq!(pool.available_permits(), 0);
    }

    #[tokio::test]
    async fn permit_released_on_panic_unwind() {
        let counters = Arc::new(Counters::default());
        let pool = PluginPool::new(1);

        // Lease is created then a panic unwinds through its scope. The
        // permit must still be released by `Lease::Drop` running during
        // unwind — otherwise the pool wedges forever.
        let pool_in = Arc::clone(&pool);
        let counters_in = Arc::clone(&counters);
        let result = std::panic::AssertUnwindSafe(async move {
            let _lease = pool_in
                .acquire(|| FakeConn::spawn(&counters_in))
                .await
                .expect("fake spawn is infallible");
            panic!("simulated holder panic while leasing");
        });
        // Drive the future to the panic point on the current runtime.
        let join = tokio::spawn(result.0);
        let joined = join.await;
        assert!(joined.is_err(), "task must have panicked");

        assert_eq!(
            pool.available_permits(),
            1,
            "permit released exactly once during panic unwind"
        );
        // The lease was never poisoned and the fake is healthy, so
        // `Lease::Drop` running during the unwind returns it to idle — the
        // permit accounting is the invariant under test here, and it must
        // stay exact even when the holder dies abnormally.
        assert_eq!(
            pool.idle_len(),
            1,
            "a healthy un-poisoned connection is still re-pooled on a panic-unwind drop"
        );
        assert_eq!(
            counters.dropped.load(Ordering::SeqCst),
            0,
            "healthy connection re-pooled (not destroyed) — it is only dropped \
             when the pool itself is dropped at end of test"
        );
    }

    #[tokio::test]
    async fn permit_released_on_early_return() {
        let counters = Arc::new(Counters::default());
        let pool = PluginPool::new(1);

        // A helper that acquires and then early-returns via `?`-style
        // control flow without explicitly dropping the lease.
        async fn use_then_return(
            pool: &Arc<PluginPool<FakeConn>>,
            counters: &Arc<Counters>,
        ) -> Result<(), &'static str> {
            let _lease = pool
                .acquire(|| FakeConn::spawn(counters))
                .await
                .expect("fake spawn is infallible");
            // Early return; `_lease` drops here on the error path.
            Err("early out")
        }

        let r = use_then_return(&pool, &counters).await;
        assert!(r.is_err());
        assert_eq!(
            pool.available_permits(),
            1,
            "permit released exactly once on early return"
        );
    }

    /// Guards: a spawn failure must not leak a permit or wedge the pool.
    #[tokio::test]
    async fn spawn_failure_releases_permit_and_pool_not_wedged() {
        let counters = Arc::new(Counters::default());
        let pool = PluginPool::new(1);

        // No idle connection → `spawn` runs and fails. No `Lease` is built,
        // so the just-acquired permit is dropped on the failure path.
        let res = pool
            .acquire(|| Err::<FakeConn, &'static str>("spawn boom"))
            .await;
        assert!(res.is_err(), "a failing spawn surfaces as a per-call error");
        assert_eq!(
            res.err(),
            Some("spawn boom"),
            "the spawn error propagates verbatim to the caller"
        );

        assert_eq!(
            pool.available_permits(),
            1,
            "the permit was released exactly once on the spawn-failure path \
             (not leaked) — this is the whole point of the fix"
        );
        assert_eq!(pool.idle_len(), 0, "a failed spawn pools no connection");

        // The prior spawn failure must not have wedged the pool for this
        // key: a subsequent acquire still gets a permit and a connection.
        let _ok = pool
            .acquire(|| FakeConn::spawn(&counters))
            .await
            .expect("pool not wedged after a prior spawn failure");
        assert_eq!(
            pool.available_permits(),
            0,
            "a prior spawn failure did not wedge the pool for that key"
        );
    }

    // ---- invariant 2: poison-gated return ---------------------------

    #[tokio::test]
    async fn poisoned_connection_is_not_returned_to_idle() {
        let counters = Arc::new(Counters::default());
        let pool = PluginPool::new(2);

        let mut lease = pool
            .acquire(|| FakeConn::spawn(&counters))
            .await
            .expect("fake spawn is infallible");
        lease.poison();
        drop(lease);

        assert_eq!(
            pool.idle_len(),
            0,
            "poisoned connection must NOT grow the idle set"
        );
        assert_eq!(
            counters.dropped.load(Ordering::SeqCst),
            1,
            "poisoned connection must be destroyed on drop"
        );

        // The next acquire cannot reuse it: it must spawn a fresh one.
        let spawned_before = counters.spawned.load(Ordering::SeqCst);
        let _fresh = pool
            .acquire(|| FakeConn::spawn(&counters))
            .await
            .expect("fake spawn is infallible");
        assert_eq!(
            counters.spawned.load(Ordering::SeqCst),
            spawned_before + 1,
            "a poisoned connection must force a fresh spawn, never be reused"
        );
    }

    #[tokio::test]
    async fn unhealthy_connection_is_not_returned_even_without_explicit_poison() {
        let counters = Arc::new(Counters::default());
        let pool = PluginPool::new(1);

        // Connection self-reports unhealthy; holder never calls poison().
        // The `is_healthy()` gate alone must keep it out of idle.
        let lease = pool
            .acquire(|| FakeConn::spawn_unhealthy(&counters))
            .await
            .expect("fake spawn is infallible");
        drop(lease);

        assert_eq!(
            pool.idle_len(),
            0,
            "self-reported-unhealthy connection must not be re-pooled"
        );
    }

    #[tokio::test]
    async fn healthy_connection_is_returned_and_reused() {
        let counters = Arc::new(Counters::default());
        let pool = PluginPool::new(1);

        let lease = pool
            .acquire(|| FakeConn::spawn(&counters))
            .await
            .expect("fake spawn is infallible");
        drop(lease);
        assert_eq!(pool.idle_len(), 1, "healthy connection returned to idle");
        assert_eq!(counters.spawned.load(Ordering::SeqCst), 1);

        // Second acquire must reuse the idle connection — no new spawn.
        let lease2 = pool
            .acquire(|| FakeConn::spawn(&counters))
            .await
            .expect("fake spawn is infallible");
        assert_eq!(
            counters.spawned.load(Ordering::SeqCst),
            1,
            "healthy idle connection must be reused, not respawned"
        );
        drop(lease2);
    }

    // ---- invariant 3: distinct PoolKeys are isolated ----------------

    #[tokio::test]
    async fn distinct_keys_have_independent_pools_and_capacity() {
        let registry: PoolRegistry<FakeConn> = PoolRegistry::new(1);
        let counters = Arc::new(Counters::default());

        let k_bin_a = key("/bin/a", &["slot"]);
        let k_bin_b = key("/bin/b", &["slot"]); // different binary
        let k_scope = key("/bin/a", &["other"]); // different ScopeHash

        let pool_a = registry.pool_for(&k_bin_a);
        let pool_b = registry.pool_for(&k_bin_b);
        let pool_scope = registry.pool_for(&k_scope);

        assert_eq!(registry.pool_count(), 3, "three distinct keys → 3 pools");
        assert!(
            !Arc::ptr_eq(&pool_a, &pool_b),
            "different binary ⇒ different pool"
        );
        assert!(
            !Arc::ptr_eq(&pool_a, &pool_scope),
            "different ScopeHash ⇒ different pool"
        );

        // Same key returns the same pool instance (shared capacity).
        let pool_a_again = registry.pool_for(&k_bin_a);
        assert!(Arc::ptr_eq(&pool_a, &pool_a_again));
        assert_eq!(registry.pool_count(), 3, "re-fetch must not create a pool");

        // pool_a is saturated at capacity 1; pool_b is independent and
        // still has its own free permit.
        let _lease_a = pool_a
            .acquire(|| FakeConn::spawn(&counters))
            .await
            .expect("fake spawn is infallible");
        assert_eq!(pool_a.available_permits(), 0);
        assert_eq!(
            pool_b.available_permits(),
            1,
            "a different key's pool has independent capacity"
        );
    }

    // ---- concurrency: N concurrent acquires, N+1th waits ------------

    #[tokio::test]
    async fn n_concurrent_acquires_proceed_and_n_plus_one_waits() {
        let counters = Arc::new(Counters::default());
        let capacity = 3usize;
        let pool: Arc<PluginPool<FakeConn>> = PluginPool::new(capacity);

        // Hold all `capacity` leases concurrently — all must proceed.
        let mut leases = Vec::new();
        for _ in 0..capacity {
            leases.push(
                pool.acquire(|| FakeConn::spawn(&counters))
                    .await
                    .expect("fake spawn is infallible"),
            );
        }
        assert_eq!(
            pool.available_permits(),
            0,
            "all N permits taken by N concurrent leases"
        );

        // The N+1th acquire must NOT complete while the pool is saturated.
        let pool_for_waiter = Arc::clone(&pool);
        let counters_for_waiter = Arc::clone(&counters);
        let waiter = tokio::spawn(async move {
            let _l = pool_for_waiter
                .acquire(|| FakeConn::spawn(&counters_for_waiter))
                .await
                .expect("fake spawn is infallible");
        });

        // Give the waiter a chance to run; it must still be parked because
        // no permit is free.
        tokio::task::yield_now().await;
        assert!(
            !waiter.is_finished(),
            "N+1th acquire must block until a lease drops"
        );

        // Drop one lease → its permit frees → the waiter proceeds.
        leases.pop();
        waiter.await.expect("waiter task must join cleanly");

        // Net: one permit was freed by the dropped lease and one is held
        // by the now-completed-then-dropped waiter lease, leaving
        // `capacity - (capacity - 1)` free.
        assert_eq!(
            pool.available_permits(),
            capacity - leases.len(),
            "permit accounting stays exact across concurrent acquire/drop"
        );
    }

    #[tokio::test]
    async fn registry_same_key_concurrent_get_creates_one_pool() {
        // Racing `pool_for` on the same key must converge on a single
        // shared pool (entry() serializes the create).
        let registry: Arc<PoolRegistry<FakeConn>> = Arc::new(PoolRegistry::new(2));
        let k = key("/bin/x", &["s"]);

        let mut handles = Vec::new();
        for _ in 0..8 {
            let reg = Arc::clone(&registry);
            let kk = k.clone();
            handles.push(tokio::spawn(async move { reg.pool_for(&kk) }));
        }
        let mut pools = Vec::new();
        for h in handles {
            pools.push(h.await.expect("pool_for task must join"));
        }
        let first = &pools[0];
        for p in &pools[1..] {
            assert!(
                Arc::ptr_eq(first, p),
                "all racing pool_for calls must share one pool instance"
            );
        }
        assert_eq!(registry.pool_count(), 1, "exactly one pool for the key");
    }
}
