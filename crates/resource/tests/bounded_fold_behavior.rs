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
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use nebula_core::{ExecutionId, ResourceKey, resource_key, scope::Scope};
use nebula_resource::{
    AcquireOptions, BoundedConfig as BoundedRtConfig, BoundedRuntime, Resource, ResourceConfig,
    ResourceContext,
    error::Error,
    release_queue::ReleaseQueue,
    resource::ResourceMetadata,
    topology::bounded::{Bounded, BoundedRelease, Capped},
};
use tokio_util::sync::CancellationToken;

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
// Transport-fold preserve net — `close_session` healthy flag on normal drop
// ---------------------------------------------------------------------------
//
// The former `Transport::close_session(_, _, healthy)` folds onto
// `BoundedRelease::release_one(_, _, healthy)` for the `Capped` cap. The
// AE1/AE2/AE3 acceptance outcomes are replayed byte-for-byte against the
// committed U1 goldens in `mod golden_replay` below; `mod bounded_path`
// covers the `Capped<N>` concurrency bound and keepalive. The one
// behavior those do not pin is the *value* of the `healthy` flag handed
// to the release hook on a *normally-dropped* lease — a healthy drop must
// pass `healthy = true`. This net preserves that assertion on the unified
// `BoundedRuntime`.

#[derive(Clone)]
struct HealthyFlagFixture {
    last_release_healthy: Arc<AtomicU64>,
}

impl Resource for HealthyFlagFixture {
    type Config = BoundedConfig;
    type Runtime = &'static str;
    type Lease = u64;
    type Error = BoundedError;

