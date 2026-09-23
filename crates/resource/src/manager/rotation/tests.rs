use std::{sync::Arc, time::Duration};

use nebula_core::ResourceKey;

use super::{Manager, SlotHookDirection, slot_hook_observation_deadline};
use crate::runtime::acquire_loop::{RetiredCleanupObservation, SlotHookObservation};
use crate::{ErrorKind, ManagerConfig, ResourceEvent};

#[test]
fn slot_hook_observation_deadline_saturates_overflowing_timeout() {
    let before = tokio::time::Instant::now();

    let deadline = slot_hook_observation_deadline(Duration::MAX);

    assert!(deadline >= before);
}

#[tokio::test]
async fn admitted_hook_abandonment_emits_one_terminal_metric_and_redacted_event() {
    let manager = Manager::with_config(
        ManagerConfig::default()
            .with_metrics_registry(Arc::new(nebula_metrics::MetricsRegistry::new())),
    );
    let mut events = manager.subscribe_events();
    let (settlement, admission) = manager.slot_hook_settlement(
        ResourceKey::new("abandoned-hook").expect("valid static resource key"),
        "credential".to_owned(),
        SlotHookDirection::Refresh,
    );

    admission.admit();
    drop(settlement);

    let snapshot = manager
        .metrics()
        .expect("configured manager exposes metrics")
        .snapshot()
        .slot_refresh_outcomes;
    assert_eq!(snapshot.success, 0);
    assert_eq!(snapshot.failed, 0);
    assert_eq!(snapshot.timed_out, 0);
    assert_eq!(snapshot.abandoned, 1);

    let event = events
        .try_recv()
        .expect("abandonment emits a terminal event");
    let ResourceEvent::SlotRefreshFailed { kind, message, .. } = event else {
        panic!("abandonment must emit the refresh failure observation")
    };
    assert_eq!(kind, ErrorKind::Cancelled);
    assert_eq!(
        message.as_str(),
        "admitted credential hook was abandoned before producing a result"
    );
    assert!(
        events.try_recv().is_none(),
        "abandonment must emit exactly one terminal event"
    );
}

#[tokio::test]
async fn retained_cleanup_faults_are_separate_from_hook_terminal_settlement() {
    let cases = [
        (
            RetiredCleanupObservation::Failed(ErrorKind::Permanent),
            ErrorKind::Permanent,
        ),
        (RetiredCleanupObservation::TimedOut, ErrorKind::Backpressure),
        (RetiredCleanupObservation::Abandoned, ErrorKind::Cancelled),
    ];

    for (cleanup_observation, expected_kind) in cases {
        let manager = Manager::with_config(
            ManagerConfig::default()
                .with_metrics_registry(Arc::new(nebula_metrics::MetricsRegistry::new())),
        );
        let mut events = manager.subscribe_events();
        let (settlement, admission) = manager.slot_hook_settlement(
            ResourceKey::new("retained-cleanup-fault").expect("valid static resource key"),
            "credential".to_owned(),
            SlotHookDirection::Refresh,
        );
        admission.admit();
        let cleanup_observer = settlement
            .settle(SlotHookObservation::Completed)
            .expect("manager installs retained cleanup observation");
        cleanup_observer(cleanup_observation);

        let snapshot = manager
            .metrics()
            .expect("configured manager exposes metrics")
            .snapshot();
        assert_eq!(snapshot.release_errors, 1);
        assert_eq!(snapshot.slot_refresh_outcomes.success, 1);
        assert_eq!(snapshot.slot_refresh_outcomes.failed, 0);
        assert_eq!(snapshot.slot_refresh_outcomes.timed_out, 0);
        assert_eq!(snapshot.slot_refresh_outcomes.abandoned, 0);

        let mut hook_successes = 0;
        let mut cleanup_failures = 0;
        while let Some(event) = events.try_recv() {
            match event {
                ResourceEvent::SlotRefreshed { .. } => hook_successes += 1,
                ResourceEvent::RetiredCleanupFailed { kind, .. } => {
                    assert_eq!(kind, expected_kind);
                    cleanup_failures += 1;
                },
                ResourceEvent::SlotRefreshFailed { .. } => {
                    panic!("retained cleanup must not rewrite the hook terminal outcome")
                },
                _ => {},
            }
        }
        assert_eq!(hook_successes, 1);
        assert_eq!(cleanup_failures, 1);
    }
}
