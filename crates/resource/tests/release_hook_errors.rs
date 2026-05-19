//! Release-hook error handling — characterization of the previously-swallowed
//! `reset` / `close_session` / `release_token` failures, now fixed on the
//! unified `BoundedRuntime` path (R17 / S4).
//!
//! Before the fold every per-topology release helper discarded the hook
//! result with `let _ = resource.<hook>(...).await;`. A failed `reset`
//! therefore silently handed the lock to the next caller as if the previous
//! lease was cleanly reset — a half-reset instance could be served to the
//! next acquirer, and a failed token-return / session-close vanished
//! without a trace.
//!
//! The contract the unified `BoundedRuntime` release path enforces (S4):
//! - On `reset` `Err`, the permit IS still returned (withholding it would
//!   deadlock the semaphore — it lives in the handle, outside the callback,
//!   for exactly this reason) BUT the runtime is **poisoned**: every
//!   subsequent `acquire` fails closed instead of being handed the
//!   half-reset instance. (Destroying a throwaway `(*runtime).clone()`
//!   would not isolate the next caller — for the Exclusive cap the lease
//!   *is* a clone of the shared interior-mutable runtime that `acquire_one`
//!   hands out next; the poison latch is what enforces isolation. The
//!   clone is still `destroy`d best-effort to release its resources.)
//! - Every `release_one` `Err` (token return / session close / reset) is
//!   **observed**: a `tracing::warn!` plus a bump of the release-error
//!   metric (`ResourceOpsMetrics::record_release_error`), never
//!   `let _ =`-swallowed.
//!
//! What is asserted here:
//! - **R17 (GREEN on the `Bounded` path):** a failed `reset` poisons the
//!   runtime so the next acquire fails closed (and best-effort destroys
//!   the matching instance) instead of silently handing the half-reset
//!   instance onward. The fix lives on `BoundedRuntime` with
//!   `Cap = Exclusive`, so the probe is expressed against a
//!   directly-constructed `BoundedRuntime`.
//! - **GREEN preserve:** a failed `reset` must still return the permit so
//!   the next acquire does not deadlock — including with an explicitly
//!   configured `acquire_timeout`. S4 explicitly preserves this; it does
//!   not regress when the destroy-on-failure behavior is added.
//! - **Observed, not swallowed:** a failing `release_one` for a
//!   release-bearing cap increments the release-error metric (the
//!   `release_token` / `close_session` analogue).

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_core::{ExecutionId, scope::Scope};
use nebula_core::{ResourceKey, resource_key};
use nebula_metrics::MetricsRegistry;
use nebula_resource::{
    AcquireOptions, BoundedConfig, BoundedRuntime, Resource, ResourceConfig, ResourceContext,
    error::{Error, ErrorKind},
    metrics::ResourceOpsMetrics,
    release_queue::ReleaseQueue,
    resource::ResourceMetadata,
    topology::bounded::{Bounded, BoundedRelease, Capped, Exclusive as ExclusiveCap},
};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct R17Error(String);

impl std::fmt::Display for R17Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for R17Error {}

impl From<R17Error> for Error {
    fn from(e: R17Error) -> Self {
        Error::transient(e.0)
    }
}

#[derive(Clone)]
struct R17Config;

nebula_resource::impl_empty_has_schema!(R17Config);

impl ResourceConfig for R17Config {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
}

fn ctx() -> ResourceContext {
    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        ..Default::default()
    };
    ResourceContext::minimal(scope, CancellationToken::new())
}

/// Polls `cond` until it returns `true` or the deadline elapses.
async fn poll_until(deadline: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        if cond() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    cond()
}

// ---------------------------------------------------------------------------
// BoundedRuntime, Cap = Exclusive — R17 fix + S4 preserve net
// ---------------------------------------------------------------------------