    fn key() -> ResourceKey {
        resource_key!("bounded-healthy-flag")
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

impl Bounded for HealthyFlagFixture {
    type Cap = Capped<4>;

    async fn acquire_one(
        &self,
        _runtime: &&'static str,
        _ctx: &ResourceContext,
    ) -> Result<u64, BoundedError> {
        Ok(0)
    }
}

impl BoundedRelease for HealthyFlagFixture {
    async fn release_one(
        &self,
        _runtime: &&'static str,
        _lease: u64,
        healthy: bool,
    ) -> Result<(), BoundedError> {
        // Folds the old `Transport::close_session` healthy flag.
        self.last_release_healthy
            .store(u64::from(healthy), Ordering::SeqCst);
        Ok(())
    }
}

/// Preserve-net (folds the U1 `transport_close_session_fires_on_drop`
/// healthy-flag assertion): a normally-dropped Capped lease runs
/// `release_one` with `healthy = true`.
#[tokio::test]
async fn bounded_capped_release_one_healthy_on_normal_drop() {
    let resource = HealthyFlagFixture {
        last_release_healthy: Arc::new(AtomicU64::new(u64::MAX)),
    };
    let runtime: BoundedRuntime<HealthyFlagFixture> =
        BoundedRuntime::new(&resource, "transport-runtime", BoundedRtConfig::default());
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let session = runtime
        .acquire(&resource, &ctx(), &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("capped acquire must succeed");

    drop(session);
    let observed = poll_until(Duration::from_secs(2), || {
        resource.last_release_healthy.load(Ordering::SeqCst) != u64::MAX
    })
    .await;
    assert!(observed, "release_one must fire on a normal drop");
    assert_eq!(
        resource.last_release_healthy.load(Ordering::SeqCst),
        1,
        "a normally-dropped lease closes healthy (healthy = true)"
    );

    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}

// ===========================================================================
// Bounded-path equivalents — the same acceptance scenarios re-expressed on
// the unified `BoundedRuntime`. These ADD coverage on top of the
// `golden_replay` byte-equality nets below: AE1 (Unbounded -> owned), AE2
// (Capped -> release fires), AE3 (Exclusive cap -> next acquire waits for
// reset), the `Capped<N>` concurrency bound, and keepalive actually
// firing (the previously-unwired `Transport::keepalive`).
// ===========================================================================
mod bounded_path {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use nebula_core::{ResourceKey, resource_key};
    use nebula_resource::{
        AcquireOptions, BoundedConfig, BoundedRuntime, Resource, ResourceConfig, ResourceContext,
        error::Error,
        release_queue::ReleaseQueue,
        resource::ResourceMetadata,
        topology::bounded::{
            Bounded, BoundedRelease, Capped, Exclusive as ExclusiveCap, Unbounded,
        },
    };
    use tokio::sync::Notify;

    use super::{BoundedError, ctx, poll_until};

    // -- AE1 Bounded: Unbounded cap → owned handle, no release callback ----

    /// Unbounded-cap resource (Service `Cloned` analogue). It has no
    /// `release_one` of its own — the blanket `BoundedRelease for
    /// Cap = Unbounded` supplies the never-called no-op, which is itself
    /// the proof that Cloned-mode authors write zero release boilerplate.
    #[derive(Clone)]
    struct UnboundedFixture {
        // If this ever increments, the owned-handle contract broke.
        release_calls: Arc<AtomicUsize>,
    }

    impl Resource for UnboundedFixture {
        type Config = BoundedConfig2;
        type Runtime = &'static str;
        type Lease = String;
        type Error = BoundedError;

        fn key() -> ResourceKey {
            resource_key!("bp-unbounded")
        }

        async fn create(
            &self,
            _config: &BoundedConfig2,
            _ctx: &ResourceContext,
        ) -> Result<&'static str, BoundedError> {
            Ok("svc")
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Bounded for UnboundedFixture {
        type Cap = Unbounded;

        async fn acquire_one(
            &self,
            runtime: &&'static str,
            _ctx: &ResourceContext,
        ) -> Result<String, BoundedError> {
            Ok(format!("{runtime}-token"))
        }
    }

    #[derive(Clone)]
    struct BoundedConfig2;
    nebula_resource::impl_empty_has_schema!(BoundedConfig2);
    impl ResourceConfig for BoundedConfig2 {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn ae1_bounded_unbounded_yields_owned_no_release_callback() {
        let resource = UnboundedFixture {
            release_calls: Arc::new(AtomicUsize::new(0)),
        };
        let runtime: BoundedRuntime<UnboundedFixture> =
            BoundedRuntime::new(&resource, "svc", BoundedConfig::default());
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);

        let handle = runtime
            .acquire(&resource, &ctx(), &rq, 0, &AcquireOptions::default(), None)
            .await
            .expect("unbounded acquire must succeed");
        assert_eq!(&*handle, "svc-token");
        // Owned handle — no generation, no callback.
        assert!(handle.generation().is_none());

        drop(handle);
        let fired = poll_until(Duration::from_millis(200), || {
            resource.release_calls.load(Ordering::SeqCst) > 0
        })
        .await;
        assert!(
            !fired,
            "Unbounded cap must produce an owned handle with NO release \
             callback"
        );

        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;
    }

    // -- AE2 Bounded: Capped cap → guarded handle, release_one fires ------

    #[derive(Clone)]
    struct CappedFixture {
        release_calls: Arc<AtomicUsize>,
    }

    impl Resource for CappedFixture {
        type Config = BoundedConfig2;
        type Runtime = &'static str;
        type Lease = String;
        type Error = BoundedError;

        fn key() -> ResourceKey {
            resource_key!("bp-capped")
        }

        async fn create(
            &self,
            _config: &BoundedConfig2,
            _ctx: &ResourceContext,
        ) -> Result<&'static str, BoundedError> {
            Ok("svc")
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Bounded for CappedFixture {
        type Cap = Capped<8>;

        async fn acquire_one(
            &self,
            runtime: &&'static str,
            _ctx: &ResourceContext,
        ) -> Result<String, BoundedError> {
            Ok(format!("{runtime}-tracked"))
        }
    }

    impl BoundedRelease for CappedFixture {
        async fn release_one(
            &self,
            _runtime: &&'static str,
            _lease: String,
            _healthy: bool,
        ) -> Result<(), BoundedError> {
            self.release_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn ae2_bounded_capped_release_one_fires_on_drop() {
        let resource = CappedFixture {
            release_calls: Arc::new(AtomicUsize::new(0)),
        };
        let runtime: BoundedRuntime<CappedFixture> =
            BoundedRuntime::new(&resource, "svc", BoundedConfig::default());
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);

        let handle = runtime
            .acquire(&resource, &ctx(), &rq, 1, &AcquireOptions::default(), None)
            .await
            .expect("capped acquire must succeed");
        assert_eq!(&*handle, "svc-tracked");
        assert_eq!(handle.generation(), Some(1));

        drop(handle);
        let fired = poll_until(Duration::from_secs(2), || {
            resource.release_calls.load(Ordering::SeqCst) == 1
        })
        .await;
        assert!(
            fired,
            "Capped cap must produce a guarded handle whose drop runs \
             release_one exactly once; observed {}",
            resource.release_calls.load(Ordering::SeqCst)
        );

        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;
    }

    // -- AE3 Bounded: Exclusive cap → next acquire waits for reset --------

    #[derive(Clone)]
    struct GatedResetBounded {
        reset_started: Arc<Notify>,
        release_reset: Arc<Notify>,
        reset_completed: Arc<AtomicUsize>,
    }

    impl Resource for GatedResetBounded {
        type Config = BoundedConfig2;
        type Runtime = u32;
        type Lease = u32;
        type Error = BoundedError;

        fn key() -> ResourceKey {
            resource_key!("bp-excl-gated")
        }

        async fn create(
            &self,
            _config: &BoundedConfig2,
            _ctx: &ResourceContext,
        ) -> Result<u32, BoundedError> {
            Ok(1)
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Bounded for GatedResetBounded {
        type Cap = ExclusiveCap;

        async fn acquire_one(
            &self,
            runtime: &u32,
            _ctx: &ResourceContext,
        ) -> Result<u32, BoundedError> {
            Ok(*runtime)
        }
    }

    impl BoundedRelease for GatedResetBounded {
        async fn release_one(
            &self,
            _runtime: &u32,
            _lease: u32,
            _healthy: bool,
        ) -> Result<(), BoundedError> {
            self.reset_started.notify_one();
            self.release_reset.notified().await;
            self.reset_completed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn ae3_bounded_exclusive_next_acquire_waits_for_reset() {
        let resource = GatedResetBounded {
            reset_started: Arc::new(Notify::new()),
            release_reset: Arc::new(Notify::new()),
            reset_completed: Arc::new(AtomicUsize::new(0)),
        };
        let runtime: Arc<BoundedRuntime<GatedResetBounded>> = Arc::new(BoundedRuntime::new(
            &resource,
            1u32,
            BoundedConfig::default(),
        ));
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);

        let h1 = runtime
            .acquire(&resource, &ctx(), &rq, 0, &AcquireOptions::default(), None)
            .await
            .expect("first exclusive-cap acquire must succeed");

        drop(h1);
        resource.reset_started.notified().await;

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
            "the second acquire must be parked while the previous lease's \
             reset is still running (permit-held-until-reset, #384)"
        );
        assert_eq!(
            resource.reset_completed.load(Ordering::SeqCst),
            0,
            "reset must not have completed yet"
        );

        resource.release_reset.notify_one();
        let h2 = second
            .await
            .expect("second acquire task must not panic")
            .expect("second acquire must succeed once reset completes");
        assert_eq!(
            resource.reset_completed.load(Ordering::SeqCst),
            1,
            "reset must have completed exactly once before the next acquire \
             was granted"
        );

        drop(h2);
        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;
    }

    // -- Edge: Capped<1> bounds concurrency to 1 -------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn bounded_capped_one_bounds_concurrency() {
        // Capped<1>: a second acquire must wait for the first to release.
        #[derive(Clone)]
        struct Cap1 {
            released: Arc<AtomicUsize>,
        }
        impl Resource for Cap1 {
            type Config = BoundedConfig2;
            type Runtime = &'static str;
            type Lease = String;
            type Error = BoundedError;
            fn key() -> ResourceKey {
                resource_key!("bp-cap1")
            }
            async fn create(
                &self,
                _c: &BoundedConfig2,
                _x: &ResourceContext,
            ) -> Result<&'static str, BoundedError> {
                Ok("rt")
            }
            fn metadata() -> ResourceMetadata {
                ResourceMetadata::from_key(&Self::key())
            }
        }
        impl Bounded for Cap1 {
            type Cap = Capped<1>;
            async fn acquire_one(
                &self,
                r: &&'static str,
                _x: &ResourceContext,
            ) -> Result<String, BoundedError> {
                Ok((*r).to_owned())
            }
        }
        impl BoundedRelease for Cap1 {
            async fn release_one(
                &self,
                _r: &&'static str,
                _l: String,
                _h: bool,
            ) -> Result<(), BoundedError> {
                self.released.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }

        let resource = Cap1 {
            released: Arc::new(AtomicUsize::new(0)),
        };
        let runtime: Arc<BoundedRuntime<Cap1>> = Arc::new(BoundedRuntime::new(
            &resource,
            "rt",
            BoundedConfig::default(),
        ));
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);

        let s1 = runtime
            .acquire(&resource, &ctx(), &rq, 0, &AcquireOptions::default(), None)
            .await
            .expect("first acquire must succeed");

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
            "Capped<1> must block the second acquire until the first \
             permit is released"
        );

        drop(s1);
        let s2 = second
            .await
            .expect("second task must not panic")
            .expect("second acquire must succeed once the permit is freed");
        drop(s2);

        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;
    }

    // -- Edge: keepalive actually fires (previously-unwired path) ---------

    #[derive(Clone)]
    struct KeepaliveFixture {
        keepalive_calls: Arc<AtomicUsize>,
    }

    impl Resource for KeepaliveFixture {
        type Config = BoundedConfig2;
        type Runtime = &'static str;
        type Lease = String;
        type Error = BoundedError;

        fn key() -> ResourceKey {
            resource_key!("bp-keepalive")
        }

        async fn create(
            &self,
            _config: &BoundedConfig2,
            _ctx: &ResourceContext,
        ) -> Result<&'static str, BoundedError> {
            Ok("conn")
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Bounded for KeepaliveFixture {
        type Cap = Capped<4>;

        async fn acquire_one(
            &self,
            runtime: &&'static str,
            _ctx: &ResourceContext,
        ) -> Result<String, BoundedError> {
            Ok((*runtime).to_owned())
        }
    }

    impl BoundedRelease for KeepaliveFixture {
        async fn release_one(
            &self,
            _runtime: &&'static str,
            _lease: String,
            _healthy: bool,
        ) -> Result<(), BoundedError> {
            Ok(())
        }

        async fn keepalive(&self, _runtime: &&'static str) -> Result<(), BoundedError> {
            self.keepalive_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// The Bounded fold wires `keepalive` (the previously-unwired
    /// `Transport::keepalive`): with a non-`None` interval the runtime
    /// drives it on a background ticker.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bounded_keepalive_fires_on_interval() {
        let resource = KeepaliveFixture {
            keepalive_calls: Arc::new(AtomicUsize::new(0)),
        };
        let config = BoundedConfig {
            keepalive_interval: Some(Duration::from_millis(20)),
            ..BoundedConfig::default()
        };
        let runtime: BoundedRuntime<KeepaliveFixture> =
            BoundedRuntime::new(&resource, "conn", config);

        let fired = poll_until(Duration::from_secs(2), || {
            resource.keepalive_calls.load(Ordering::SeqCst) >= 2
        })
        .await;
        assert!(
            fired,
            "keepalive must fire repeatedly on the configured interval; \
             observed {}",
            resource.keepalive_calls.load(Ordering::SeqCst)
        );

        // Dropping the runtime aborts the keepalive task.
        drop(runtime);
    }
}

// ===========================================================================
// Golden replay — the U1↔U11 output-equivalence oracle.
//
// U1 drove AE1/AE2/AE3 through the *old* per-topology runtimes and froze
// the observable outcome as committed JSON fixtures
// (`tests/fixtures/ae{1,2,3}_*.golden`). The U3 Bounded-path tests above
// add Bounded coverage but deliberately do NOT assert against those
// goldens. These replays close the loop: the *same* AE scenarios driven
// through the unified `BoundedRuntime`, emitting the *same* ordered
// `EventLog`, asserted **byte-for-byte** against the committed fixtures.
// A non-empty diff is a behavior regression in the fold — investigate,
// never regenerate the goldens to paper over it.
//
// The fixtures here therefore reproduce the *observable* values of the
// U1 fixtures exactly (runtime string `svc-runtime`, token
// `svc-runtime-token`, the reset-ordering step names) — only the API
// underneath changed (Service/Transport/Exclusive → one Bounded cap).
// ===========================================================================

mod golden_replay {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use nebula_core::{ResourceKey, resource_key};
    use nebula_resource::{
        AcquireOptions, BoundedConfig, BoundedRuntime, Resource, ResourceConfig, ResourceContext,
        error::Error,
        release_queue::ReleaseQueue,
        resource::ResourceMetadata,
        topology::bounded::{
            Bounded, BoundedRelease, Capped, Exclusive as ExclusiveCap, Unbounded,
        },
    };
    use tokio::sync::Notify;

    use super::{BoundedError, ctx, poll_until};
    use crate::golden::EventLog;

    #[derive(Clone)]
    struct GReplayConfig;
    nebula_resource::impl_empty_has_schema!(GReplayConfig);
    impl ResourceConfig for GReplayConfig {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
    }

    // -- AE1 replay: Unbounded cap, observable outcome == ae1 golden ------

    /// Reproduces the U1 AE1 fixture's observable values exactly: the
    /// runtime is `svc-runtime`, the lease is `svc-runtime-token`, and a
    /// Cloned/Unbounded handle never runs a release callback. Driven
    /// through `BoundedRuntime<Cap = Unbounded>` instead of the old
    /// `ServiceRuntime`. It carries no release counter: the `Unbounded`
    /// cap is covered by the blanket no-op `BoundedRelease` (an author
    /// *cannot* supply `release_one` for it), so "release never fires" is
    /// guaranteed structurally — the test asserts it via a standalone
    /// counter that nothing can bump.
    #[derive(Clone)]
    struct Ae1Replay;

    impl Resource for Ae1Replay {
        type Config = GReplayConfig;
        type Runtime = &'static str;
        type Lease = String;
        type Error = BoundedError;

        fn key() -> ResourceKey {
            resource_key!("golden-replay-ae1")
        }

        async fn create(
            &self,
            _config: &GReplayConfig,
            _ctx: &ResourceContext,
        ) -> Result<&'static str, BoundedError> {
            Ok("svc-runtime")
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Bounded for Ae1Replay {
        type Cap = Unbounded;

        async fn acquire_one(
            &self,
            runtime: &&'static str,
            _ctx: &ResourceContext,
        ) -> Result<String, BoundedError> {
            // Byte-identical to the U1 ServiceFixture token shape.
            Ok(format!("{runtime}-token"))
        }
    }

    #[tokio::test]
    async fn ae1_cloned_service_replayed_through_bounded_matches_golden() {
        // Nothing can ever bump this for an Unbounded-cap resource (the
        // blanket no-op `BoundedRelease` is the only impl), so a non-zero
        // reading would be a structural break in the fold.
        let release_calls = Arc::new(AtomicUsize::new(0));
        let resource = Ae1Replay;
        let runtime: BoundedRuntime<Ae1Replay> =
            BoundedRuntime::new(&resource, "svc-runtime", BoundedConfig::default());
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);

        let mut log = EventLog::new();

        let handle = runtime
            .acquire(&resource, &ctx(), &rq, 0, &AcquireOptions::default(), None)
            .await
            .expect("cloned/unbounded acquire must succeed");
        log.push("acquired", &format!("token={}", &*handle));

        drop(handle);
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
            "Unbounded cap must produce an owned handle with NO release \
             callback"
        );

        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;

        // The whole point of U11: the Bounded path reproduces the U1
        // golden byte-for-byte.
        log.assert_matches_golden("ae1_cloned_service");
    }

    // -- AE2 replay: Capped cap, observable outcome == ae2 golden --------

    #[derive(Clone)]
    struct Ae2Replay {
        release_calls: Arc<AtomicUsize>,
    }

    impl Resource for Ae2Replay {
        type Config = GReplayConfig;
        type Runtime = &'static str;
        type Lease = String;
        type Error = BoundedError;

        fn key() -> ResourceKey {
            resource_key!("golden-replay-ae2")
        }

        async fn create(
            &self,
            _config: &GReplayConfig,
            _ctx: &ResourceContext,
        ) -> Result<&'static str, BoundedError> {
            Ok("svc-runtime")
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Bounded for Ae2Replay {
        type Cap = Capped<8>;

        async fn acquire_one(
            &self,
            runtime: &&'static str,
            _ctx: &ResourceContext,
        ) -> Result<String, BoundedError> {
            Ok(format!("{runtime}-token"))
        }
    }

    impl BoundedRelease for Ae2Replay {
        async fn release_one(
            &self,
            _runtime: &&'static str,
            _lease: String,
            _healthy: bool,
        ) -> Result<(), BoundedError> {
            self.release_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn ae2_tracked_service_replayed_through_bounded_matches_golden() {
        let release_calls = Arc::new(AtomicUsize::new(0));
        let resource = Ae2Replay {
            release_calls: Arc::clone(&release_calls),
        };
        let runtime: BoundedRuntime<Ae2Replay> =
            BoundedRuntime::new(&resource, "svc-runtime", BoundedConfig::default());
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);

        let mut log = EventLog::new();

        let handle = runtime
            .acquire(&resource, &ctx(), &rq, 0, &AcquireOptions::default(), None)
            .await
            .expect("tracked/capped acquire must succeed");
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
            "Capped cap must run release_one exactly once on drop; \
             observed {}",
            release_calls.load(Ordering::SeqCst)
        );

        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;

        log.assert_matches_golden("ae2_tracked_service");
    }

    // -- AE3 replay: Exclusive cap, observable outcome == ae3 golden -----

    #[derive(Clone)]
    struct Ae3Replay {
        reset_started: Arc<Notify>,
        release_reset: Arc<Notify>,
        reset_completed: Arc<AtomicUsize>,
    }

    impl Resource for Ae3Replay {
        type Config = GReplayConfig;
        type Runtime = u32;
        type Lease = u32;
        type Error = BoundedError;

        fn key() -> ResourceKey {
            resource_key!("golden-replay-ae3")
        }

        async fn create(
            &self,
            _config: &GReplayConfig,
            _ctx: &ResourceContext,
        ) -> Result<u32, BoundedError> {
            Ok(1)
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Bounded for Ae3Replay {
        type Cap = ExclusiveCap;

        async fn acquire_one(
            &self,
            runtime: &u32,
            _ctx: &ResourceContext,
        ) -> Result<u32, BoundedError> {
            Ok(*runtime)
        }
    }

    impl BoundedRelease for Ae3Replay {
        async fn release_one(
            &self,
            _runtime: &u32,
            _lease: u32,
            _healthy: bool,
        ) -> Result<(), BoundedError> {
            // The Exclusive cap's `release_one` IS the reset (folds the
            // old `Exclusive::reset`); the permit is held until it
            // resolves (#384), exactly as the U1 fixture proved.
            self.reset_started.notify_one();
            self.release_reset.notified().await;
            self.reset_completed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn ae3_exclusive_reset_ordering_replayed_through_bounded_matches_golden() {
        let resource = Ae3Replay {
            reset_started: Arc::new(Notify::new()),
            release_reset: Arc::new(Notify::new()),
            reset_completed: Arc::new(AtomicUsize::new(0)),
        };
        let config = BoundedConfig {
            // Generous permit-acquire budget so the gated reset, not a
            // timeout, is what the second acquire waits on (matches the
            // 5s the U1 Exclusive fixture used).
            acquire_timeout: Duration::from_secs(5),
            ..BoundedConfig::default()
        };
        let runtime: Arc<BoundedRuntime<Ae3Replay>> =
            Arc::new(BoundedRuntime::new(&resource, 1u32, config));
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);

        let mut log = EventLog::new();

        let h1 = runtime
            .acquire(&resource, &ctx(), &rq, 0, &AcquireOptions::default(), None)
            .await
            .expect("first exclusive-cap acquire must succeed");
        log.push("first_acquired", "");

        drop(h1);
        resource.reset_started.notified().await;
        log.push("reset_started", "");

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
            "the second acquire must be parked while the previous lease's \
             reset is still running (permit-held-until-reset, #384)"
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
}
