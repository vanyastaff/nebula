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
//!   failed reset **poisons** the runtime so the next acquire fails closed
//!   instead of being served the half-reset instance (S4).
//!
//! Unlike the three runtimes it replaces, the release **error is observed**
//! (a `tracing::warn!` plus the release-error metric), never
//! `let _ =`-swallowed — this is the R17 fix.
//!
//! The keepalive task is **not** spawned from the synchronous constructor
//! (that would `panic!` outside a Tokio runtime — a library panic). It is
//! lazily started on the first `acquire`, where an async runtime is
//! guaranteed, and is still aborted on `Drop`.

use std::sync::{
    Arc, Mutex, Once,
    atomic::{AtomicBool, Ordering},
};

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

// ─── Static error messages ───────────────────────────────────────────────────

/// The concurrency semaphore was closed (runtime is shutting down).
const ERR_SEMAPHORE_CLOSED: &str = "bounded: semaphore closed";

/// Timed out waiting for a concurrency permit.
const ERR_PERMIT_TIMEOUT: &str = "bounded: timed out waiting for a permit";

/// Exclusive runtime was poisoned by a prior failed reset (S4).
const ERR_POISONED: &str = "bounded: exclusive runtime poisoned by a prior failed reset \
     (S4) — re-register the resource to obtain a fresh instance";

