//! Unit tests for the rotation outcome substrate (П2 foundation).
//!
//! Covers `RefreshOutcome` / `RotationOutcome` aggregation and the
//! `Error::missing_credential_id` constructor. Behavior-bearing dispatcher
//! tests live in `tests/rotation.rs` (Task 14).

use std::time::Duration;

use nebula_resource::{Error, RefreshOutcome, RotationOutcome};

#[test]
fn rotation_outcome_aggregates_correctly() {
    let outcomes = vec![
        RefreshOutcome::Ok,
        RefreshOutcome::Ok,
        RefreshOutcome::Failed(Error::permanent("test")),
        RefreshOutcome::TimedOut {
            budget: Duration::from_secs(30),
        },
    ];

    let agg = RotationOutcome {
        ok: outcomes
            .iter()
            .filter(|o| matches!(o, RefreshOutcome::Ok))
            .count(),
        failed: outcomes
            .iter()
            .filter(|o| matches!(o, RefreshOutcome::Failed(_)))
            .count(),
        timed_out: outcomes
            .iter()
            .filter(|o| matches!(o, RefreshOutcome::TimedOut { .. }))
            .count(),
    };

    assert_eq!(agg.total(), 4);
    assert_eq!(agg.ok, 2);
    assert_eq!(agg.failed, 1);
    assert_eq!(agg.timed_out, 1);
    assert!(agg.has_partial_failure());
}

#[test]
fn missing_credential_id_error_carries_key() {
    let err = Error::missing_credential_id(nebula_core::resource_key!("test.resource"));
    let formatted = format!("{err}");
    assert!(
        formatted.contains("test.resource"),
        "error must mention key, got: {formatted}"
    );
    assert!(
        formatted.contains("credential_id"),
        "error must mention credential_id, got: {formatted}"
    );
}
