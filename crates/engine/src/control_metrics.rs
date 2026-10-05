//! Shared-registry telemetry for authoritative execution-owner decisions.

use nebula_execution::ExecutionControlOutcome;
use nebula_metrics::{
    MetricsRegistry, MetricsResult, NEBULA_EXECUTION_CONTROL_OBSERVATIONS_UNRECORDED_TOTAL,
    NEBULA_EXECUTION_CONTROL_OUTCOMES_TOTAL,
};
use nebula_storage_port::StorageBackendKind;

/// Record only a decision acknowledged by its owning aggregate.
///
/// The caller supplies the deployment's shared registry and owning backend.
/// This observation never substitutes for the persisted journal decision.
pub(crate) fn record_execution_control_outcome(
    metrics: &MetricsRegistry,
    backend: StorageBackendKind,
    outcome: ExecutionControlOutcome,
) -> MetricsResult<()> {
    let span = tracing::Span::current();
    span.record("backend", backend.as_str());
    span.record("outcome", outcome.as_str());
    let labels = metrics
        .interner()
        .label_set(&[("backend", backend.as_str()), ("outcome", outcome.as_str())]);
    metrics
        .counter_labeled(NEBULA_EXECUTION_CONTROL_OUTCOMES_TOTAL, &labels)?
        .inc();
    Ok(())
}

/// Make a decision whose durable observation is missing visible: a counter on
/// the same closed labels plus a warning. The decision itself is unaffected.
pub(crate) fn record_unrecorded_execution_control_outcome(
    metrics: &MetricsRegistry,
    backend: StorageBackendKind,
    outcome: ExecutionControlOutcome,
    cause: &'static str,
) {
    tracing::warn!(
        backend = backend.as_str(),
        outcome = outcome.as_str(),
        cause,
        "execution-control decision stands but its journal observation is missing"
    );
    let labels = metrics
        .interner()
        .label_set(&[("backend", backend.as_str()), ("outcome", outcome.as_str())]);
    match metrics.counter_labeled(
        NEBULA_EXECUTION_CONTROL_OBSERVATIONS_UNRECORDED_TOTAL,
        &labels,
    ) {
        Ok(counter) => counter.inc(),
        Err(error) => {
            tracing::warn!(%error, "unrecorded-observation metric could not be recorded");
        },
    }
}

/// The observation write failed and was rolled back.
pub(crate) const CAUSE_WRITE_FAILED: &str = "write_failed";
/// The observation's commit acknowledgement was lost.
pub(crate) const CAUSE_ACKNOWLEDGEMENT_LOST: &str = "acknowledgement_lost";
/// The owner could not be reached to decide or record the refusal.
pub(crate) const CAUSE_OWNER_UNAVAILABLE: &str = "owner_unavailable";

/// One closed cause vocabulary for a storage error met while recording a
/// decision, shared by every caller.
pub(crate) fn unrecorded_cause(error: &nebula_storage_port::StorageError) -> &'static str {
    if matches!(
        error,
        nebula_storage_port::StorageError::AcknowledgementUnknown { .. }
    ) {
        CAUSE_ACKNOWLEDGEMENT_LOST
    } else {
        CAUSE_OWNER_UNAVAILABLE
    }
}

/// Count a decision by whether its observation is durable: a newly recorded
/// one on the outcome counter, a missing one on the unrecorded counter, and
/// an already-recorded replay on neither.
pub(crate) fn observe_execution_control_decision(
    metrics: &MetricsRegistry,
    backend: StorageBackendKind,
    outcome: ExecutionControlOutcome,
    acknowledgement: nebula_storage_port::store::ControlObservationAcknowledgement,
) {
    use nebula_storage_port::store::ControlObservationAcknowledgement as Ack;
    match acknowledgement {
        Ack::Recorded => {
            if let Err(error) = record_execution_control_outcome(metrics, backend, outcome) {
                tracing::warn!(%error, "execution-control outcome metric could not be recorded");
            }
        },
        Ack::AlreadyRecorded => {},
        Ack::Unrecorded => {
            record_unrecorded_execution_control_outcome(
                metrics,
                backend,
                outcome,
                CAUSE_WRITE_FAILED,
            );
        },
        Ack::Unknown => {
            record_unrecorded_execution_control_outcome(
                metrics,
                backend,
                outcome,
                CAUSE_ACKNOWLEDGEMENT_LOST,
            );
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_export_through_the_supplied_registry_with_closed_labels() {
        let metrics = MetricsRegistry::new();
        let backends = [
            StorageBackendKind::InMemory,
            StorageBackendKind::Sqlite,
            StorageBackendKind::Postgres,
        ];
        let outcomes = [
            ExecutionControlOutcome::Accepted,
            ExecutionControlOutcome::Fenced,
            ExecutionControlOutcome::Deferred,
            ExecutionControlOutcome::Throttled,
            ExecutionControlOutcome::Recovered,
            ExecutionControlOutcome::FlavorMismatch,
        ];
        for backend in backends {
            for outcome in outcomes {
                record_execution_control_outcome(&metrics, backend, outcome).unwrap();
                record_execution_control_outcome(&metrics, backend, outcome).unwrap();
                let labels = metrics
                    .interner()
                    .label_set(&[("backend", backend.as_str()), ("outcome", outcome.as_str())]);
                assert_eq!(
                    metrics
                        .counter_labeled(NEBULA_EXECUTION_CONTROL_OUTCOMES_TOTAL, &labels)
                        .unwrap()
                        .get(),
                    2
                );
            }
        }
        assert_eq!(metrics.snapshot_counters().len(), 18);
        let wire = nebula_metrics::snapshot(&metrics);
        assert!(wire.contains("outcome=\"flavor-mismatch\""));
        assert!(wire.contains("backend=\"in_memory\""));
        assert!(!wire.contains("execution_id="));
    }
}
