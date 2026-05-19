//! Behavior baseline for the three topologies that will be folded into one
//! parameterized `Bounded` runtime (Service / Transport / Exclusive).
//!
//! These tests are an **output-equivalence oracle**. They drive the current
//! per-topology runtimes and capture the externally observable outcome of
//! each acceptance scenario as an ordered event log, serialized to a stable
//! JSON golden fixture under `tests/fixtures/`. A later refactor re-authors
//! the assertions onto `Bounded` and replays the *same* scenarios against
//! the *same* committed goldens, asserting byte-equality — so the fold is
//! diffed against a frozen baseline that does not move with the API.
//!
//! Acceptance scenarios captured:
//! - **AE1** Service `TokenMode::Cloned` → owned handle, release callback
//!   never fires.
//! - **AE2** Service `TokenMode::Tracked` → guarded handle, `release_token`
//!   fires on drop.
//! - **AE3** Exclusive → the next acquire blocks until the previous lease's
//!   `reset` has completed *and* the permit was returned (permit-held-until-
//!   reset ordering, #384).
//!
//! Preserve-nets (green now, green after the fold):
//! - Transport `close_session` fires on session drop with the healthy flag.
//! - A `max_sessions`-bounded transport caps concurrency.
//!
//! `Transport::keepalive` is deliberately *not* asserted here: the current
//! `TransportRuntime` never invokes it (it is an unwired trait default), so
//! a U1 "keepalive fires" net would assert a non-existent path. Wiring +
//! testing keepalive belongs with the `Bounded` fold that owns that method.
//!
//! Regenerate the goldens with `NEBULA_REGENERATE_GOLDENS=1`.

