//! Bounded runtime — one runtime, capped short-lived leases.
//!
//! Folds `ServiceRuntime`, `TransportRuntime`, and `ExclusiveRuntime` into
//! one runtime parameterized by the [`Cap`](crate::topology::bounded::Bounded::Cap)
//! typestate:
//!
//! - **`Unbounded`** (Service `Cloned`): no semaphore, owned handle, no
//!   release callback.
//! - **`Capped<N>`** (Service `Tracked`, Transport): `Semaphore(N)`,
//!   guarded handle, `release_one` returns the token / closes the session.
//! - **`Exclusive`**: `Semaphore(1)`, guarded handle, `release_one` is the
//!   reset; the permit is held until the reset resolves (#384) and a
//!   failed reset destroys the instance instead of recycling it (S4).
//!
//! Unlike the three runtimes it replaces, the release **error is observed**
//! (a `tracing::warn!` plus the release-error metric), never
//! `let _ =`-swallowed — this is the R17 fix.

use std::sync::Arc;

use tokio::{sync::Semaphore, task::JoinHandle, time};

use crate::{
    context::ResourceContext,
    error::Error,
    guard::ResourceGuard,
    metrics::ResourceOpsMetrics,
    options::AcquireOptions,
    release_queue::ReleaseQueue,
    resource::Resource,
    topology::bounded::{BoundedRelease, CapMarker, config::Config},
    topology_tag::TopologyTag,
};

/// Runtime state for a [`Bounded`](crate::topology::bounded::Bounded)
/// resource.
///
/// Holds the long-lived runtime, an optional concurrency semaphore sized
/// by the cap typestate, and (when configured) a background keepalive
/// task. The cap (`R::Cap`) decides every release-shape branch at compile
/// time — there is no runtime topology discriminant.
pub struct BoundedRuntime<R: Resource> {
    runtime: Arc<R::Runtime>,
    /// `None` for the [`Unbounded`](crate::topology::bounded::Unbounded)
    /// cap (acquire is never gated); `Some(Semaphore(n))` otherwise, where
    /// `n` is `Capped<N>`'s `N` or `1` for `Exclusive`.
    semaphore: Option<Arc<Semaphore>>,
    config: Config,
    /// Background keepalive driver, aborted on drop. Present only when
    /// `config.keepalive_interval` is `Some` and the resource overrides
    /// [`BoundedRelease::keepalive`](crate::topology::bounded::BoundedRelease::keepalive).
    keepalive: Option<JoinHandle<()>>,
}

impl<R: Resource> BoundedRuntime<R> {
    /// Returns the current configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Returns a reference to the underlying long-lived runtime.
    pub fn runtime(&self) -> &R::Runtime {
        &self.runtime
    }
}

impl<R: Resource> Drop for BoundedRuntime<R> {
    fn drop(&mut self) {
        if let Some(handle) = self.keepalive.take() {
            handle.abort();
        }
    }
}

