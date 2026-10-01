//! Journaled effects across node retries, crashes, lease takeovers and
//! changed requests.

use std::sync::atomic::Ordering;

use super::{
    journal_fixture::*,
    restart::{Backend, Database},
    *,
};

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_write_settles_once_and_the_provider_sees_the_recorded_key(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = JournalFixture::new(database.ports()).await;
    let mut keys = Vec::new();
    for _ in 0..2 {
        let execution = fixture.start(&[write("order-1:7")], json!({})).await;
        let result = fixture.run(execution).await.unwrap();
        assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
        let slots = fixture.slots(execution).await;
        assert_eq!(slots.len(), 1, "one slot per effect");
        assert_eq!(phase(&slots[0]), EffectPhase::Resolved);
        assert_eq!(
            slots[0].occurrence(),
            "unit/v1/test.journal.payments/op/#000000"
        );
        keys.push(recorded_key(&slots[0]));
    }
    let received = fixture.gateway.call_keys();
    assert_eq!(received.len(), 2, "one provider call per execution");
    assert_eq!(
        received,
        keys.iter().cloned().map(Some).collect::<Vec<_>>(),
        "the provider received the composite key recorded at prepare"
    );
    assert_eq!(keys[0].len(), 43);
    assert_ne!(keys[0], keys[1], "without a key part each run has its own");

    // With a developer key part, two executions present one key.
    let keyed = json!({"request": "order-2:7", "key": "order-2"});
    for _ in 0..2 {
        let execution = fixture.start(std::slice::from_ref(&keyed), json!({})).await;
        let result = fixture.run(execution).await.unwrap();
        assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    }
    let received = fixture.gateway.call_keys();
    assert_eq!(
        received[2], received[3],
        "one developer key across executions"
    );
    assert_eq!(fixture.gateway.applied(), 3, "the provider deduplicated");
}

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_node_retry_after_an_action_failure_replays_the_settled_write(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture =
        JournalFixture::with_retry(database.ports(), nebula_workflow::RetryConfig::fixed(3, 1))
            .await;
    fixture.controls.fail_after_units.store(1, Ordering::SeqCst);
    let execution = fixture
        .start(&[write("order-3:7"), write("order-4:7")], json!({}))
        .await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(fixture.controls.dispatches.load(Ordering::SeqCst), 2);
    assert_eq!(
        fixture.gateway.call_count(),
        2,
        "the retry replayed both writes without a provider call"
    );
    assert_eq!(receipts(&result), json!([1, 2]), "the recorded outputs");
    let slots = fixture.slots(execution).await;
    assert_eq!(slots.len(), 2, "the retry reused the occurrences");
    assert!(
        slots
            .iter()
            .all(|slot| phase(slot) == EffectPhase::Resolved)
    );
}

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn a_takeover_after_the_effects_settled_replays_without_provider_calls(
    #[case] backend: Backend,
) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = JournalFixture::new(database.ports()).await;
    let execution = fixture
        .start(&[write("a:1"), write("b:2"), write("c:3")], json!({}))
        .await;
    fixture
        .controls
        .hold_after_units
        .store(true, Ordering::SeqCst);
    fixture
        .crash_at(execution, &fixture.controls.after_units)
        .await;
    assert_eq!(fixture.gateway.call_count(), 3);
    let slots = fixture.slots(execution).await;
    assert_eq!(slots.len(), 3);
    assert!(
        slots
            .iter()
            .all(|slot| phase(slot) == EffectPhase::Resolved)
    );

    fixture
        .controls
        .hold_after_units
        .store(false, Ordering::SeqCst);
    fixture.ports = database.reconnect().await;
    database.expire_abandoned_leases().await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(
        fixture.gateway.call_count(),
        3,
        "the new owner replays every recorded effect"
    );
    assert_eq!(receipts(&result), json!([1, 2, 3]));
    assert_eq!(
        fixture.slots(execution).await,
        slots,
        "replay writes nothing"
    );
}

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn a_submission_dropped_unpolled_before_a_crash_shifts_nothing(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = JournalFixture::new(database.ports()).await;
    let execution = fixture.start(&[write("order-8:7")], json!({})).await;
    // The first dispatch builds a submission it drops unpolled, settles the
    // write and dies before the node is recorded; the resumed dispatch
    // takes the other branch.
    fixture
        .controls
        .drop_unpolled_once
        .store(true, Ordering::SeqCst);
    fixture
        .controls
        .hold_after_units
        .store(true, Ordering::SeqCst);
    fixture
        .crash_at(execution, &fixture.controls.after_units)
        .await;
    let slots = fixture.slots(execution).await;
    assert_eq!(slots.len(), 1);
    assert_eq!(
        slots[0].occurrence(),
        "unit/v1/test.journal.payments/op/#000000",
        "the dropped submission took no position"
    );

    fixture
        .controls
        .hold_after_units
        .store(false, Ordering::SeqCst);
    fixture.ports = database.reconnect().await;
    database.expire_abandoned_leases().await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(receipts(&result), json!([1]), "the recorded output");
    assert_eq!(fixture.gateway.call_count(), 1, "no second provider call");
    assert_eq!(fixture.slots(execution).await, slots);
}

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn a_resume_that_skips_a_settled_effect_fails_the_node(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = JournalFixture::new(database.ports()).await;
    let execution = fixture.start(&[write("order-9:7")], json!({})).await;
    fixture
        .controls
        .hold_after_units
        .store(true, Ordering::SeqCst);
    fixture
        .crash_at(execution, &fixture.controls.after_units)
        .await;
    let settled = fixture.slots(execution).await;
    assert_eq!(phase(&settled[0]), EffectPhase::Resolved);

    // The resumed dispatch takes another branch and submits nothing.
    fixture
        .controls
        .hold_after_units
        .store(false, Ordering::SeqCst);
    fixture.controls.skip_units.store(true, Ordering::SeqCst);
    fixture.ports = database.reconnect().await;
    database.expire_abandoned_leases().await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Failed, "{result:?}");
    assert!(
        node_error(&result).starts_with("ENGINE:EFFECT_OCCURRENCE_MISMATCH"),
        "{}",
        node_error(&result)
    );
    assert_eq!(fixture.gateway.call_count(), 1);
    assert_eq!(fixture.slots(execution).await, settled);
}

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn an_interrupted_write_fails_the_node_unknown_and_a_resume_sends_nothing(
    #[case] backend: Backend,
) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    // The answer is lost on the wire, with the action propagating or
    // swallowing the unit's error; or the process dies mid-call.
    for (lost, swallow) in [(true, false), (true, true), (false, false)] {
        let mut fixture =
            JournalFixture::with_retry(database.ports(), nebula_workflow::RetryConfig::fixed(3, 1))
                .await;
        let execution = fixture
            .start(&[write("order-5:7")], json!({ "swallow": swallow }))
            .await;
        if lost {
            fixture.gateway.lose_first.store(1, Ordering::SeqCst);
            let result = fixture.run(execution).await.unwrap();
            assert_eq!(result.status, ExecutionStatus::Failed, "{result:?}");
            assert!(
                node_error(&result).starts_with("ENGINE:EFFECT_OUTCOME_UNKNOWN"),
                "swallow={swallow}: {}",
                node_error(&result)
            );
            assert_eq!(
                fixture.controls.dispatches.load(Ordering::SeqCst),
                1,
                "an unknown outcome is never retried"
            );
        } else {
            fixture.gateway.hang_next.store(true, Ordering::SeqCst);
            fixture.crash_at(execution, &fixture.gateway.entered).await;
            assert_eq!(
                phase(&fixture.slots(execution).await[0]),
                EffectPhase::InvocationOutstanding
            );
            fixture.ports = database.reconnect().await;
            database.expire_abandoned_leases().await;
            let result = fixture.run(execution).await.unwrap();
            assert_eq!(result.status, ExecutionStatus::Failed, "{result:?}");
            assert!(
                node_error(&result).starts_with("ENGINE:EFFECT_OUTCOME_UNKNOWN"),
                "{}",
                node_error(&result)
            );
        }
        assert_eq!(fixture.gateway.call_count(), 1);
        let slot = fixture.slots(execution).await.remove(0);
        assert_eq!(phase(&slot), EffectPhase::OutcomeUnknown);
        // A later turn changes nothing and calls nothing.
        let _ = fixture.run(execution).await;
        assert_eq!(fixture.gateway.call_count(), 1);
        assert_eq!(fixture.slots(execution).await.remove(0), slot);
    }
}

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn idempotent_crash_residue_is_sent_again_under_the_same_key(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = JournalFixture::new(database.ports()).await;
    let unit = json!({"request": "order-6:7", "idempotent": true, "budget": 2});
    let execution = fixture.start(std::slice::from_ref(&unit), json!({})).await;
    fixture.gateway.hang_next.store(true, Ordering::SeqCst);
    fixture.crash_at(execution, &fixture.gateway.entered).await;
    let slot = fixture.slots(execution).await.remove(0);
    assert_eq!(phase(&slot), EffectPhase::InvocationOutstanding);

    fixture.ports = database.reconnect().await;
    database.expire_abandoned_leases().await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    let keys = fixture.gateway.call_keys();
    assert_eq!(keys.len(), 2, "sent again once within the key window");
    assert_eq!(keys[0], keys[1], "under the same provider key");
    assert_eq!(keys[0].as_deref(), Some(recorded_key(&slot).as_str()));
    assert_eq!(fixture.gateway.applied(), 1, "the provider deduplicated");
    assert_eq!(receipts(&result), json!([1]));
    let resumed = fixture.slots(execution).await.remove(0);
    assert_eq!(
        resumed.record().operation(),
        slot.record().operation(),
        "recovery keeps the slot's identity and key"
    );
    assert_eq!(phase(&resumed), EffectPhase::Resolved);
}

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_changed_request_operation_or_repointed_credential_is_an_occurrence_mismatch(
    #[case] backend: Backend,
) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    #[derive(Debug, Clone, Copy)]
    enum Change {
        Rotation,
        Request,
        Repoint,
        /// A redeploy (no action version bump) changed the operation at the
        /// settled position: its version, or its key.
        Redeploy(Drift),
    }
    for change in [
        Change::Rotation,
        Change::Request,
        Change::Repoint,
        Change::Redeploy(Drift::Version),
        Change::Redeploy(Drift::Operation),
    ] {
        let mut fixture = JournalFixture::new(database.ports()).await;
        *fixture.identity.lock() = bound_to("cred-a");
        let execution = fixture.start(&[write("order-7:7")], json!({})).await;
        fixture
            .controls
            .hold_after_units
            .store(true, Ordering::SeqCst);
        fixture
            .crash_at(execution, &fixture.controls.after_units)
            .await;
        fixture
            .controls
            .hold_after_units
            .store(false, Ordering::SeqCst);
        let before = fixture.slots(execution).await;
        assert_eq!(before.len(), 1);

        match change {
            // A rotated credential keeps its id: the same binding.
            Change::Rotation => {},
            Change::Request => {
                *fixture.controls.request_override.lock() = Some("order-7:8".to_owned());
            },
            Change::Repoint => *fixture.identity.lock() = bound_to("cred-b"),
            Change::Redeploy(drift) => *fixture.controls.drift.lock() = Some(drift),
        }
        fixture.ports = database.reconnect().await;
        database.expire_abandoned_leases().await;
        let result = fixture.run(execution).await.unwrap();
        assert_eq!(
            fixture.gateway.call_count(),
            1,
            "{change:?}: nothing further sent"
        );
        assert_eq!(fixture.slots(execution).await, before, "{change:?}");
        match change {
            Change::Rotation => {
                assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
                assert_eq!(receipts(&result), json!([1]));
            },
            Change::Request | Change::Repoint | Change::Redeploy(_) => {
                assert_eq!(result.status, ExecutionStatus::Failed, "{result:?}");
                assert!(
                    node_error(&result).starts_with("ENGINE:EFFECT_OCCURRENCE_MISMATCH"),
                    "{change:?}: {}",
                    node_error(&result)
                );
            },
        }
    }
}
