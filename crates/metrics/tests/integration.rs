use std::sync::Arc;

use nebula_metrics::naming::{
    NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS, NEBULA_RESOURCE_CREATE_TOTAL,
};
use nebula_metrics::{MetricsRegistry, TelemetryAdapter};

#[test]
fn telemetry_adapter_resource_metrics_round_trip_via_generic_accessors() {
    let registry = Arc::new(MetricsRegistry::new());
    let adapter = TelemetryAdapter::new(Arc::clone(&registry));

    tracing::debug!(
        "incrementing {} via generic adapter",
        NEBULA_RESOURCE_CREATE_TOTAL
    );
    adapter.counter(NEBULA_RESOURCE_CREATE_TOTAL).inc();

    tracing::debug!(
        "observing {} via generic adapter with sample=0.5",
        NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS
    );
    adapter
        .histogram(NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS)
        .observe(0.5);

    let create_total = registry.counter(NEBULA_RESOURCE_CREATE_TOTAL).get();
    let acquire_wait = registry.histogram(NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS);

    assert_eq!(create_total, 1);
    assert_eq!(acquire_wait.count(), 1);
    assert!((acquire_wait.sum() - 0.5).abs() < f64::EPSILON);
}
