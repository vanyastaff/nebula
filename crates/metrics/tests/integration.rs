use nebula_metrics::{
    MetricsRegistry,
    naming::{NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS, NEBULA_RESOURCE_CREATE_TOTAL},
};

#[test]
fn registry_records_resource_metrics_via_canonical_names() {
    let registry = MetricsRegistry::new();

    tracing::debug!("incrementing {NEBULA_RESOURCE_CREATE_TOTAL} via registry");
    registry
        .counter(NEBULA_RESOURCE_CREATE_TOTAL)
        .unwrap()
        .inc();

    tracing::debug!(
        "observing {NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS} via registry with sample=0.5"
    );
    registry
        .histogram(NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS)
        .unwrap()
        .observe(0.5);

    let create_total = registry
        .counter(NEBULA_RESOURCE_CREATE_TOTAL)
        .unwrap()
        .get();
    let acquire_wait = registry
        .histogram(NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS)
        .unwrap();

    assert_eq!(create_total, 1);
    assert_eq!(acquire_wait.count(), 1);
    assert!((acquire_wait.sum() - 0.5).abs() < f64::EPSILON);
}
