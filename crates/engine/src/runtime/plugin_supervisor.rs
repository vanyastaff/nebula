//! `PluginSupervisor` — owner of the engine's plugin-process pools with a
//! clean drain-on-shutdown.
//!
//! Behind the `out-of-process-plugins` feature. The supervisor owns the
//! per-key [`PoolRegistry`] of pooled [`ProcessSandbox`] processes and is
//! the single thing the gated dispatch path holds: acquisition delegates
//! per key, and [`shutdown`](PluginSupervisor::shutdown) drains every
//! pool's warm connections so their plugin children are SIGKILLed via
//! `kill_on_drop` at a controlled point instead of lingering until
//! process exit.
//!
//! Deliberately minimal: there is **no** state file, persistence,
//! reattach, pidfd, or HMAC. A drained pool simply re-spawns on the next
//! acquire; supervised reattach across host restarts is explicitly out of
//! scope here.

use std::sync::Arc;

use nebula_sandbox::ProcessSandbox;

use crate::runtime::{
    RuntimeError,
    plugin_pool::{Lease, PoolKey, PoolRegistry},
};

/// Owns the engine's plugin-process pools and drains them on shutdown.
///
/// Cloneable view over a shared [`PoolRegistry`] (`Arc` inside) so the
/// dispatch path and a shutdown hook can both hold it. `acquire`
/// delegates to the per-key pool; `shutdown` reclaims all warm
/// connections.
#[derive(Clone)]
pub struct PluginSupervisor {
    registry: Arc<PoolRegistry<ProcessSandbox>>,
}

impl std::fmt::Debug for PluginSupervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginSupervisor").finish_non_exhaustive()
    }
}

impl PluginSupervisor {
    /// Create a supervisor whose on-demand pools each get
    /// `max_processes_per_key` capacity.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::InvalidPoolCapacity`] when
    /// `max_processes_per_key == 0`: each pool's [`Semaphore`] would be
    /// constructed with zero permits, so every `acquire` would block
    /// forever. Rejecting at construction turns a silent runtime hang into
    /// a typed composition-root error.
    ///
    /// [`Semaphore`]: tokio::sync::Semaphore
    pub fn new(max_processes_per_key: usize) -> Result<Self, RuntimeError> {
        if max_processes_per_key == 0 {
            return Err(RuntimeError::InvalidPoolCapacity {
                requested: max_processes_per_key,
            });
        }
        Ok(Self {
            registry: Arc::new(PoolRegistry::new(max_processes_per_key)),
        })
    }

    /// Acquire a leased [`ProcessSandbox`] for `key`, spawning a fresh one
    /// via `spawn` only if no warm connection is available for that key.
    ///
    /// Delegates to the per-key [`PluginPool`](crate::runtime::plugin_pool)
    /// — distinct keys have fully independent capacity and idle sets
    /// . The returned [`Lease`] owns its capacity
    /// permit; dropping it releases the permit exactly once and either
    /// re-pools or (if poisoned) destroys the connection.
    ///
    /// `spawn` is fallible: a spawn failure surfaces as a per-call error
    /// `E` and never leaks a permit or wedges the key's pool.
    ///
    /// `pub(crate)`: `PoolKey` / `Lease` are engine-owned internals
    /// ( — the pool is engine-owned, the keyed type does not
    /// leave the crate). Only the gated dispatch path calls this; the
    /// composition root holds the supervisor solely for
    /// [`shutdown`](Self::shutdown).
    pub(crate) async fn acquire<E>(
        &self,
        key: &PoolKey,
        spawn: impl FnOnce() -> Result<ProcessSandbox, E>,
    ) -> Result<Lease<ProcessSandbox>, E> {
        self.registry.pool_for(key).acquire(spawn).await
    }