// ─────────────────────────────────────────────────────────────────────────────

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
    /// S4 fail-closed latch. Set **only** for the
    /// [`Exclusive`](crate::topology::bounded::Exclusive) cap
    /// (`Cap::RESET_ON_RELEASE`) when a `release_one` *reset* fails: the
    /// single shared instance is then in an unknown half-reset state, so
    /// every subsequent `acquire` returns an error rather than handing the
    /// poisoned instance onward. Never set for `Unbounded` / `Capped<N>`
    /// (their `release_one` does not reset the shared runtime), so those
    /// caps are wholly unaffected. Shared with the release closure via an
    /// `Arc` so the release path can latch it after the failed reset.
    poisoned: Arc<AtomicBool>,
    /// Background keepalive driver. **Lazily** started on the first
    /// `acquire` (a `tokio::spawn` from the synchronous constructor would
    /// `panic!` when there is no ambient runtime — a library panic).
    /// Populated only when `config.keepalive_interval` is `Some` and the
    /// resource overrides
    /// [`BoundedRelease::keepalive`](crate::topology::bounded::BoundedRelease::keepalive);
    /// aborted on `Drop`. The [`Once`] guards a single spawn even under
    /// concurrent first acquires.
    keepalive_started: Once,
    keepalive: Mutex<Option<JoinHandle<()>>>,
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
        // The lazily-spawned keepalive task (if it was ever started) is
        // aborted here so it cannot outlive the runtime. `get_mut` cannot
        // be poisoned-by-panic-relevant here: we have `&mut self`, so no
        // other thread holds the lock.
        if let Ok(slot) = self.keepalive.get_mut()
            && let Some(handle) = slot.take()
        {
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
    /// background task drives [`BoundedRelease::keepalive`] on that
    /// interval (folds the previously-unwired `Transport::keepalive`).
    ///
    /// This constructor is **synchronous and does not spawn**: the
    /// keepalive task is started lazily on the first [`acquire`](Self::acquire)
    /// (where an ambient Tokio runtime is guaranteed). Spawning here would
    /// `panic!` for a caller that builds the runtime with
    /// `keepalive_interval: Some(_)` outside a Tokio runtime — a library
    /// panic this fold must not introduce.
    ///
    /// # `Capped<0>` is rejected by construction
    ///
    /// A [`Capped<0>`](crate::topology::bounded::Capped) cap would size the
    /// semaphore at zero permits — every acquire would stall until
    /// `acquire_timeout` and fail, a permanently unavailable resource. An
    /// inline `const { assert!(..) }` here is const-evaluated per
    /// monomorphization, so building a `BoundedRuntime` whose
    /// [`Cap`](crate::topology::bounded::Bounded::Cap) is `Capped<0>` is a
    /// **compile error**, not a runtime dead end. `Capped<1>`+, `Unbounded`
    /// and `Exclusive` are unaffected.
    ///
    /// ```compile_fail
    /// use nebula_core::{ResourceKey, resource_key};
    /// use nebula_resource::{
    ///     BoundedConfig, BoundedRuntime, Resource, ResourceConfig,
    ///     ResourceContext, error::Error, resource::ResourceMetadata,
    ///     topology::bounded::{Bounded, BoundedRelease, Capped},
    /// };
    ///
    /// #[derive(Clone)]
    /// struct Cfg;
    /// nebula_resource::impl_empty_has_schema!(Cfg);
    /// impl ResourceConfig for Cfg {
    ///     fn validate(&self) -> Result<(), Error> { Ok(()) }
    /// }
    ///
    /// #[derive(Debug)]
    /// struct E(String);
    /// impl std::fmt::Display for E {
    ///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    ///         f.write_str(&self.0)
    ///     }
    /// }
    /// impl std::error::Error for E {}
    /// impl From<E> for Error {
    ///     fn from(e: E) -> Self { Error::transient(e.0) }
    /// }
    ///
    /// #[derive(Clone)]
    /// struct CappedZero;
    ///
    /// impl Resource for CappedZero {
    ///     type Config = Cfg;
    ///     type Runtime = u32;
    ///     type Lease = u32;
    ///     type Error = E;
    ///     fn key() -> ResourceKey { resource_key!("doc.capped_zero") }
    ///     async fn create(&self, _c: &Cfg, _x: &ResourceContext) -> Result<u32, E> {
    ///         Ok(1)
    ///     }
    ///     fn metadata() -> ResourceMetadata {
    ///         ResourceMetadata::from_key(&Self::key())
    ///     }
    /// }
    ///
    /// impl Bounded for CappedZero {
    ///     type Cap = Capped<0>;
    ///     async fn acquire_one(&self, runtime: &u32, _x: &ResourceContext) -> Result<u32, E> {
    ///         Ok(*runtime)
    ///     }
    /// }
    ///
    /// impl BoundedRelease for CappedZero {
    ///     async fn release_one(&self, _r: &u32, _l: u32, _h: bool) -> Result<(), E> {
    ///         Ok(())
    ///     }
    /// }
    ///
    /// // `Capped<0>` makes this `new` call a hard compile error: the inline
    /// // `const` assert in the monomorphized constructor body fires.
    /// let r = CappedZero;
    /// let _rt: BoundedRuntime<CappedZero> =
    ///     BoundedRuntime::new(&r, 1u32, BoundedConfig::default());
    /// ```
    pub fn new(resource: &R, runtime: R::Runtime, config: Config) -> Self {
        // Reject a zero-permit cap (`Capped<0>`) by construction: a
        // `Semaphore(0)` never grants a permit, so every acquire would stall
        // until `acquire_timeout` and fail — a permanently unavailable
        // resource. This inline `const` block is const-evaluated per
        // monomorphization of `new`, so `BoundedRuntime::<R>::new` with
        // `R::Cap = Capped<0>` is a hard compile error rather than a runtime
        // dead end. `Unbounded` (`None`), `Exclusive` (`Some(1)`) and
        // `Capped<N>` (N >= 1) const-evaluate this with no panic.
        const {
            assert!(
                !matches!(R::Cap::PERMITS, Some(0)),
                "bounded cap must have a non-zero permit count (Capped<0> is invalid)"
            );
        };
        let _ = resource;
        let runtime = Arc::new(runtime);
        let semaphore = R::Cap::PERMITS.map(|n| Arc::new(Semaphore::new(n)));
        Self {
            runtime,
            semaphore,
            config,
            poisoned: Arc::new(AtomicBool::new(false)),
            keepalive_started: Once::new(),
            keepalive: Mutex::new(None),
        }
    }

    /// Lazily starts the background keepalive task on the first call.
    ///
    /// Called from [`acquire`](Self::acquire), so `tokio::spawn` always
    /// runs inside an ambient runtime (no library panic, unlike spawning
    /// from the synchronous [`new`](Self::new)). The [`Once`] guarantees a
    /// single spawn even under concurrent first acquires; a no-keepalive
    /// config makes this a cheap one-time no-op. The spawned task is parked
    /// in `self.keepalive` so [`Drop`] still aborts it (no task leak).
    fn ensure_keepalive(&self, resource: &R) {
        let Some(interval) = self.config.keepalive_interval else {
            return;
        };
        self.keepalive_started.call_once(|| {
            let resource = resource.clone();
            let runtime = Arc::clone(&self.runtime);
            let handle = tokio::spawn(async move {
                let mut ticker = time::interval(interval);
                ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
                // First tick fires immediately; skip it so the first
                // keepalive is one interval after the first acquire.
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
            });
            // Lock contention here is impossible: `call_once` serializes
            // the single initializer and `Drop` needs `&mut self`.
            if let Ok(mut slot) = self.keepalive.lock() {
                *slot = Some(handle);
            }
        });
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
    /// - `permanent` if the semaphore is closed, **or** (Exclusive cap
    ///   only) the runtime was poisoned by a prior failed reset (S4
    ///   fail-closed — the half-reset instance is never handed onward).
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
        // Lazily start the keepalive task now that an ambient Tokio
        // runtime is guaranteed (it must NOT be spawned from the
        // synchronous `new`; that panics with no runtime). The `Once`
        // makes every call after the first a cheap no-op.
        self.ensure_keepalive(resource);

        // 1. Gate on the cap's concurrency permit, if any.
        let permit = match &self.semaphore {
            None => None,
            Some(sem) => {
                let timeout = options.remaining().unwrap_or(self.config.acquire_timeout);
                match time::timeout(timeout, sem.clone().acquire_owned()).await {
                    Ok(Ok(p)) => Some(p),
                    Ok(Err(_)) => return Err(Error::permanent(ERR_SEMAPHORE_CLOSED)),
                    Err(_) => {
                        return Err(Error::backpressure(ERR_PERMIT_TIMEOUT));
                    },
                }
            },
        };

        // 1b. S4 fail-closed (Exclusive cap only). A prior `release_one`
        //     reset failed, leaving the single shared instance in an
        //     unknown half-reset state. `acquire_one` for the Exclusive
        //     cap hands out that very instance (the lease *is* a clone of
        //     the shared, interior-mutable runtime), so serving it would
        //     propagate corrupted session state to the next caller. Refuse
        //     instead — the permit acquired above is dropped on this early
        //     return, so the semaphore is not wedged. Gated on
        //     `RESET_ON_RELEASE` (true *only* for Exclusive): `Unbounded` /
        //     `Capped<N>` never set the latch and never read it here, so
        //     they are wholly unaffected. There is no rebuild primitive on
        //     this path (no `Config` / `ResourceContext` for `create`), so
        //     fail-closed — not silent recycling — is the S4 guarantee.
        if R::Cap::RESET_ON_RELEASE && self.poisoned.load(Ordering::Acquire) {
            if let Some(m) = &metrics {
                m.record_acquire_error();
            }
            return Err(Error::permanent(ERR_POISONED));
        }

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
        //     Exclusive cap a failed reset additionally poisons the
        //     runtime (S4) so the next acquire fails closed instead of
        //     being handed the half-reset instance.
        let runtime = Arc::clone(&self.runtime);
        let resource_clone = resource.clone();
        let rq = Arc::clone(release_queue);
        let poisoned = Arc::clone(&self.poisoned);

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
                        poisoned,
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
/// additionally **poisons the runtime** (S4): the `poisoned` latch is set
/// so every subsequent [`BoundedRuntime::acquire`] fails closed rather than
/// minting a lease over the half-reset instance. For the Exclusive cap the
/// lease *is* a clone of the shared, interior-mutable runtime and
/// `release_one` is a reset that mutates it in place, so a `(*runtime).clone()`
/// destroy alone does **not** isolate the next caller — the live shared
/// instance is what `acquire_one` would hand out next. Poisoning is what
/// enforces the S4 isolation guarantee; the clone is still `destroy`d
/// (best-effort, observed) to release whatever OS / connection resources
/// that instance holds, but it is the latch — not the destroy — that
/// protects the next acquirer. There is no in-place rebuild on this path
/// (it has no `Config` / `ResourceContext` for `create`), so fail-closed,
/// not silent recycling, is the contract; recovery is re-registration.
///
/// The latch is **only ever set for the Exclusive cap** (`RESET_ON_RELEASE`,
/// `true` *only* there). `Capped<N>` — including `Capped<1>` — is a shared
/// multiplexer: `release_one` returns a token / closes one session rather
/// than resetting the shared runtime, the next acquirer needs that runtime,
/// and it is neither poisoned nor destroyed (the failed release is still
/// observed and the permit still returned). `Unbounded` has no release
/// hook.
///
/// The semaphore `permit` (if any) is the last value dropped, **after**
/// `release_one` (and any destroy) completes — this is the
/// permit-held-until-release ordering (#384). It is always returned, even
/// on a failed reset, so a reset error cannot deadlock the semaphore (S4
/// preserve): the next acquire reaches the poison check and fails closed
/// with an error, it does not block forever on the permit.
async fn release_one_observed<R>(
    resource: R,
    runtime: Arc<R::Runtime>,
    lease: R::Lease,
    healthy: bool,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
    poisoned: Arc<AtomicBool>,
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

        // S4: a failed *reset* for the Exclusive cap means the single
        // underlying instance is in an unknown half-reset state. Latch the
        // runtime poisoned so the *next* `acquire` fails closed instead of
        // minting a lease over it — destroying a throwaway `(*runtime).clone()`
        // would not have isolated the next caller, because for the
        // Exclusive cap the lease is itself a clone of the shared
        // interior-mutable runtime that `acquire_one` hands out next.
        // Gated on `RESET_ON_RELEASE` (`true` *only* for `Exclusive`):
        // `Capped<N>` (incl. `Capped<1>`) is a shared multiplexer the next
        // acquirer still needs — never poisoned or destroyed. The
        // failed-release error is still observed above and the permit is
        // still returned below for every cap.
        if R::Cap::RESET_ON_RELEASE {
            poisoned.store(true, Ordering::Release);
            // Best-effort teardown of the matching owned instance so its
            // OS / connection resources are released promptly (the live
            // shared runtime is now unreachable behind the poison latch).
            // `destroy` consumes an owned `R::Runtime`; the lease was
            // minted by cloning the runtime, so a clone is the matching
            // instance. Its outcome is observed, never swallowed.
            let instance = (*runtime).clone();
            if let Err(de) = resource.destroy(instance).await {
                tracing::warn!(
                    resource = %R::key(),
                    error = %Into::<Error>::into(de),
                    "bounded: destroy after failed reset also failed \
                     (runtime already poisoned; acquires fail closed)"
                );
                if let Some(m) = &metrics {
                    m.record_release_error();
                }
            }
        }
    }
    // `permit` drops here — after release_one (and any S4 destroy) has
    // resolved. The next acquirer cannot enter mid-reset (#384); for the
    // Exclusive cap it then hits the poison check and fails closed.
    drop(permit);
}