mod golden;

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_core::{ExecutionId, ResourceKey, resource_key, scope::Scope};
use nebula_resource::{
    AcquireOptions, ExclusiveConfig, ExclusiveRuntime, Resource, ResourceConfig, ResourceContext,
    ServiceRuntime, TransportConfig, TransportRuntime,
    error::Error,
    release_queue::ReleaseQueue,
    resource::ResourceMetadata,
    topology::{
        service::{Service, TokenMode, config::Config as ServiceConfig},
        transport::Transport,
    },
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use golden::EventLog;

// ---------------------------------------------------------------------------
// Shared error / config scaffolding
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct BoundedError(String);

impl std::fmt::Display for BoundedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BoundedError {}

impl From<BoundedError> for Error {
    fn from(e: BoundedError) -> Self {
        Error::transient(e.0)
    }
}

#[derive(Clone)]
struct BoundedConfig;

nebula_resource::impl_empty_has_schema!(BoundedConfig);

impl ResourceConfig for BoundedConfig {
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

/// Polls `cond` until it returns `true` or the deadline elapses. Replaces
/// fixed sleeps for release-queue-driven side effects: the assertion is the
/// observed counter, not a wall-clock guess.
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
// AE1 / AE2 — Service token modes
// ---------------------------------------------------------------------------

/// A service whose token mode is selected per-instance via a const generic so
/// AE1 and AE2 share one descriptor. `release_calls` counts `release_token`
/// dispatches so the test can prove Cloned never releases and Tracked does.
#[derive(Clone)]
struct ServiceFixture<const TRACKED: bool> {
    release_calls: Arc<AtomicUsize>,
}

impl<const TRACKED: bool> Resource for ServiceFixture<TRACKED> {
    type Config = BoundedConfig;
    type Runtime = &'static str;
    type Lease = String;
    type Error = BoundedError;

    fn key() -> ResourceKey {
        if TRACKED {
            resource_key!("bounded-svc-tracked")
        } else {
            resource_key!("bounded-svc-cloned")
        }
    }

    async fn create(
        &self,
        _config: &BoundedConfig,
        _ctx: &ResourceContext,
    ) -> Result<&'static str, BoundedError> {
        Ok("svc-runtime")
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl<const TRACKED: bool> Service for ServiceFixture<TRACKED> {
    const TOKEN_MODE: TokenMode = if TRACKED {
        TokenMode::Tracked
    } else {
        TokenMode::Cloned
    };

    async fn acquire_token(
        &self,
        runtime: &&'static str,
        _ctx: &ResourceContext,
    ) -> Result<String, BoundedError> {
        Ok(format!("{runtime}-token"))
    }

    async fn release_token(
        &self,
        _runtime: &&'static str,
        _token: String,
    ) -> Result<(), BoundedError> {
        self.release_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// AE1: a `Cloned`-mode service hands out an *owned* handle and never runs
/// the release callback when the handle drops.
#[tokio::test]
async fn ae1_cloned_service_yields_owned_no_release_callback() {
    let release_calls = Arc::new(AtomicUsize::new(0));
    let resource = ServiceFixture::<false> {
        release_calls: Arc::clone(&release_calls),
    };
    let runtime: ServiceRuntime<ServiceFixture<false>> =
        ServiceRuntime::new("svc-runtime", ServiceConfig::default());
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let mut log = EventLog::new();

    let handle = runtime
        .acquire(&resource, &ctx(), &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("cloned-service acquire must succeed");
    log.push("acquired", &format!("token={}", &*handle));

    drop(handle);
    // Even with a generous settle window, a Cloned-mode handle is owned —
    // there is no callback to run, so the counter must stay at 0.
    let fired = poll_until(Duration::from_millis(200), || {
        release_calls.load(Ordering::SeqCst) > 0
    })
    .await;
    log.push(
        "release_callback_fired",
        if fired { "true" } else { "false" },
    );

    assert!(
        !fired,
        "Cloned token mode must produce an owned handle with NO release \
         callback (release_token must not be invoked)"
    );

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;

    log.assert_matches_golden("ae1_cloned_service");
}

/// AE2: a `Tracked`-mode service hands out a *guarded* handle whose drop
/// runs `release_token` exactly once.
#[tokio::test]
async fn ae2_tracked_service_release_token_fires_on_drop() {
    let release_calls = Arc::new(AtomicUsize::new(0));
    let resource = ServiceFixture::<true> {
        release_calls: Arc::clone(&release_calls),
    };
    let runtime: ServiceRuntime<ServiceFixture<true>> =
        ServiceRuntime::new("svc-runtime", ServiceConfig::default());
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let mut log = EventLog::new();

    let handle = runtime
        .acquire(&resource, &ctx(), &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("tracked-service acquire must succeed");
    log.push("acquired", &format!("token={}", &*handle));

    drop(handle);
    let fired = poll_until(Duration::from_secs(2), || {
        release_calls.load(Ordering::SeqCst) == 1
    })
    .await;
    log.push(
        "release_token_calls",
        &release_calls.load(Ordering::SeqCst).to_string(),
    );

    assert!(
        fired,
        "Tracked token mode must produce a guarded handle whose drop runs \
         release_token exactly once; observed {}",
        release_calls.load(Ordering::SeqCst)
    );

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;

    log.assert_matches_golden("ae2_tracked_service");
}

// ---------------------------------------------------------------------------
// AE3 — Exclusive reset-before-next-acquire ordering
// ---------------------------------------------------------------------------

/// An exclusive resource whose `reset` parks on a gate so the test can prove
/// the *next* acquire is blocked until reset both completes and the permit
/// is returned (permit-held-until-reset, #384).
#[derive(Clone)]
struct GatedResetExclusive {
    reset_started: Arc<Notify>,
    release_reset: Arc<Notify>,
    reset_completed: Arc<AtomicUsize>,
}

impl Resource for GatedResetExclusive {
    type Config = BoundedConfig;
    type Runtime = u32;
    type Lease = u32;
    type Error = BoundedError;

    fn key() -> ResourceKey {
        resource_key!("bounded-excl-gated")
    }

    async fn create(
        &self,
        _config: &BoundedConfig,
        _ctx: &ResourceContext,
    ) -> Result<u32, BoundedError> {
        Ok(1)
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl nebula_resource::topology::exclusive::Exclusive for GatedResetExclusive {
    async fn reset(&self, _runtime: &u32) -> Result<(), BoundedError> {
        self.reset_started.notify_one();
        self.release_reset.notified().await;
        self.reset_completed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// AE3: while the first lease's `reset` is parked, a second acquire must NOT
/// complete. It unblocks only once `reset` finishes and the permit is
/// returned — proving the next caller waits for reset (and the captured
/// ordering is frozen as the fold's contract).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ae3_exclusive_next_acquire_waits_for_reset() {
    let resource = GatedResetExclusive {
        reset_started: Arc::new(Notify::new()),
        release_reset: Arc::new(Notify::new()),
        reset_completed: Arc::new(AtomicUsize::new(0)),
    };
    let config = ExclusiveConfig {
        acquire_timeout: Duration::from_secs(5),
    };
    let runtime: Arc<ExclusiveRuntime<GatedResetExclusive>> =
        Arc::new(ExclusiveRuntime::new(1u32, config));
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let mut log = EventLog::new();

    let h1 = runtime
        .acquire(&resource, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("first exclusive acquire must succeed");
    log.push("first_acquired", "");

    // Drop the first lease: its `reset` runs on the release queue and parks.
    drop(h1);
    resource.reset_started.notified().await;
    log.push("reset_started", "");

    // Start the second acquire while reset is parked. It must be pending —
    // the permit is held until reset completes (#384).
    let second = {
        let runtime = Arc::clone(&runtime);
        let resource = resource.clone();
        let rq = Arc::clone(&rq);
        tokio::spawn(async move {
            runtime
                .acquire(&resource, &rq, 0, &AcquireOptions::default(), None)
                .await
        })
    };

    let mut second = second;
    let pending = tokio::time::timeout(Duration::from_millis(150), &mut second).await;
    assert!(
        pending.is_err(),
        "the second acquire must be parked while the previous lease's reset \
         is still running (permit-held-until-reset, #384)"
    );
    log.push(
        "second_blocked_while_reset_pending",
        &resource.reset_completed.load(Ordering::SeqCst).to_string(),
    );
    assert_eq!(
        resource.reset_completed.load(Ordering::SeqCst),
        0,
        "reset must not have completed yet"
    );

    // Release the parked reset. The second acquire now unblocks.
    resource.release_reset.notify_one();
    let h2 = second
        .await
        .expect("second acquire task must not panic")
        .expect("second acquire must succeed once reset completes");
    log.push(
        "second_acquired_after_reset",
        &resource.reset_completed.load(Ordering::SeqCst).to_string(),
    );
    assert_eq!(
        resource.reset_completed.load(Ordering::SeqCst),
        1,
        "reset must have completed exactly once before the next acquire \
         was granted"
    );

    drop(h2);
    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;

    log.assert_matches_golden("ae3_exclusive_reset_ordering");
}

// ---------------------------------------------------------------------------
// Transport preserve-nets — close_session, max_sessions cap, keepalive
// ---------------------------------------------------------------------------

/// A transport that records `open_session` / `close_session` (with the
/// healthy flag) and `keepalive` invocations through shared counters.
#[derive(Clone)]
struct TransportFixture {
    open_calls: Arc<AtomicUsize>,
    close_calls: Arc<AtomicUsize>,
    last_close_healthy: Arc<AtomicU64>,
}

impl TransportFixture {
    fn new() -> Self {
        Self {
            open_calls: Arc::new(AtomicUsize::new(0)),
            close_calls: Arc::new(AtomicUsize::new(0)),
            last_close_healthy: Arc::new(AtomicU64::new(u64::MAX)),
        }
    }
}

impl Resource for TransportFixture {
    type Config = BoundedConfig;
    type Runtime = &'static str;
    type Lease = u64;
    type Error = BoundedError;

    fn key() -> ResourceKey {
        resource_key!("bounded-transport")
    }

    async fn create(
        &self,
        _config: &BoundedConfig,
        _ctx: &ResourceContext,
    ) -> Result<&'static str, BoundedError> {
        Ok("transport-runtime")
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Transport for TransportFixture {
    async fn open_session(
        &self,
        _transport: &&'static str,
        _ctx: &ResourceContext,
    ) -> Result<u64, BoundedError> {
        let id = self.open_calls.fetch_add(1, Ordering::SeqCst) as u64;
        Ok(id)
    }

    async fn close_session(
        &self,
        _transport: &&'static str,
        _session: u64,
        healthy: bool,
    ) -> Result<(), BoundedError> {
        self.last_close_healthy
            .store(u64::from(healthy), Ordering::SeqCst);
        self.close_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Preserve-net: dropping a transport session runs `close_session` on the
/// release queue with the healthy flag.
#[tokio::test]
async fn transport_close_session_fires_on_drop() {
    let resource = TransportFixture::new();
    let runtime: TransportRuntime<TransportFixture> = TransportRuntime::new(
        "transport-runtime",
        TransportConfig {
            max_sessions: 4,
            keepalive_interval: None,
            acquire_timeout: Duration::from_secs(1),
        },
    );
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let session = runtime
        .acquire(&resource, &ctx(), &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("transport acquire must succeed");
    assert_eq!(resource.open_calls.load(Ordering::SeqCst), 1);

    drop(session);
    let closed = poll_until(Duration::from_secs(2), || {
        resource.close_calls.load(Ordering::SeqCst) == 1
    })
    .await;
    assert!(
        closed,
        "close_session must fire exactly once on session drop; observed {}",
        resource.close_calls.load(Ordering::SeqCst)
    );
    assert_eq!(
        resource.last_close_healthy.load(Ordering::SeqCst),
        1,
        "a normally-dropped session closes healthy"
    );

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

/// Preserve-net: a `max_sessions = 1` transport serializes a second acquire
/// behind the first session's drop (concurrency bound).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transport_max_sessions_bounds_concurrency() {
    let resource = TransportFixture::new();
    let runtime: Arc<TransportRuntime<TransportFixture>> = Arc::new(TransportRuntime::new(
        "transport-runtime",
        TransportConfig {
            max_sessions: 1,
            keepalive_interval: None,
            acquire_timeout: Duration::from_secs(2),
        },
    ));
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let s1 = runtime
        .acquire(&resource, &ctx(), &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("first session must open");

    // Second acquire must be pending while s1 holds the only session slot.
    let second = {
        let runtime = Arc::clone(&runtime);
        let resource = resource.clone();
        let rq = Arc::clone(&rq);
        tokio::spawn(async move {
            runtime
                .acquire(&resource, &ctx(), &rq, 0, &AcquireOptions::default(), None)
                .await
        })
    };
    let mut second = second;
    let pending = tokio::time::timeout(Duration::from_millis(150), &mut second).await;
    assert!(
        pending.is_err(),
        "max_sessions=1 must block the second acquire until s1 is released"
    );

    drop(s1);
    let s2 = second
        .await
        .expect("second session task must not panic")
        .expect("second session must open once the first is released");
    drop(s2);

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}
