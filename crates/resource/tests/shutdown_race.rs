//! Regression tests for the `graceful_shutdown` race with in-flight acquires.
//!
//! Pre-existing race surfaced by CodeRabbit's post-fix review on PR #613:
//!
//! Phase 2 of `graceful_shutdown` watches `drain_tracker`, but if an acquire
//! passes `lookup()` BEFORE the cancel token fires, the acquire can complete
//! AFTER Phase 3 clears the registry — caller ends up with a `ResourceGuard`
//! pointing at cleared state.
//!
//! Fix combines two defenses:
//!
//! - **Defense A** — `lookup<R>()` rejects with `Error::cancelled` once `Manager::shutting_down` is
//!   set, so any post-cancel acquire fails fast.
//! - **Defense B** — `drain_tracker` is incremented BEFORE any acquire `await` point via an
//!   `InFlightCounter` RAII guard; the slot is handed off to the resulting `ResourceGuard` on
//!   success or decremented on failure / cancel. Together they ensure `wait_for_drain()` blocks
//!   until every in-flight acquire either drains into a guard or fails fast.
//!
//! Test invariant: there is no scenario where the caller holds a guard but
//! the registry has been cleared.

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use nebula_core::{ExecutionId, ResourceKey, scope::Scope};
use nebula_credential::{Credential, NoCredential};
use nebula_resource::{
    AcquireOptions, Manager, ResourceContext, ScopeLevel, ShutdownConfig, TopologyTag,
    error::{Error, ErrorKind},
    resource::{Resource, ResourceConfig, ResourceMetadata},
    runtime::{TopologyRuntime, resident::ResidentRuntime},
    topology::{resident, resident::Resident},
};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// SlowCreateResource — a resident resource whose `create()` sleeps for a
// configurable duration. Lets the test drive an acquire into a known
// in-flight window before triggering shutdown.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct SlowError(&'static str);

impl std::fmt::Display for SlowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SlowError {}

impl From<SlowError> for Error {
    fn from(e: SlowError) -> Self {
        Error::permanent(e.0)
    }
}

#[derive(Clone, Debug, Default)]
struct SlowConfig;

nebula_schema::impl_empty_has_schema!(SlowConfig);

impl ResourceConfig for SlowConfig {}

#[derive(Clone)]
struct SlowCreateResource {
    create_delay: Duration,
    create_count: Arc<AtomicU64>,
}

