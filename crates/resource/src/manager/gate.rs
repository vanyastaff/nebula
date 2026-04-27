//! Recovery-gate admission machinery for the [`Manager`](super::Manager)
//! acquire path (#322).
//!
//! Splits the old "check_recovery_gate → trigger_recovery_on_failure" pair
//! into a single CAS-based pre-acquire admission with end-to-end ticket
//! ownership.

use std::{sync::Arc, time::Instant};

use crate::{
    error::Error,
    recovery::gate::{GateState, RecoveryGate, RecoveryTicket, TryBeginError},
};

/// Outcome of the pre-acquire gate admission check (#322).
///
/// The old `check_recovery_gate` → `trigger_recovery_on_failure` split was
/// a stampede hazard: after the backoff expired, every caller's `state()`
/// read returned the same snapshot and all of them proceeded through to
/// hit the dead backend. The gate was only advanced to `InProgress` *after*
/// failure, in `trigger_recovery_on_failure`, which is too late.
///
/// This enum pushes the CAS-based single-probe claim into the pre-acquire
/// check: either the caller is granted a ticket (`Probe`) that must be
/// resolved end-to-end with the acquire result, or the gate was idle/absent
/// (`Open`), or we got a typed error out directly.
pub(super) enum GateAdmission {
    /// No gate attached. Proceed normally; no ticket ownership.
    Open,
    /// Gate attached and currently healthy (`Idle`). Proceed without a
    /// ticket, but retain the gate so a retryable acquire error can mark it
    /// failed and open the backoff window for subsequent callers.
    OpenGated(Arc<RecoveryGate>),
    /// This caller has been granted the single recovery slot. The acquire
    /// **must** consume the ticket by calling `resolve()`, `fail_transient`,
    /// or `fail_permanent` based on the acquire result. Dropping it without
    /// resolution auto-fails to `GateState::Failed` via its `Drop` impl —
    /// so even a cancellation or panic in the acquire path is safe.
    Probe(RecoveryTicket),
}

/// Admits a caller through the optional recovery gate.
///
/// Healthy gates (`Idle`) admit immediately with no CAS so regular traffic
/// keeps full pool concurrency. Only callers entering while the gate is in a
/// retryable `Failed` state claim a probe ticket.
pub(super) fn admit_through_gate(gate: &Option<Arc<RecoveryGate>>) -> Result<GateAdmission, Error> {
    let Some(gate) = gate else {
        return Ok(GateAdmission::Open);
    };

    match gate.state() {
        GateState::Idle => Ok(GateAdmission::OpenGated(Arc::clone(gate))),
        GateState::InProgress { .. } => Err(Error::transient(
            "backend recovery in progress, retry later",
        )),
        GateState::Failed { retry_at, .. } => {
            if Instant::now() < retry_at {
                let wait = retry_at.saturating_duration_since(Instant::now());
                return Err(Error::exhausted("backend recovering", Some(wait)));
            }
            match gate.try_begin() {
                Ok(ticket) => Ok(GateAdmission::Probe(ticket)),
                Err(TryBeginError::AlreadyInProgress(_waiter)) => Err(Error::transient(
                    "backend recovery in progress, retry later",
                )),
                Err(TryBeginError::RetryLater { retry_at }) => {
                    let wait = retry_at.saturating_duration_since(Instant::now());
                    Err(Error::exhausted("backend recovering", Some(wait)))
                },
                Err(TryBeginError::PermanentlyFailed { message }) => Err(Error::permanent(message)),
            }
        },
        GateState::PermanentlyFailed { message } => Err(Error::permanent(message)),
    }
}

/// Resolves the ticket granted by [`admit_through_gate`] based on the
/// acquire result. No-op when the admission was [`GateAdmission::Open`]
/// (no gate attached), so callers can always call this unconditionally.
pub(super) fn settle_gate_admission<T>(admission: GateAdmission, result: &Result<T, Error>) {
    match (admission, result) {
        (GateAdmission::Probe(ticket), Ok(_)) => ticket.resolve(),
        (GateAdmission::Probe(ticket), Err(e)) if e.is_retryable() => {
            ticket.fail_transient(e.to_string());
        },
        (GateAdmission::Probe(ticket), Err(_e)) => {
            // Non-retryable errors are not backend-health signals; keep the
            // gate open to avoid permanently bricking acquires.
            ticket.resolve();
        },
        (GateAdmission::OpenGated(gate), Err(e)) if e.is_retryable() => {
            // First retryable failure on healthy path opens the backoff gate.
            if let Ok(ticket) = gate.try_begin() {
                ticket.fail_transient(e.to_string());
            }
        },
        (GateAdmission::OpenGated(_) | GateAdmission::Open, _) => {},
    }
}

#[cfg(test)]
mod gate_admission_tests {
    use std::time::Duration;

    use super::*;
    use crate::recovery::gate::RecoveryGateConfig;

    /// #322: after `Failed { retry_at = past }`, concurrent callers must
    /// see **exactly one** `Probe` ticket, not a stampede. The CAS-based
    /// single-probe claim lives in `admit_through_gate`. Each spawned
    /// task parks on a `Barrier` before calling so the 32 attempts
    /// really contend, instead of being serviced one at a time.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn expired_failed_state_admits_only_one_probe() {
        let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig {
            max_attempts: 16,
            base_backoff: Duration::from_millis(5),
        }));

        // Drive the gate into Failed { retry_at ≈ past }.
        let ticket = gate.try_begin().expect("first ticket");
        ticket.fail_transient("seed");
        tokio::time::sleep(Duration::from_millis(20)).await;

        async fn contend(
            gate: Arc<RecoveryGate>,
            barrier: Arc<tokio::sync::Barrier>,
        ) -> (u32, u32) {
            // Park here until every task is ready so we really stress
            // the CAS claim.
            barrier.wait().await;
            let some_gate: Option<Arc<RecoveryGate>> = Some(gate);
            match admit_through_gate(&some_gate) {
                Ok(GateAdmission::Probe(ticket)) => {
                    // Hold the probe until the test is done counting so
                    // a second caller can't race in after a fast
                    // resolve/fail cycle.
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    drop(ticket);
                    (1, 0)
                },
                Ok(GateAdmission::Open | GateAdmission::OpenGated(_)) => (0, 1),
                Err(_) => (0, 1),
            }
        }

        let barrier = Arc::new(tokio::sync::Barrier::new(32));
        let mut handles = Vec::with_capacity(32);
        for _ in 0..32 {
            handles.push(tokio::spawn(contend(
                Arc::clone(&gate),
                Arc::clone(&barrier),
            )));
        }

        let mut probes = 0u32;
        let mut blocked = 0u32;
        for h in handles {
            let (p, b) = h.await.expect("admission task");
            probes += p;
            blocked += b;
        }

        assert_eq!(probes, 1, "exactly one Probe ticket must be granted (#322)");
        assert_eq!(
            probes + blocked,
            32,
            "every call must be accounted for: probes={probes}, blocked={blocked}",
        );
    }
}