impl<R> BoundedRuntime<R>
where
    R: BoundedRelease + Clone + Send + Sync + 'static,
    R::Lease: Send + 'static,
    R::Runtime: Clone + Send + Sync + 'static,
{
    /// Creates a new bounded runtime wrapping an existing runtime instance.
    ///
    /// The semaphore is sized from the cap typestate
    /// ([`CapMarker::PERMITS`]): `None` → unbounded, `Some(n)` →
    /// `Semaphore(n)`. When `config.keepalive_interval` is `Some`, a
    /// background task drives [`BoundedRelease::keepalive`] on that interval
    /// (folds the previously-unwired `Transport::keepalive`).
    pub fn new(resource: &R, runtime: R::Runtime, config: Config) -> Self {
        let runtime = Arc::new(runtime);
        let semaphore = R::Cap::PERMITS.map(|n| Arc::new(Semaphore::new(n)));
        let keepalive = config.keepalive_interval.map(|interval| {
            let resource = resource.clone();
            let runtime = Arc::clone(&runtime);
            tokio::spawn(async move {
                let mut ticker = time::interval(interval);
                ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
                // First tick fires immediately; skip it so the first
                // keepalive is one interval after construction.
                ticker.tick().await;
                loop {
                    ticker.tick().await;
                    if let Err(e) = resource.keepalive(&runtime).await {
                        tracing::warn!(
                            resource = %R::key(),
                            error = %Into::<Error>::into(e),
                            "bounded: keepalive failed"
                        );
                    }
                }
            })
        });
        Self {
            runtime,
            semaphore,
            config,
            keepalive,
        }
    }

    /// Acquires one lease from the bounded runtime.
    ///
    /// 1. If the cap has a permit count, acquires a permit (bounded by
    ///    `config.acquire_timeout` or the caller's remaining deadline).
    /// 2. Calls [`Bounded::acquire_one`](crate::topology::bounded::Bounded::acquire_one).
    /// 3. If the cap does not require release
    ///    ([`Unbounded`](crate::topology::bounded::Unbounded)) — returns
    ///    an owned handle (no callback). Otherwise returns a guarded
    ///    handle whose drop submits the release to the [`ReleaseQueue`].
    ///
    /// The semaphore permit (when present) is moved into the release
    /// closure and then into the submitted release future, so it is
    /// dropped **after** `release_one` resolves — preserving the
    /// permit-held-until-release ordering (#384) the `Exclusive` cap
    /// relies on.
    ///
    /// # Errors
    ///
    /// - `permanent` if the semaphore is closed.
    /// - `backpressure` if the permit acquire times out.
    /// - Propagates `acquire_one` errors.
    pub async fn acquire(
        &self,
        resource: &R,
        ctx: &ResourceContext,
        release_queue: &Arc<ReleaseQueue>,
        generation: u64,
        options: &AcquireOptions,
        metrics: Option<ResourceOpsMetrics>,
    ) -> Result<ResourceGuard<R>, Error> {
        // 1. Gate on the cap's concurrency permit, if any.
        let permit = match &self.semaphore {
            None => None,
            Some(sem) => {
                let timeout = options.remaining().unwrap_or(self.config.acquire_timeout);
                match time::timeout(timeout, sem.clone().acquire_owned()).await {
                    Ok(Ok(p)) => Some(p),
                    Ok(Err(_)) => return Err(Error::permanent("bounded: semaphore closed")),
                    Err(_) => {
                        return Err(Error::backpressure(
                            "bounded: timed out waiting for a permit",
                        ));
                    },
                }
            },
        };

        // 2. Mint the lease.
        let lease = resource
            .acquire_one(&self.runtime, ctx)
            .await
            .map_err(Into::into)?;

        // 3a. Unbounded / Cloned: owned handle, no release callback. The
        //     cap typestate guarantees `release_one` is not part of this
        //     resource's contract, so there is nothing to run on drop.
        if !R::Cap::RELEASE_REQUIRED {
            return Ok(ResourceGuard::owned(lease, R::key(), TopologyTag::Bounded));
        }

        // 3b. Capped / Exclusive: guarded handle. The permit lives in the
        //     release closure → submitted future so it outlives
        //     `release_one` (permit-held-until-release, #384). For the
        //     Exclusive cap a failed reset additionally destroys the
        //     instance (S4) instead of handing it onward.
        let runtime = Arc::clone(&self.runtime);
        let resource_clone = resource.clone();
        let rq = Arc::clone(release_queue);

        Ok(ResourceGuard::guarded(
            lease,
            R::key(),
            TopologyTag::Bounded,
            generation,
            move |returned_lease, tainted| {
                if let Some(m) = &metrics {
                    m.record_release();
                }
                rq.submit(move || {
                    Box::pin(release_one_observed(
                        resource_clone,
                        runtime,
                        returned_lease,
                        !tainted,
                        permit,
                        metrics,
                    ))
                });
            },
        ))
    }
}

/// Runs `release_one` and **observes** its outcome (R17).
///
/// On `Err`:
/// - emits a `tracing::warn!` carrying the resource key and error, and
/// - increments the release-error metric
///   ([`ResourceOpsMetrics::record_release_error`]).
///
/// For the [`Exclusive`](crate::topology::bounded::Exclusive) cap an `Err`
/// additionally `destroy`s a runtime instance (S4): the half-reset state
/// is torn down rather than handed to the next caller. The shared runtime
/// `Arc` itself is never mutated by a lease, so destroying a clone is
/// sufficient — the next acquirer clones a fresh lease from the pristine
/// runtime, never the failed-reset one.
///
/// The semaphore `permit` (if any) is the last value dropped, **after**
/// `release_one` (and any destroy) completes — this is the
/// permit-held-until-release ordering (#384). It is always returned, even
/// on a failed reset, so a reset error cannot deadlock the semaphore (S4
/// preserve).
async fn release_one_observed<R>(
    resource: R,
    runtime: Arc<R::Runtime>,
    lease: R::Lease,
    healthy: bool,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
    metrics: Option<ResourceOpsMetrics>,
) where
    R: BoundedRelease + Send + Sync + 'static,
    R::Runtime: Clone + Send + Sync + 'static,
    R::Lease: Send + 'static,
{
    if let Err(e) = resource.release_one(&runtime, lease, healthy).await {
        tracing::warn!(
            resource = %R::key(),
            error = %Into::<Error>::into(e),
            "bounded: release hook failed (observed, not swallowed)"
        );
        if let Some(m) = &metrics {
            m.record_release_error();
        }

        // S4: a failed release for the Exclusive cap means the runtime is
        // in an unknown half-reset state — destroy it rather than letting
        // the next acquirer be served a poisoned instance. `destroy`
        // consumes an owned `R::Runtime`; the lease was minted by cloning
        // the shared runtime, so a clone is the matching instance to tear
        // down (the shared `Arc` stays pristine for the next acquire).
        if R::Cap::PERMITS == Some(1) && R::Cap::RELEASE_REQUIRED {
            let instance = (*runtime).clone();
            if let Err(de) = resource.destroy(instance).await {
                tracing::warn!(
                    resource = %R::key(),
                    error = %Into::<Error>::into(de),
                    "bounded: destroy after failed reset also failed"
                );
                if let Some(m) = &metrics {
                    m.record_release_error();
                }
            }
        }
    }
    // `permit` drops here — after release_one (and any S4 destroy) has
    // resolved. The next acquirer cannot enter mid-reset (#384).
    drop(permit);
}
