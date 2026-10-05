//! Shared-registry telemetry for authoritative execution-owner decisions.

use nebula_execution::ExecutionControlOutcome;
use nebula_metrics::{MetricsRegistry, MetricsResult, NEBULA_EXECUTION_CONTROL_OUTCOMES_TOTAL};
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

    #[test]
    fn incompatible_shared_registration_is_reported_without_fallback() {
        let metrics = MetricsRegistry::new();
        metrics
            .gauge(NEBULA_EXECUTION_CONTROL_OUTCOMES_TOTAL)
            .unwrap();
        assert!(
            record_execution_control_outcome(
                &metrics,
                StorageBackendKind::Sqlite,
                ExecutionControlOutcome::Accepted,
            )
            .is_err()
        );
        assert!(metrics.snapshot_counters().is_empty());
    }
}