/// S4 preserve on the `Bounded` `Exclusive`-cap path with an **explicit
/// non-default `acquire_timeout`** (the config facet the folded exclusive
/// topology carried): a failed `reset` must still return the permit so the
/// next acquire is not wedged on the semaphore. With the poison latch the
/// next acquire fails *closed* (a `permanent` error) rather than being
/// served the half-reset instance — but it must fail **promptly**, not by
/// blocking on a permit that was never returned. Distinct from
/// `bounded_exclusive_failed_reset_poisons_runtime_next_acquire_fails_closed`
/// (default config) — this pins the permit-return invariant (prompt
/// `Permanent`, observed well within the deadline) when the Exclusive
/// cap's `acquire_timeout` is explicitly configured. The permit lives in
/// the handle, outside the release callback, exactly so a failed reset
/// cannot wedge the semaphore.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounded_exclusive_failed_reset_returns_permit_with_explicit_acquire_timeout() {
    let resource = BoundedFailingReset::new();
    let config = nebula_resource::topology::bounded::config::Config {
        acquire_timeout: Duration::from_secs(5),
        ..Default::default()
    };
    let runtime: Arc<BoundedRuntime<BoundedFailingReset>> =
        Arc::new(BoundedRuntime::new(&resource, 1u32, config));
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let h1 = runtime
        .acquire(&resource, &ctx(), &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("first acquire must succeed");
    drop(h1);

    // The failed reset must have run and poisoned the runtime before the
    // next acquire is evaluated.
    let poisoned = poll_until(Duration::from_secs(2), || {
        resource.release_attempts.load(Ordering::SeqCst) >= 1
    })
    .await;
    assert!(poisoned, "the failing reset must have run");

    let started = std::time::Instant::now();
    let h2 = runtime
        .acquire(
            &resource,
            &ctx(),
            &rq,
            0,
            &AcquireOptions::default()
                .with_deadline(std::time::Instant::now() + Duration::from_secs(2)),
            None,
        )
        .await;
    let err = h2.expect_err(
        "S4: after a failed reset the Exclusive runtime is poisoned, so the \
         next acquire must fail closed (not be served the half-reset \
         instance)",
    );
    assert_eq!(
        *err.kind(),
        ErrorKind::Permanent,
        "a poisoned-runtime acquire must be a prompt `Permanent` error — \
         NOT a backpressure/timeout, which would mean the permit was not \
         returned and the semaphore is wedged (S4 preserve, explicit \
         acquire_timeout): {err:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the fail-closed rejection must be prompt (permit was returned, no \
         deadlock); took {:?}",
        started.elapsed()
    );

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// BoundedRuntime, Cap = Exclusive — R17 fix (formerly the ignored RED probe)
// ---------------------------------------------------------------------------

/// `Bounded` view of the failing-reset exclusive resource: `release_one`
/// IS the reset and always fails; `destroy` and `release_one` attempts are
/// counted so the test can prove S4 (failed reset → instance destroyed,
/// permit still returned).
#[derive(Clone)]
struct BoundedFailingReset {
    release_attempts: Arc<AtomicUsize>,
    destroy_calls: Arc<AtomicUsize>,
}

impl BoundedFailingReset {
    fn new() -> Self {
        Self {
            release_attempts: Arc::new(AtomicUsize::new(0)),
            destroy_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Resource for BoundedFailingReset {
    type Config = R17Config;
    type Runtime = u32;
    type Lease = u32;
    type Error = R17Error;

    fn key() -> ResourceKey {
        resource_key!("r17-bounded-failing-reset")
    }

    async fn create(&self, _config: &R17Config, _ctx: &ResourceContext) -> Result<u32, R17Error> {
        Ok(1)
    }

    async fn destroy(&self, _runtime: u32) -> Result<(), R17Error> {
        self.destroy_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Bounded for BoundedFailingReset {
    type Cap = ExclusiveCap;

    async fn acquire_one(&self, runtime: &u32, _ctx: &ResourceContext) -> Result<u32, R17Error> {
        Ok(*runtime)
    }
}

impl BoundedRelease for BoundedFailingReset {
    async fn release_one(
        &self,
        _runtime: &u32,
        _lease: u32,
        _healthy: bool,
    ) -> Result<(), R17Error> {
        self.release_attempts.fetch_add(1, Ordering::SeqCst);
        Err(R17Error("reset failed".to_owned()))
    }
}

/// R17 — GREEN on the `Bounded` path. A failed `release_one` (the reset for
/// the `Exclusive` cap) must NOT be silently treated as a successful
/// release: the half-reset instance is destroyed rather than handed to the
/// next acquirer. The unified release path calls `destroy` on a reset
/// failure, so `destroy_calls` reaches `1`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounded_exclusive_failed_reset_destroys_instance_not_handed_onward() {
    let resource = BoundedFailingReset::new();
    let runtime: BoundedRuntime<BoundedFailingReset> =
        BoundedRuntime::new(&resource, 1u32, BoundedConfig::default());
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let handle = runtime
        .acquire(&resource, &ctx(), &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("first acquire must succeed");

    // Drop the lease → `release_one` (reset) runs on the release queue and
    // fails.
    drop(handle);

    let reset_ran = poll_until(Duration::from_secs(2), || {
        resource.release_attempts.load(Ordering::SeqCst) >= 1
    })
    .await;
    assert!(
        reset_ran,
        "the reset hook must have been invoked on release"
    );

    let destroyed = poll_until(Duration::from_secs(2), || {
        resource.destroy_calls.load(Ordering::SeqCst) >= 1
    })
    .await;
    assert!(
        destroyed,
        "a failed `reset` must destroy the instance (S4: the half-reset \
         instance is never recycled or handed onward); observed \
         destroy_calls={}",
        resource.destroy_calls.load(Ordering::SeqCst)
    );

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

/// An `Arc`-backed-runtime Exclusive resource whose `release_one` (the
/// reset) always fails. `Runtime = Arc<AtomicUsize>` deliberately models
/// the reviewer's case: a `Runtime` whose `Clone` is **shallow** (the
/// clone shares the same `AtomicUsize` cell as the live shared runtime).
/// `acquire_one` increments `acquire_one_calls` so the test can prove that
/// after a failed reset the runtime is poisoned **before** `acquire_one`
/// is reached — the half-reset instance is never minted into a lease.
#[derive(Clone)]
struct ArcBackedFailingReset {
    acquire_one_calls: Arc<AtomicUsize>,
    /// Incremented by the best-effort `destroy` the release path runs
    /// **after** latching the poison flag — so `destroy_calls >= 1` is a
    /// deterministic "the failed reset ran and the runtime is now
    /// poisoned" signal the test can poll on (no sleeps, no nested
    /// blocking).
    destroy_calls: Arc<AtomicUsize>,
}

impl Resource for ArcBackedFailingReset {
    type Config = R17Config;
    type Runtime = Arc<AtomicUsize>;
    type Lease = Arc<AtomicUsize>;
    type Error = R17Error;

    fn key() -> ResourceKey {
        resource_key!("r17-arc-backed-failing-reset")
    }

    async fn create(
        &self,
        _config: &R17Config,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicUsize>, R17Error> {
        Ok(Arc::new(AtomicUsize::new(0)))
    }

    async fn destroy(&self, _runtime: Arc<AtomicUsize>) -> Result<(), R17Error> {
        self.destroy_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Bounded for ArcBackedFailingReset {
    type Cap = ExclusiveCap;

    async fn acquire_one(
        &self,
        runtime: &Arc<AtomicUsize>,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicUsize>, R17Error> {
        self.acquire_one_calls.fetch_add(1, Ordering::SeqCst);
        // A shallow clone — shares the same cell as the shared runtime,
        // exactly the `Arc`-backed shape the reviewer flagged.
        Ok(Arc::clone(runtime))
    }
}

impl BoundedRelease for ArcBackedFailingReset {
    async fn release_one(
        &self,
        _runtime: &Arc<AtomicUsize>,
        _lease: Arc<AtomicUsize>,
        _healthy: bool,
    ) -> Result<(), R17Error> {
        Err(R17Error("reset failed".to_owned()))
    }
}

/// S4 isolation guarantee (the Thread-1 fix). After a failed `reset` the
/// Exclusive runtime is **poisoned**: the next `acquire` fails *closed*
/// rather than being handed the half-reset, shallow-`Arc`-clone instance.
/// Two things are asserted:
///
/// 1. **Isolation:** the post-failed-reset acquire returns an `Err`
///    (`Permanent`), and `acquire_one` is **never reached** for it — the
///    poisoned instance is never minted into a lease (`acquire_one_calls`
///    stays at `1`, the one successful first acquire).
/// 2. **S4 preserve (no deadlock):** the rejection is *prompt*, not a
///    timeout — proving the permit was still returned (it lives in the
///    handle, outside the release callback) so the semaphore is not
///    wedged. A pre-fix `h2.is_ok()` here was the bug: it encoded the
///    half-reset instance being handed onward.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounded_exclusive_failed_reset_poisons_runtime_next_acquire_fails_closed() {
    let resource = ArcBackedFailingReset {
        acquire_one_calls: Arc::new(AtomicUsize::new(0)),
        destroy_calls: Arc::new(AtomicUsize::new(0)),
    };
    let runtime: Arc<BoundedRuntime<ArcBackedFailingReset>> = Arc::new(BoundedRuntime::new(
        &resource,
        Arc::new(AtomicUsize::new(0)),
        BoundedConfig::default(),
    ));
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let h1 = runtime
        .acquire(&resource, &ctx(), &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("first acquire must succeed");
    assert_eq!(
        resource.acquire_one_calls.load(Ordering::SeqCst),
        1,
        "the first acquire mints exactly one lease"
    );
    drop(h1);

    // Deterministic poison signal: the release path latches `poisoned`
    // *before* the best-effort `destroy`, so `destroy_calls >= 1` proves
    // the failed reset ran and the runtime is now poisoned — no sleeps.
    let poisoned = poll_until(Duration::from_secs(2), || {
        resource.destroy_calls.load(Ordering::SeqCst) >= 1
    })
    .await;
    assert!(
        poisoned,
        "the failing reset must have run and poisoned the runtime \
         (destroy_calls={})",
        resource.destroy_calls.load(Ordering::SeqCst)
    );

    // The next acquire must fail closed — and promptly, proving the permit
    // was returned (it lives in the handle, outside the release callback)
    // so the semaphore is not wedged. A 2s deadline that is NOT consumed
    // is the no-deadlock proof (S4 preserve); a pre-fix `is_ok()` here was
    // the Thread-1 bug (half-reset instance handed onward).
    let started = std::time::Instant::now();
    let h2 = runtime
        .acquire(
            &resource,
            &ctx(),
            &rq,
            0,
            &AcquireOptions::default()
                .with_deadline(std::time::Instant::now() + Duration::from_secs(2)),
            None,
        )
        .await;
    let err = h2.expect_err(
        "S4: after a failed reset the Exclusive runtime is poisoned, so the \
         next acquire must fail closed — it must NOT be served the \
         half-reset (shallow-Arc-clone) instance",
    );
    assert_eq!(
        *err.kind(),
        ErrorKind::Permanent,
        "the poisoned-runtime rejection must be a `Permanent` error, NOT a \
         backpressure/timeout (which would mean the permit was not returned \
         and the semaphore wedged — S4 preserve): {err:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the fail-closed rejection must be prompt (permit returned, no \
         deadlock — well under the 2s deadline); took {:?}",
        started.elapsed()
    );
    assert_eq!(
        resource.acquire_one_calls.load(Ordering::SeqCst),
        1,
        "S4 isolation: `acquire_one` must NOT be reached for the \
         post-failed-reset acquire — the poisoned instance is never minted \
         into a lease (still exactly the 1 first-acquire call)"
    );

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// Observed-not-swallowed — release-error metric (release_token / close_session
// analogue, Cap = Capped)
// ---------------------------------------------------------------------------

/// A capped (Tracked-service / Transport-shaped) resource whose
/// `release_one` always fails. The release-error metric proves the failure
/// is observed rather than `let _ =`-swallowed.
#[derive(Clone)]
struct CappedFailingRelease {
    release_attempts: Arc<AtomicUsize>,
}

impl Resource for CappedFailingRelease {
    type Config = R17Config;
    type Runtime = &'static str;
    type Lease = String;
    type Error = R17Error;

    fn key() -> ResourceKey {
        resource_key!("r17-capped-failing-release")
    }

    async fn create(
        &self,
        _config: &R17Config,
        _ctx: &ResourceContext,
    ) -> Result<&'static str, R17Error> {
        Ok("rt")
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Bounded for CappedFailingRelease {
    type Cap = Capped<4>;

    async fn acquire_one(
        &self,
        runtime: &&'static str,
        _ctx: &ResourceContext,
    ) -> Result<String, R17Error> {
        Ok((*runtime).to_owned())
    }
}

impl BoundedRelease for CappedFailingRelease {
    async fn release_one(
        &self,
        _runtime: &&'static str,
        _lease: String,
        _healthy: bool,
    ) -> Result<(), R17Error> {
        self.release_attempts.fetch_add(1, Ordering::SeqCst);
        Err(R17Error("release_token failed".to_owned()))
    }
}

/// Observed, not swallowed: a failing `release_one` on a `Capped` cap (the
/// Tracked `release_token` / Transport `close_session` analogue) bumps the
/// release-error metric. Pre-fold the equivalent `let _ = …` discarded the
/// error with no metric and no log.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounded_capped_failed_release_is_observed_via_metric() {
    let registry = MetricsRegistry::new();
    let metrics = ResourceOpsMetrics::new(&registry).expect("metrics");
    let resource = CappedFailingRelease {
        release_attempts: Arc::new(AtomicUsize::new(0)),
    };
    let runtime: BoundedRuntime<CappedFailingRelease> =
        BoundedRuntime::new(&resource, "rt", BoundedConfig::default());
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let before = metrics.snapshot().release_errors;

    let handle = runtime
        .acquire(
            &resource,
            &ctx(),
            &rq,
            0,
            &AcquireOptions::default(),
            Some(metrics.clone()),
        )
        .await
        .expect("acquire must succeed");
    drop(handle);

    let observed = poll_until(Duration::from_secs(2), || {
        metrics.snapshot().release_errors > before
    })
    .await;
    assert!(
        observed,
        "a failing release_one must increment the release-error metric \
         (observed, not swallowed); release_errors stayed at {before}"
    );
    assert!(
        resource.release_attempts.load(Ordering::SeqCst) >= 1,
        "release_one must have been invoked"
    );

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ---------------------------------------------------------------------------
// Capped<1> failed release must NOT destroy the shared runtime (S4 is
// Exclusive-only). RED-proof of the destroy-on-failed-release regression
// and its fix: pre-collapse Transport/Service with a single-permit cap did
// NOT destroy on a failed release; the shared multiplexer must survive so
// the next acquirer can still use it. The S4 destroy is gated on
// `Cap::RESET_ON_RELEASE`, which is `true` only for `Exclusive` and `false`
// for every `Capped<N>` including `Capped<1>`.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Cap1Error(String);

impl std::fmt::Display for Cap1Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Cap1Error {}

impl From<Cap1Error> for Error {
    fn from(e: Cap1Error) -> Self {
        Error::transient(e.0)
    }
}

#[derive(Clone)]
struct Cap1Config;

nebula_resource::impl_empty_has_schema!(Cap1Config);

impl ResourceConfig for Cap1Config {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
}

/// A `Capped<1>` resource (a single-session shared multiplexer — e.g. a
/// transport with `max_sessions = 1`) whose `release_one` (the session
/// close / token return) always fails. `destroy` and `release_one` attempts
/// are counted so the test can prove the shared runtime is NOT destroyed on
/// a failed release while the error is still observed and the permit is
/// still returned.
#[derive(Clone)]
struct Cap1FailingRelease {
    release_attempts: Arc<AtomicUsize>,
    destroy_calls: Arc<AtomicUsize>,
}

impl Resource for Cap1FailingRelease {
    type Config = Cap1Config;
    type Runtime = &'static str;
    type Lease = String;
    type Error = Cap1Error;

    fn key() -> ResourceKey {
        resource_key!("cap1-failing-release")
    }

    async fn create(
        &self,
        _config: &Cap1Config,
        _ctx: &ResourceContext,
    ) -> Result<&'static str, Cap1Error> {
        Ok("shared-mux")
    }

    async fn destroy(&self, _runtime: &'static str) -> Result<(), Cap1Error> {
        self.destroy_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Bounded for Cap1FailingRelease {
    type Cap = Capped<1>;

    async fn acquire_one(
        &self,
        runtime: &&'static str,
        _ctx: &ResourceContext,
    ) -> Result<String, Cap1Error> {
        Ok((*runtime).to_owned())
    }
}

impl BoundedRelease for Cap1FailingRelease {
    async fn release_one(
        &self,
        _runtime: &&'static str,
        _lease: String,
        _healthy: bool,
    ) -> Result<(), Cap1Error> {
        self.release_attempts.fetch_add(1, Ordering::SeqCst);
        Err(Cap1Error("close_session failed".to_owned()))
    }
}

/// A failed `release_one` on a `Capped<1>` cap must:
/// 1. be **observed** (release-error metric bumped) — same as any cap,
/// 2. still **return the permit** so the next acquire does not deadlock,
/// 3. **NOT destroy** the shared runtime — `destroy` is never called,
///    because `Capped<1>` is a shared multiplexer (S4 destroy is
///    `Exclusive`-only via `Cap::RESET_ON_RELEASE`). The next acquire must
///    still succeed against the surviving shared runtime.
///
/// This is the RED-proof of the destroy-on-failed-release regression: with
/// the old `PERMITS == Some(1) && RELEASE_REQUIRED` heuristic `destroy`
/// fired here too (a `Capped<1>` matches that predicate); with the
/// `RESET_ON_RELEASE` gate it does not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capped_one_failed_release_observed_permit_returned_runtime_not_destroyed() {
    let registry = MetricsRegistry::new();
    let metrics = ResourceOpsMetrics::new(&registry).expect("metrics");
    let resource = Cap1FailingRelease {
        release_attempts: Arc::new(AtomicUsize::new(0)),
        destroy_calls: Arc::new(AtomicUsize::new(0)),
    };
    let runtime: Arc<BoundedRuntime<Cap1FailingRelease>> = Arc::new(BoundedRuntime::new(
        &resource,
        "shared-mux",
        BoundedConfig::default(),
    ));
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let before = metrics.snapshot().release_errors;

    // First acquire + release: release_one fails.
    let h1 = runtime
        .acquire(
            &resource,
            &ctx(),
            &rq,
            0,
            &AcquireOptions::default(),
            Some(metrics.clone()),
        )
        .await
        .expect("first acquire must succeed");
    drop(h1);

    // (1) The failed release is observed via the release-error metric.
    let observed = poll_until(Duration::from_secs(2), || {
        metrics.snapshot().release_errors > before
    })
    .await;
    assert!(
        observed,
        "a failing release_one on Capped<1> must increment the \
         release-error metric (observed, not swallowed); release_errors \
         stayed at {before}"
    );
    assert!(
        resource.release_attempts.load(Ordering::SeqCst) >= 1,
        "release_one must have been invoked"
    );

    // (3) The shared runtime must NOT have been destroyed. Give the release
    // queue ample time to run the (mis-)destroy if the regression were
    // present, then assert it never happened.
    let wrongly_destroyed = poll_until(Duration::from_millis(500), || {
        resource.destroy_calls.load(Ordering::SeqCst) >= 1
    })
    .await;
    assert!(
        !wrongly_destroyed,
        "Capped<1> is a shared multiplexer: a failed release_one must NOT \
         destroy the shared runtime (S4 destroy is Exclusive-only); \
         observed destroy_calls={}",
        resource.destroy_calls.load(Ordering::SeqCst)
    );

    // (2) The permit was returned, so a second acquire still succeeds —
    // against the surviving shared runtime, proving it was not destroyed.
    let h2 = runtime
        .acquire(
            &resource,
            &ctx(),
            &rq,
            0,
            &AcquireOptions::default()
                .with_deadline(std::time::Instant::now() + Duration::from_secs(2)),
            Some(metrics.clone()),
        )
        .await;
    assert!(
        h2.is_ok(),
        "the permit must be returned after a failed release_one so the \
         next acquire does not deadlock, and the shared runtime must still \
         be usable: {:?}",
        h2.err()
    );
    drop(h2);

    // Still no destroy after the second cycle.
    assert_eq!(
        resource.destroy_calls.load(Ordering::SeqCst),
        0,
        "the shared Capped<1> runtime must never be destroyed on a failed \
         release"
    );

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}