impl SlowCreateResource {
    fn new(create_delay: Duration) -> Self {
        Self {
            create_delay,
            create_count: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Resource for SlowCreateResource {
    type Config = SlowConfig;
    type Runtime = ();
    type Lease = ();
    type Error = SlowError;
    type Credential = NoCredential;

    fn key() -> ResourceKey {
        nebula_core::resource_key!("test.shutdown_race.slow")
    }

    fn create(
        &self,
        _config: &Self::Config,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<(), SlowError>> + Send {
        let delay = self.create_delay;
        let counter = Arc::clone(&self.create_count);
        async move {
            tokio::time::sleep(delay).await;
            counter.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Resident for SlowCreateResource {
    fn is_alive_sync(&self, _runtime: &()) -> bool {
        true
    }
}

fn test_ctx() -> ResourceContext {
    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        ..Default::default()
    };
    ResourceContext::minimal(scope, CancellationToken::new())
}

// ---------------------------------------------------------------------------
// Test: graceful_shutdown blocks (or rejects) an in-flight acquire that
// passed `lookup()` before the cancel token fired. The pre-fix code would
// (a) let `wait_for_drain` see `0` because the acquire had not yet built
// the guard, then (b) clear the registry, then (c) hand the caller a guard
// pointing at the cleared registry.
//
// Post-fix: either the acquire is counted in `drain_tracker` (Defense B)
// so `wait_for_drain` blocks for it, or — in the rare case the test
// schedules shutdown before lookup but after the spawn — `lookup()` itself
// rejects via Defense A.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn graceful_shutdown_blocks_in_flight_acquire() {
    // 200ms create delay gives us a wide in-flight window. The drain
    // timeout is set far higher (2s) so a healthy fix waits for the
    // acquire instead of timing out.
    let create_delay = Duration::from_millis(200);
    let resource = SlowCreateResource::new(create_delay);
    let create_counter = Arc::clone(&resource.create_count);

    let manager = Arc::new(Manager::new());
    let resident_rt =
        ResidentRuntime::<SlowCreateResource>::new(resident::config::Config::default());

    manager
        .register(
            resource,
            SlowConfig,
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
            None,
            None,
        )
        .expect("register succeeds");

    // Spawn the acquire — it will spend ~200ms inside `create()`. The task
    // drops the guard immediately on success so `wait_for_drain` can return.
    // Returns the topology tag of the guard (or the error) so the assertion
    // logic stays at the surface of the test.
    let mgr = Arc::clone(&manager);
    let acquire_handle = tokio::spawn(async move {
        let ctx = test_ctx();
        let result = mgr
            .acquire_resident::<SlowCreateResource>(&(), &ctx, &AcquireOptions::default())
            .await;
        // Drop the guard inline so the in-flight slot is released — otherwise
        // `wait_for_drain` would block until `drain_timeout` because the guard
        // stays alive in this test task. The PRE-FIX bug was that the slot
        // wasn't even counted; with the fix, slot is counted AND released
        // when this guard drops.
        result.map(|guard| guard.topology_tag())
    });

    // Yield + small sleep so the acquire enters `create()` (past `lookup()`,
    // before the future resolves). 30ms is well inside the 200ms delay but
    // long enough for `tokio::spawn` to have scheduled the task and for it
    // to have gotten past `lookup()` + `InFlightCounter::new`.
    tokio::time::sleep(Duration::from_millis(30)).await;

    // Trigger graceful shutdown with a generous drain timeout (2s) so the
    // expected behaviour is "wait for the in-flight acquire to drain"
    // rather than "abort with DrainTimeout".
    let shutdown_started = Instant::now();
    let shutdown_result = manager
        .graceful_shutdown(ShutdownConfig::default().with_drain_timeout(Duration::from_secs(2)))
        .await;
    let shutdown_elapsed = shutdown_started.elapsed();

    let acquire_result = acquire_handle.await.expect("acquire task did not panic");

    match acquire_result {
        Ok(tag) => {
            // The acquire's `create()` completed before shutdown finished.
            // Defense B: shutdown must have waited for the in-flight slot,
            // and Defense B again on guard Drop released the slot so the
            // drain finished cleanly.
            assert!(
                shutdown_result.is_ok(),
                "shutdown should have waited for in-flight acquire instead of returning {shutdown_result:?}"
            );
            // The guard was real — registry was NOT cleared while it was in flight.
            assert_eq!(tag, TopologyTag::Resident);
            // `create()` ran exactly once before the registry cleared.
            assert_eq!(
                create_counter.load(Ordering::Relaxed),
                1,
                "create() should have completed exactly once",
            );

            // Critical discriminator: pre-fix, `wait_for_drain` saw `0`
            // because the in-flight acquire was not counted, so shutdown
            // returned in microseconds. Post-fix, `wait_for_drain` blocks
            // for the in-flight slot — by the time we sleep 30ms before
            // calling shutdown, ~170ms of the 200ms create delay remain.
            // We use a conservative 100ms threshold to allow scheduler
            // jitter on slow CI while still catching the race regression
            // (where elapsed would be < 10ms).
            let create_remaining = create_delay
                .saturating_sub(Duration::from_millis(30))
                .saturating_sub(Duration::from_millis(50)); // jitter buffer
            assert!(
                shutdown_elapsed >= create_remaining,
                "shutdown must wait for in-flight acquire to complete; \
                 elapsed={shutdown_elapsed:?} expected_at_least={create_remaining:?} \
                 (a quick return means the in-flight slot was not tracked — race regression)",
            );
        },
        Err(e) if matches!(e.kind(), ErrorKind::Cancelled) => {
            // Defense A: shutdown beat us into `lookup()`. The acquire
            // fast-failed via the `shutting_down` check. Either path is
            // race-safe.
            //
            // Note: this branch is unlikely with our 30ms head start but
            // tolerated for scheduler-jitter resilience on slow CI.
            assert!(
                shutdown_result.is_ok(),
                "shutdown should still succeed when acquire fast-fails, got {shutdown_result:?}"
            );
        },
        Err(other) => panic!("unexpected acquire error: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test: an acquire that arrives AFTER `graceful_shutdown` has set the flag
// must fast-fail at `lookup()` (Defense A), even if the cancel token has not
// yet propagated through every observer. Order of writes inside Phase 1:
// `shutting_down=true` (CAS line ~115) BEFORE `cancel.cancel()` (line ~128).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lookup_rejects_acquire_after_shutdown_starts() {
    let manager = Arc::new(Manager::new());
    let resident_rt =
        ResidentRuntime::<SlowCreateResource>::new(resident::config::Config::default());

    manager
        .register(
            SlowCreateResource::new(Duration::ZERO),
            SlowConfig,
            ScopeLevel::Global,
            TopologyRuntime::Resident(resident_rt),
            None,
            None,
            None,
            None,
        )
        .expect("register succeeds");

    // Run shutdown to completion first.
    manager
        .graceful_shutdown(ShutdownConfig::default().with_drain_timeout(Duration::from_millis(50)))
        .await
        .expect("shutdown succeeds with no in-flight handles");

    // Subsequent acquire must fast-fail with Cancelled (Defense A: the
    // `shutting_down` flag is observed inside `lookup()` even after the
    // cancel token has fired).
    let ctx = test_ctx();
    let result = manager
        .acquire_resident::<SlowCreateResource>(&(), &ctx, &AcquireOptions::default())
        .await;

    match result {
        Err(e) if matches!(e.kind(), ErrorKind::Cancelled) => {},
        Err(other) => panic!("expected Cancelled, got {other:?}"),
        Ok(_) => panic!("acquire after graceful_shutdown must not succeed"),
    }
}