    /// Drain every pool's warm connections, returning how many were
    /// destroyed across all keys.
    ///
    /// Each destroyed [`ProcessSandbox`] SIGKILLs its plugin child via
    /// `kill_on_drop`. Connections currently held by an in-flight
    /// [`Lease`] are untouched and die with their lease. The pool `Arc`s
    /// stay registered, so any post-shutdown acquire simply spawns fresh
    /// — there is no reattach.
    pub fn shutdown(&self) -> usize {
        let drained = self.registry.drain_all();
        tracing::debug!(
            target = "engine::plugin_supervisor",
            drained,
            "PluginSupervisor shutdown: drained all pooled plugin processes \
             (kill_on_drop); no state persisted"
        );
        drained
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use nebula_sandbox::scope_hash;

    use super::*;

    fn key(binary: &str, slots: &[&str]) -> PoolKey {
        PoolKey::new(PathBuf::from(binary), scope_hash(slots))
    }

    #[test]
    fn new_rejects_zero_capacity_and_accepts_positive() {
        // A 0-permit semaphore makes every `acquire` block forever, so the
        // supervisor must refuse to construct rather than wedge dispatch.
        let err = PluginSupervisor::new(0).expect_err("zero capacity must be rejected");
        assert!(
            matches!(err, RuntimeError::InvalidPoolCapacity { requested: 0 }),
            "expected RuntimeError::InvalidPoolCapacity {{ requested: 0 }}, got {err:?}"
        );

        // Any positive capacity is accepted.
        PluginSupervisor::new(1).expect("capacity 1 must be accepted");
        PluginSupervisor::new(8).expect("capacity 8 must be accepted");
    }

    #[tokio::test]
    async fn acquire_delegates_per_key_and_isolates_capacity() {
        // Capacity 1 per key. Two distinct keys ⇒ two independent pools:
        // saturating one must not block the other.
        let sup = PluginSupervisor::new(1).expect("capacity 1 is valid");
        let k_a = key("/bin/a", &["s"]);
        let k_b = key("/bin/b", &["s"]); // different binary ⇒ different pool

        let lease_a = sup
            .acquire(&k_a, || {
                Ok::<_, std::convert::Infallible>(ProcessSandbox::new(
                    PathBuf::from("/bin/a"),
                    Duration::from_secs(1),
                ))
            })
            .await
            .expect("first acquire on key A spawns");

        // Key A is saturated (capacity 1, lease held). Key B is a distinct
        // pool and must still hand out a lease without blocking.
        let lease_b = sup
            .acquire(&k_b, || {
                Ok::<_, std::convert::Infallible>(ProcessSandbox::new(
                    PathBuf::from("/bin/b"),
                    Duration::from_secs(1),
                ))
            })
            .await
            .expect("acquire on the distinct key B is independent");

        drop(lease_a);
        drop(lease_b);
    }

    #[tokio::test]
    async fn shutdown_drains_and_kills_all_pools() {
        let sup = PluginSupervisor::new(2).expect("capacity 2 is valid");
        let k1 = key("/bin/p", &["one"]);
        let k2 = key("/bin/p", &["two"]); // different scope ⇒ different pool

        // Acquire then drop so each pool has one warm idle connection.
        for k in [&k1, &k2] {
            let lease = sup
                .acquire(k, || {
                    Ok::<_, std::convert::Infallible>(ProcessSandbox::new(
                        PathBuf::from("/bin/p"),
                        Duration::from_secs(1),
                    ))
                })
                .await
                .expect("spawn");
            drop(lease); // healthy, un-poisoned ⇒ returned to that pool's idle set
        }

        // Shutdown must reclaim both warm connections across both keys.
        let drained = sup.shutdown();
        assert_eq!(
            drained, 2,
            "shutdown must drain every pool's warm connection (one per key)"
        );

        // A second shutdown finds nothing left — idempotent and empty.
        assert_eq!(
            sup.shutdown(),
            0,
            "a second drain finds the pools already empty"
        );

        // Post-shutdown acquire still works (pools re-spawn; no reattach).
        let _fresh = sup
            .acquire(&k1, || {
                Ok::<_, std::convert::Infallible>(ProcessSandbox::new(
                    PathBuf::from("/bin/p"),
                    Duration::from_secs(1),
                ))
            })
            .await
            .expect("acquire after shutdown re-spawns");
    }
}
