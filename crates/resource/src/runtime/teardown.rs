//! Shared teardown primitives for the resource runtime.
//!
//! Provides the per-resource deadline computation and the bounded async
//! `destroy` dispatch that every topology path (acquire loop, release, warmup,
//! maintenance, resident create-vs-rotate) uses whenever it needs to tear an
//! instance down.
//!
//! Factored out of [`managed`](super::managed) so the `Resident` topology can
//! import `destroy_within` without pulling in the full
//! `ManagedResource<R>` acquire-loop machinery.

use std::time::{Duration, Instant};

use crate::{
    error::Error,
    resource::{Provider, TeardownCx, TeardownReason},
};

/// A `Revoked` teardown is urgent — a credential is no longer trustworthy, so
/// the framework will not wait the resource's full declared budget. The
/// composed deadline is capped at this for revoke, even if
/// [`Provider::teardown_budget`] is larger. See ADR-0093.
pub(crate) const REVOKE_TEARDOWN_CAP: Duration = Duration::from_secs(5);

/// Compose the teardown deadline: the resource's declared
/// [`teardown_budget`](Provider::teardown_budget), capped short for a revoke
/// (urgent). `now` is taken once at call time. See ADR-0093.
pub(crate) fn teardown_deadline<R: Provider>(resource: &R, reason: TeardownReason) -> Instant {
    let budget = resource.teardown_budget();
    let effective = if matches!(reason, TeardownReason::Revoked) {
        budget.min(REVOKE_TEARDOWN_CAP)
    } else {
        budget
    };
    Instant::now() + effective
}

/// Tear one instance down under a per-resource, per-context deadline.
///
/// Composes the deadline from [`teardown_deadline`], builds the read-only
/// [`TeardownCx`], and runs [`Provider::destroy`] under
/// [`tokio::time::timeout_at`]. On timeout the in-flight destroy future is
/// dropped (abandoned) and a typed [`Error::backpressure`] is returned so the
/// caller can record the abandoned teardown — the framework never blocks past
/// the deadline. An author doing graceful work bounds it to the same
/// `cx.deadline`, so the two deadlines coincide. See ADR-0093.
pub(crate) async fn destroy_within<R: Provider>(
    resource: &R,
    instance: R::Instance,
    reason: TeardownReason,
) -> Result<(), Error> {
    let deadline = teardown_deadline(resource, reason);
    let cx = TeardownCx::new(deadline, reason);
    match tokio::time::timeout_at(deadline.into(), resource.destroy(instance, cx)).await {
        Ok(res) => res,
        Err(_elapsed) => Err(Error::backpressure(format!(
            "{}: destroy exceeded teardown budget",
            R::key()
        ))),
    }
}
