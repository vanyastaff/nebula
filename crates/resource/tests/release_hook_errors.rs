//! Release-hook error handling — characterization of the currently-swallowed
//! `reset` / `close_session` / `release_token` failures.
//!
//! Today every topology release helper discards the hook result with
//! `let _ = resource.<hook>(...).await;` (see `runtime/exclusive.rs`,
//! `runtime/service.rs`, `runtime/transport.rs`). A failed `reset` therefore
//! silently hands the lock to the next caller as if the previous lease was
//! cleanly reset — a half-reset instance can be served to the next acquirer.
//!
//! The contract the unified release path must enforce (S4):
//! - On `reset` `Err`, the permit IS still returned (withholding it would
//!   deadlock the semaphore — it lives in the handle, outside the callback,
//!   for exactly this reason) BUT the instance is **destroyed**, never
//!   recycled or handed onward; the next acquirer gets a freshly built
//!   instance, not the failed-reset one.
//!
//! What is asserted here:
//! - **RED (proves R17):** a failed `reset` destroys the instance instead of
//!   silently handing it onward. `Resource::destroy` is never invoked on the
//!   failed-reset runtime today, so this fails until the unified release path
//!   lands — marked `#[ignore]` with a reason so the suite stays green while
//!   the defect is recorded.
//! - **GREEN preserve:** a failed `reset` must still return the permit so the
//!   next acquire does not deadlock. S4 explicitly preserves this; it must
//!   not regress when the destroy-on-failure behavior is added.
//!
//! The matching `release_token` / `close_session` "observed, not swallowed"
//! assertions key off the release-error *metric* the unified path adds. That
//! metric does not exist against the current API, so those assertions live
//! with the unit that introduces it, not here — pinning them in this file
//! would require referencing a not-yet-existing symbol (an `#[ignore]`d test
//! must still compile).

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_core::{ResourceKey, resource_key};
use nebula_resource::{
    AcquireOptions, ExclusiveConfig, ExclusiveRuntime, Resource, ResourceConfig, ResourceContext,
    error::Error, release_queue::ReleaseQueue, resource::ResourceMetadata,
};

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

/// An exclusive resource whose `reset` always fails and which counts every
/// `reset` attempt and every `destroy` invocation. `destroy` being called on
/// the failed-reset runtime is the post-fix observable (S4: the half-reset
/// instance is destroyed, not handed onward).
#[derive(Clone)]
struct FailingResetExclusive {
    reset_attempts: Arc<AtomicUsize>,
    destroy_calls: Arc<AtomicUsize>,
}

impl FailingResetExclusive {
    fn new() -> Self {
        Self {
            reset_attempts: Arc::new(AtomicUsize::new(0)),
            destroy_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Resource for FailingResetExclusive {
    type Config = R17Config;
    type Runtime = u32;
    type Lease = u32;
    type Error = R17Error;

    fn key() -> ResourceKey {
        resource_key!("r17-failing-reset")
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

impl nebula_resource::topology::exclusive::Exclusive for FailingResetExclusive {
    async fn reset(&self, _runtime: &u32) -> Result<(), R17Error> {
        self.reset_attempts.fetch_add(1, Ordering::SeqCst);
        Err(R17Error("reset failed".to_owned()))
    }
}

/// RED — proves R17. A failed `reset` must NOT be silently treated as a
/// successful release: the half-reset instance must be destroyed rather than
/// handed to the next acquirer. Today `release_exclusive` does
/// `let _ = resource.reset(&runtime).await;` and never calls `destroy`, so
/// `destroy_calls` stays `0` — the next caller can be served the half-reset
/// instance. Passes once the unified release path destroys the instance on a
/// reset error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "RED until U3 — proves R17 (failed reset must destroy the instance, not silently hand it onward)"]
async fn exclusive_failed_reset_destroys_instance_not_handed_onward() {
    let resource = FailingResetExclusive::new();
    let config = ExclusiveConfig {
        acquire_timeout: Duration::from_secs(5),
    };
    let runtime: ExclusiveRuntime<FailingResetExclusive> = ExclusiveRuntime::new(1u32, config);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let handle = runtime
        .acquire(&resource, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("first acquire must succeed");

    // Drop the lease → `reset` runs on the release queue and fails.
    drop(handle);

    // The reset attempt must have happened, and on its failure the instance
    // must be destroyed (S4) rather than silently recycled.
    let reset_ran = poll_until(Duration::from_secs(2), || {
        resource.reset_attempts.load(Ordering::SeqCst) >= 1
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

/// GREEN preserve. S4 explicitly keeps the "permit is still returned on a
/// failed reset" behavior so the semaphore cannot deadlock. This is true
/// today (the permit lives in the handle, not the callback) and must remain
/// true after the destroy-on-failure behavior is added — adding the destroy
/// must not also start withholding the permit.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exclusive_failed_reset_still_returns_permit_no_deadlock() {
    let resource = FailingResetExclusive::new();
    let config = ExclusiveConfig {
        acquire_timeout: Duration::from_secs(5),
    };
    let runtime: Arc<ExclusiveRuntime<FailingResetExclusive>> =
        Arc::new(ExclusiveRuntime::new(1u32, config));
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);

    let h1 = runtime
        .acquire(&resource, &rq, 0, &AcquireOptions::default(), None)
        .await
        .expect("first acquire must succeed");
    drop(h1);

    // The second acquire must still succeed despite the reset failure: the
    // permit was returned (it is held in the handle, outside the failing
    // callback). A bounded deadline turns a regression into a fast failure
    // rather than a hang.
    let h2 = runtime
        .acquire(
            &resource,
            &rq,
            0,
            &AcquireOptions::default()
                .with_deadline(std::time::Instant::now() + Duration::from_secs(2)),
            None,
        )
        .await;
    assert!(
        h2.is_ok(),
        "a failed reset must still return the permit so the next acquire \
         does not deadlock (S4 preserve): {:?}",
        h2.err()
    );

    drop(h2);
    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;
}
