use std::sync::Arc;

use nebula_metrics::naming::{ACTION_EXECUTIONS, RESOURCE_ACQUIRE_WAIT_DURATION, RESOURCE_CREATE};
use nebula_metrics::{LabelAllowlist, MetricsRegistry, snapshot};

#[test]
fn resource_metrics_round_trip_via_registry() {
    let registry = Arc::new(MetricsRegistry::new());

    registry.counter(RESOURCE_CREATE.as_str()).inc();
    registry
        .histogram(RESOURCE_ACQUIRE_WAIT_DURATION.as_str())
        .observe(0.5);

    assert_eq!(registry.counter(RESOURCE_CREATE.as_str()).get(), 1);
    let hist = registry.histogram(RESOURCE_ACQUIRE_WAIT_DURATION.as_str());
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
        .counter_labeled(ACTION_EXECUTIONS.as_str(), &safe)
        .inc_by(10);

    let out = snapshot(&registry);
    assert!(
        out.contains(r#"action_type="http.request""#),
        "allowed label present:\n{out}"
    );
    assert!(!out.contains("execution_id"), "filtered key absent:\n{out}");
}

#[test]
fn mixed_labeled_and_unlabeled_metrics_export() {
    let registry = Arc::new(MetricsRegistry::new());
    let labels = registry
        .interner()
        .label_set(&[("action_type", "http.request")]);

    registry.counter(ACTION_EXECUTIONS.as_str()).inc_by(5);
    registry
        .counter_labeled(ACTION_EXECUTIONS.as_str(), &labels)
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
