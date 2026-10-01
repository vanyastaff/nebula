//! Which action kinds the node effect journal records: control actions are
//! journaled exactly like stateless ones; a stateful action keeps read-only
//! handles and says why a write is refused. (An agent action cannot be
//! compiled into a durable plan; `resource_integration` covers its
//! refusal.)

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
async fn a_journaled_control_action_writes_through_one_flat_slot(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = JournalFixture::build(database.ports(), Kind::Control, None).await;
    let calls_before = fixture.gateway.call_count();
    let execution = fixture.start(&[write("order-21:7")], json!({})).await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    let slots = fixture.slots(execution).await;
    assert_eq!(slots.len(), 1, "one slot per effect");
    assert_eq!(slots[0].occurrence(), "unit/v1/#000000");
    assert_eq!(phase(&slots[0]), EffectPhase::Resolved);
    assert_eq!(
        fixture.gateway.call_count() - calls_before,
        1,
        "the provider was called once"
    );
    assert_eq!(
        fixture.gateway.call_keys().last().cloned().flatten(),
        Some(recorded_key(&slots[0])),
        "the provider received the key recorded at prepare"
    );
    assert_eq!(fixture.gateway.applied(), 1, "applied once");
    assert_eq!(receipts(&result), json!([1]));
}

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_control_node_retry_replays_its_settled_write_without_a_provider_call(
    #[case] backend: Backend,
) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = JournalFixture::build(
        database.ports(),
        Kind::Control,
        Some(nebula_workflow::RetryConfig::fixed(3, 1)),
    )
    .await;
    let dispatches_before = fixture.controls.dispatches.load(Ordering::SeqCst);
    fixture.controls.fail_after_units.store(1, Ordering::SeqCst);
    let execution = fixture
        .start(&[write("order-22:7"), write("order-23:7")], json!({}))
        .await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(
        fixture.controls.dispatches.load(Ordering::SeqCst) - dispatches_before,
        2,
        "the node was retried once"
    );
    assert_eq!(
        fixture.gateway.call_count(),
        2,
        "the retry replayed both writes without a provider call"
    );
    assert_eq!(receipts(&result), json!([1, 2]), "the recorded outputs");
    let slots = fixture.slots(execution).await;
    assert_eq!(
        slots
            .iter()
            .map(nebula_storage_port::EffectOccurrenceRecord::occurrence)
            .collect::<Vec<_>>(),
        ["unit/v1/#000000", "unit/v1/#000001"],
        "the retry reused the occurrences"
    );
    assert!(
        slots
            .iter()
            .all(|slot| phase(slot) == EffectPhase::Resolved)
    );
}

#[tokio::test]
async fn a_read_only_control_action_keeps_read_only_handles() {
    let fixture = JournalFixture::build(Ports::memory(), Kind::ReadOnlyControl, None).await;
    let execution = fixture
        .start(&[write("order-24:7")], json!({ "swallow": true }))
        .await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    let refused = &receipts(&result)[0];
    assert_eq!(refused["kind"], "permanent", "{refused}");
    assert_eq!(refused["sent"], "not_sent", "{refused}");
    assert_eq!(
        refused["detail"], "managed row effect requires execution-owner authority",
        "{refused}"
    );
    assert_eq!(fixture.gateway.call_count(), 0);
    assert!(fixture.slots(execution).await.is_empty(), "no journal");
}

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_journaled_stateful_action_keeps_read_only_handles_saying_why(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = JournalFixture::build(database.ports(), Kind::Stateful, None).await;
    let execution = fixture
        .start(&[write("order-11:7")], json!({ "swallow": true }))
        .await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    let refused = &receipts(&result)[0];
    assert_eq!(refused["kind"], "permanent", "{refused}");
    assert_eq!(refused["sent"], "not_sent", "{refused}");
    assert_eq!(
        refused["detail"], "stateful effects are journaled per iteration in a later release",
        "{refused}"
    );
    assert_eq!(fixture.gateway.call_count(), 0);
    assert!(fixture.slots(execution).await.is_empty(), "no journal");
}
