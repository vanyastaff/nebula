use std::sync::Arc;

use nebula_metrics::naming::{
    NEBULA_ACTION_EXECUTIONS_TOTAL, NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
    NEBULA_RESOURCE_CREATE_TOTAL,
};
use nebula_metrics::{LabelAllowlist, MetricsRegistry, snapshot};

#[test]
fn resource_metrics_round_trip_via_registry() {
    let registry = Arc::new(MetricsRegistry::new());

    registry.counter(NEBULA_RESOURCE_CREATE_TOTAL).inc();
    registry
        .histogram(NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS)
        .observe(0.5);

    assert_eq!(registry.counter(NEBULA_RESOURCE_CREATE_TOTAL).get(), 1);
    let hist = registry.histogram(NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS);
    assert_eq!(hist.count(), 1);
    assert!((hist.sum() - 0.5).abs() < f64::EPSILON);
}

#[test]
fn labeled_metrics_round_trip_to_prometheus_export() {
    let registry = Arc::new(MetricsRegistry::new());
    let allowlist = LabelAllowlist::only(["action_type"]);

    let raw = registry.interner().label_set(&[
        ("action_type", "http.request"),
        ("execution_id", "550e8400-e29b-41d4-a716-446655440000"),
    ]);
    let safe = allowlist.apply(&raw, registry.interner());
    assert_eq!(safe.len(), 1);

    registry
        .counter_labeled(NEBULA_ACTION_EXECUTIONS_TOTAL, &safe)
        .inc_by(10);

    let out = snapshot(&registry);
    assert!(
        out.contains(r#"action_type="http.request""#),
        "allowed label present:\n{out}"
    );
    assert!(
        !out.contains("execution_id"),
        "filtered key absent:\n{out}"
    );
}

#[test]
fn mixed_labeled_and_unlabeled_metrics_export() {
    let registry = Arc::new(MetricsRegistry::new());
    let labels = registry
        .interner()
        .label_set(&[("action_type", "http.request")]);

    registry
        .counter(NEBULA_ACTION_EXECUTIONS_TOTAL)
        .inc_by(5);
    registry
        .counter_labeled(NEBULA_ACTION_EXECUTIONS_TOTAL, &labels)
        .inc_by(10);

    let out = snapshot(&registry);

    let type_count = out
        .matches("# TYPE nebula_action_executions_total counter")
        .count();
    assert_eq!(type_count, 1, "single TYPE header:\n{out}");

    assert!(
        out.contains("nebula_action_executions_total 5\n"),
        "unlabeled line:\n{out}"
    );
    assert!(
        out.contains(r#"nebula_action_executions_total{action_type="http.request"} 10"#),
        "labeled line:\n{out}"
    );
}
