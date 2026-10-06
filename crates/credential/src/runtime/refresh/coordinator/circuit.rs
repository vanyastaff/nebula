//! Private process-local transport circuit for admitted refresh commands.
//!
//! [`RefreshDispatchCircuit`] is an advisory, per-process gate bound to one
//! tenant-qualified credential selector. It observes only the actual provider
//! transport outcome — never durable settlement — and refuses dispatch with an
//! exact before-dispatch [`RefreshNotAppliedContext`] while open.
use super::super::{l1::L1RefreshCoalescer, metrics::RefreshCircuitMetrics};
use crate::{
    RefreshDiagnosticCode, RefreshErrorKind, RefreshFailureSpec, RefreshNotAppliedContext,
    RefreshNotAppliedPhase, RetryAdvice, RetryDelay,
};
use nebula_resilience::circuit_breaker::{Admission, CircuitBreaker, Outcome};
use std::{fmt, sync::Arc, time::Duration};

/// Transport circuit for one refresh attempt against one tenant-qualified
/// credential. Cloning shares the same single outstanding probe.
#[derive(Clone)]
pub(crate) struct RefreshDispatchCircuit {
    inner: Arc<CircuitHandle>,
}
struct CircuitHandle {
    l1: Arc<L1RefreshCoalescer>,
    key: String,
    probe_retry_backoff: Duration,
    metrics: RefreshCircuitMetrics,
    probe: parking_lot::Mutex<Option<DispatchProbe>>,
}
/// One admitted dispatch, settled exactly once (explicitly or on drop).
struct DispatchProbe {
    l1: Arc<L1RefreshCoalescer>,
    key: String,
    breaker: Arc<CircuitBreaker>,
    metrics: RefreshCircuitMetrics,
    /// Epoch-bound breaker permit. Outcomes are applied only while the
    /// breaker is still in the epoch this dispatch was admitted in, so a late
    /// closed-state result can neither release nor settle a later half-open
    /// probe. `None` once settled: taking it is what makes settlement
    /// happen at most once.
    admission: Option<Admission>,
    provider_started: bool,
}
impl Drop for DispatchProbe {
    fn drop(&mut self) {
        let Some(admission) = self.admission.take() else {
            return;
        };
        if self.provider_started {
            L1RefreshCoalescer::record_dispatch_failure(&self.breaker, admission);
            self.metrics.transport_cancelled.inc();
            tracing::warn!(
                circuit_outcome = "transport_cancelled",
                "refresh circuit observed interrupted provider transport"
            );
        } else {
            // No provider operation began. Cancellation never counts as a
            // failure; it releases a half-open slot only if this admission
            // reserved one in the breaker's current epoch.
            self.breaker
                .record_admitted_outcome(admission, Outcome::Cancelled);
            self.metrics.admission_cancelled.inc();
            tracing::debug!(
                circuit_outcome = "admission_cancelled",
                "refresh circuit released an unstarted provider probe"
            );
        }
    }
}
impl fmt::Debug for RefreshDispatchCircuit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefreshDispatchCircuit")
            .finish_non_exhaustive()
    }
}
impl RefreshDispatchCircuit {
    pub(super) fn new(
        l1: Arc<L1RefreshCoalescer>,
        key: String,
        probe_retry_backoff: Duration,
        metrics: RefreshCircuitMetrics,
    ) -> Self {
        Self {
            inner: Arc::new(CircuitHandle {
                l1,
                key,
                // RetryDelay is whole-second and rejects zero. A zero policy
                // selects its smallest representable contention delay; this
                // floor is not reported as an open circuit cooldown.
                probe_retry_backoff: probe_retry_backoff.max(Duration::from_secs(1)),
                metrics,
                probe: parking_lot::Mutex::new(None),
            }),
        }
    }
    /// Reserve the provider dispatch, or refuse with exact before-dispatch
    /// advice: the measured cooldown while open, the probe backoff while busy.
    pub(crate) fn check_before_dispatch(&self) -> Result<(), Box<RefreshNotAppliedContext>> {
        let mut probe = self.inner.probe.lock();
        if probe.is_some() {
            self.inner.metrics.probe_busy.inc();
            return Err(Box::new(refusal(
                "refresh.circuit_probe_busy",
                self.inner.probe_retry_backoff,
            )));
        }
        let breaker = self.inner.l1.dispatch_breaker(&self.inner.key);
        let Ok(admission) = breaker.try_admit::<()>() else {
            let (code, delay) = match breaker.remaining_open_duration() {
                Some(remaining) if !remaining.is_zero() => {
                    self.inner.metrics.open.inc();
                    ("refresh.circuit_open", remaining)
                },
                _ => {
                    self.inner.metrics.probe_busy.inc();
                    ("refresh.circuit_probe_busy", self.inner.probe_retry_backoff)
                },
            };
            tracing::debug!(
                circuit_outcome = code,
                "refresh circuit refused provider dispatch"
            );
            return Err(Box::new(refusal(code, delay)));
        };
        *probe = Some(DispatchProbe {
            l1: self.inner.l1.clone(),
            key: self.inner.key.clone(),
            breaker,
            metrics: self.inner.metrics.clone(),
            admission: Some(admission),
            provider_started: false,
        });
        self.inner.metrics.admitted.inc();
        tracing::debug!(
            circuit_outcome = "admitted",
            "refresh circuit admitted provider probe"
        );
        Ok(())
    }
    /// Mark that the provider operation began; an unsettled drop now counts
    /// as an interrupted transport rather than an abandoned admission.
    pub(crate) fn provider_started(&self) {
        if let Some(probe) = self.inner.probe.lock().as_mut() {
            probe.provider_started = true;
        }
    }
    /// Settle the probe with a completed provider transport response.
    pub(crate) fn record_success(&self) {
        if let Some(mut probe) = self.inner.probe.lock().take()
            && let Some(admission) = probe.admission.take()
        {
            probe
                .l1
                .record_dispatch_success(&probe.key, &probe.breaker, admission);
            probe.metrics.transport_success.inc();
            tracing::debug!(
                circuit_outcome = "transport_success",
                "refresh circuit observed completed provider transport"
            );
        }
    }
    /// Settle the probe with a failed provider transport.
    pub(crate) fn record_failure(&self) {
        if let Some(mut probe) = self.inner.probe.lock().take()
            && let Some(admission) = probe.admission.take()
        {
            L1RefreshCoalescer::record_dispatch_failure(&probe.breaker, admission);
            probe.metrics.transport_failure.inc();
            tracing::debug!(
                circuit_outcome = "transport_failure",
                "refresh circuit observed failed provider transport"
            );
        }
    }
}
/// Before-dispatch refusal carrying the circuit's whole-second retry advice.
fn refusal(code: &'static str, delay: Duration) -> RefreshNotAppliedContext {
    let retry = if let Ok(delay) = RetryDelay::new(delay) {
        RetryAdvice::After(delay)
    } else {
        // Registry policy admission bounds non-zero backoff through the
        // same RetryDelay constructor; measured cooldown is at most five
        // minutes. Reaching this branch violates that runtime invariant.
        tracing::error!(
            invariant = "refresh.circuit_retry_delay_invalid",
            "admitted refresh circuit delay violated its validated bound"
        );
        RetryAdvice::Never
    };
    let failure = RefreshFailureSpec::new(RefreshErrorKind::ProviderUnavailable, retry);
    let failure = match RefreshDiagnosticCode::parse(code) {
        Ok(code) => failure.with_diagnostic_code(code),
        Err(_) => failure,
    };
    RefreshNotAppliedContext::from_spec(RefreshNotAppliedPhase::BeforeDispatch, failure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_resilience::CircuitState;

    #[test]
    fn actual_transport_failures_open_only_their_own_gate() {
        let l1 = Arc::new(L1RefreshCoalescer::new());
        for _ in 0..5 {
            let gate = RefreshDispatchCircuit::new(
                l1.clone(),
                "tenant-qualified-credential".to_owned(),
                Duration::from_secs(1),
                RefreshCircuitMetrics::with_registry(&nebula_metrics::MetricsRegistry::new())
                    .expect("metrics"),
            );
            gate.check_before_dispatch().expect("closed circuit");
            gate.provider_started();
            gate.record_failure();
        }
        let gate = RefreshDispatchCircuit::new(
            l1.clone(),
            "tenant-qualified-credential".to_owned(),
            Duration::from_secs(1),
            RefreshCircuitMetrics::with_registry(&nebula_metrics::MetricsRegistry::new())
                .expect("metrics"),
        );
        let refusal = gate.check_before_dispatch().expect_err("open circuit");
        assert_eq!(refusal.phase(), RefreshNotAppliedPhase::BeforeDispatch);
        assert!(matches!(refusal.retry(), RetryAdvice::After(_)));
        assert!(l1.is_circuit_open("tenant-qualified-credential"));
        let other = RefreshDispatchCircuit::new(
            l1.clone(),
            "different-tenant-same-credential".to_owned(),
            Duration::from_secs(1),
            RefreshCircuitMetrics::with_registry(&nebula_metrics::MetricsRegistry::new())
                .expect("metrics"),
        );
        other.check_before_dispatch().expect("separate tenant");
        other.provider_started();
        other.record_success();
        assert!(l1.is_circuit_open("tenant-qualified-credential"));
    }

    #[test]
    fn admission_cancellation_never_counts_as_a_provider_failure() {
        let l1 = Arc::new(L1RefreshCoalescer::new());
        for _ in 0..10 {
            let gate = RefreshDispatchCircuit::new(
                l1.clone(),
                "credential".to_owned(),
                Duration::from_secs(1),
                RefreshCircuitMetrics::with_registry(&nebula_metrics::MetricsRegistry::new())
                    .expect("metrics"),
            );
            gate.check_before_dispatch().expect("closed circuit");
            let metrics = gate.inner.metrics.clone();
            drop(gate);
            assert_eq!(metrics.admission_cancelled.get(), 1);
            assert_eq!(metrics.transport_cancelled.get(), 0);
        }
        assert!(!l1.is_circuit_open("credential"));
        for _ in 0..5 {
            let gate = RefreshDispatchCircuit::new(
                l1.clone(),
                "credential".to_owned(),
                Duration::from_secs(1),
                RefreshCircuitMetrics::with_registry(&nebula_metrics::MetricsRegistry::new())
                    .expect("metrics"),
            );
            gate.check_before_dispatch().expect("closed circuit");
            gate.provider_started();
            let metrics = gate.inner.metrics.clone();
            drop(gate);
            assert_eq!(metrics.transport_cancelled.get(), 1);
            assert_eq!(metrics.admission_cancelled.get(), 0);
        }
        assert!(l1.is_circuit_open("credential"));
    }
    #[test]
    fn evicted_probe_outcomes_never_mutate_a_recreated_breaker() {
        for success in [false, true] {
            let l1 = Arc::new(L1RefreshCoalescer::new());
            let old = RefreshDispatchCircuit::new(
                l1.clone(),
                "credential".to_owned(),
                Duration::from_secs(1),
                RefreshCircuitMetrics::with_registry(&nebula_metrics::MetricsRegistry::new())
                    .expect("metrics"),
            );
            old.check_before_dispatch().expect("old probe");
            old.provider_started();
            for index in 0..6000 {
                let _ = l1.dispatch_breaker(&format!("other-{index}"));
            }
            let replacement = l1.dispatch_breaker("credential");
            if success {
                old.record_success();
            } else {
                old.record_failure();
            }
            assert_eq!(replacement.stats().failures, 0);
            assert!(Arc::ptr_eq(
                &replacement,
                &l1.dispatch_breaker("credential")
            ));
        }
    }
    #[test]
    fn completed_half_open_probe_does_not_release_its_sibling() {
        use nebula_resilience::{circuit_breaker::CircuitBreakerConfig, clock::MockInstant};
        let clock = Arc::new(MockInstant::new());
        let breaker = Arc::new(
            CircuitBreaker::new(CircuitBreakerConfig {
                reset_timeout: Duration::from_millis(100),
                max_half_open_operations: 2,
                half_open_success_threshold: Some(2),
                ..CircuitBreakerConfig::default()
            })
            .expect("config")
            .with_instant_source(clock.clone()),
        );
        breaker.force_open();
        clock.advance(Duration::from_millis(100));
        let first = breaker.try_admit::<()>().expect("first probe");
        let _sibling = breaker.try_admit::<()>().expect("sibling probe");
        let metrics = RefreshCircuitMetrics::with_registry(&nebula_metrics::MetricsRegistry::new())
            .expect("metrics");
        let l1 = Arc::new(L1RefreshCoalescer::new());
        let gate = RefreshDispatchCircuit::new(
            l1.clone(),
            "credential".to_owned(),
            Duration::from_secs(1),
            metrics.clone(),
        );
        *gate.inner.probe.lock() = Some(DispatchProbe {
            l1,
            key: "credential".to_owned(),
            breaker: breaker.clone(),
            metrics: metrics.clone(),
            admission: Some(first),
            provider_started: true,
        });
        gate.record_success();
        breaker
            .try_acquire::<()>()
            .expect("only completed slot is free");
        assert!(
            breaker.try_acquire::<()>().is_err(),
            "sibling still occupies its slot"
        );
        assert_eq!(metrics.transport_success.get(), 1);
        assert_eq!(metrics.admission_cancelled.get(), 0);
        assert_eq!(metrics.transport_cancelled.get(), 0);
    }

    #[test]
    fn abandoned_closed_admission_does_not_release_a_later_half_open_probe() {
        use nebula_resilience::{circuit_breaker::CircuitBreakerConfig, clock::MockInstant};
        let clock = Arc::new(MockInstant::new());
        let breaker = Arc::new(
            CircuitBreaker::new(CircuitBreakerConfig {
                reset_timeout: Duration::from_millis(100),
                max_half_open_operations: 1,
                ..CircuitBreakerConfig::default()
            })
            .expect("config")
            .with_instant_source(clock.clone()),
        );
        let l1 = Arc::new(L1RefreshCoalescer::new());
        l1.install_dispatch_breaker("credential", breaker.clone());
        let gate = |l1: &Arc<L1RefreshCoalescer>| {
            RefreshDispatchCircuit::new(
                l1.clone(),
                "credential".to_owned(),
                Duration::from_secs(1),
                RefreshCircuitMetrics::with_registry(&nebula_metrics::MetricsRegistry::new())
                    .expect("metrics"),
            )
        };

        // Admitted while closed: no half-open slot is reserved.
        let closed_admission = gate(&l1);
        closed_admission
            .check_before_dispatch()
            .expect("closed circuit admits");
        breaker.force_open();
        clock.advance(Duration::from_millis(100));
        let half_open_probe = gate(&l1);
        half_open_probe
            .check_before_dispatch()
            .expect("elapsed cooldown admits the single half-open probe");

        // Abandoning the closed-state admission must not free the probe slot.
        drop(closed_admission);
        let refusal = gate(&l1)
            .check_before_dispatch()
            .expect_err("the half-open probe still occupies the only slot");
        assert_eq!(refusal.phase(), RefreshNotAppliedPhase::BeforeDispatch);
        assert_eq!(breaker.circuit_state(), CircuitState::HalfOpen);

        // Abandoning the half-open probe itself does release its slot.
        drop(half_open_probe);
        gate(&l1)
            .check_before_dispatch()
            .expect("released half-open slot admits a new probe");
    }

    #[test]
    fn late_closed_epoch_transport_cannot_settle_the_half_open_probe() {
        use nebula_resilience::{circuit_breaker::CircuitBreakerConfig, clock::MockInstant};
        for success in [true, false] {
            let clock = Arc::new(MockInstant::new());
            let breaker = Arc::new(
                CircuitBreaker::new(CircuitBreakerConfig {
                    reset_timeout: Duration::from_millis(100),
                    max_half_open_operations: 1,
                    ..CircuitBreakerConfig::default()
                })
                .expect("config")
                .with_instant_source(clock.clone()),
            );
            let l1 = Arc::new(L1RefreshCoalescer::new());
            l1.install_dispatch_breaker("credential", breaker.clone());
            let gate = |l1: &Arc<L1RefreshCoalescer>| {
                RefreshDispatchCircuit::new(
                    l1.clone(),
                    "credential".to_owned(),
                    Duration::from_secs(1),
                    RefreshCircuitMetrics::with_registry(&nebula_metrics::MetricsRegistry::new())
                        .expect("metrics"),
                )
            };

            // Dispatched while closed; its provider call is still in flight
            // when the circuit opens and the single recovery probe starts.
            let stale = gate(&l1);
            stale
                .check_before_dispatch()
                .expect("closed circuit admits");
            stale.provider_started();
            breaker.force_open();
            clock.advance(Duration::from_millis(100));
            let probe = gate(&l1);
            probe
                .check_before_dispatch()
                .expect("single half-open probe");
            probe.provider_started();

            if success {
                stale.record_success();
            } else {
                stale.record_failure();
            }
            assert_eq!(
                breaker.circuit_state(),
                CircuitState::HalfOpen,
                "pre-open evidence must not settle the recovery round"
            );
            assert!(
                Arc::ptr_eq(&breaker, &l1.dispatch_breaker("credential")),
                "a stale success must not discard the recovering breaker"
            );
            gate(&l1)
                .check_before_dispatch()
                .expect_err("the probe still owns the only half-open slot");

            probe.record_success();
            assert!(!l1.is_circuit_open("credential"));
        }
    }

    #[test]
    fn fractional_measured_open_cooldown_remains_after_one_second() {
        let context = refusal("refresh.circuit_open", Duration::from_millis(1));
        let RetryAdvice::After(delay) = context.retry() else {
            panic!("fraction is retryable");
        };
        assert_eq!(delay.get(), Duration::from_secs(1));
        assert_eq!(context.phase(), RefreshNotAppliedPhase::BeforeDispatch);
    }
}
