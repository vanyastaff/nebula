//! The journal's verdict and the authority boundaries of journaled actions.

use super::{journal_fixture::*, *};

#[tokio::test]
async fn the_journal_waits_for_a_unit_its_action_did_not_await() {
    let fixture = JournalFixture::new(Ports::memory()).await;
    let gate = Arc::new(Gate::default());
    *fixture.gateway.hold_next.lock() = Some(Arc::clone(&gate));
    let execution = fixture
        .start(&[write("order-10:7")], json!({ "leak": true }))
        .await;
    let engine = fixture.engine();
    let scope = fixture.scope.clone();
    let turn = tokio::spawn(async move { engine.resume_execution(&scope, execution).await });
    tokio::time::timeout(HANG_GUARD, gate.entered.notified())
        .await
        .unwrap();
    // The action already returned; the node is not finished while its unit
    // is in flight.
    tokio::task::yield_now().await;
    assert!(!turn.is_finished());
    gate.release.notify_one();
    let result = tokio::time::timeout(HANG_GUARD, turn)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(
        receipts(&result),
        json!([]),
        "the action returned without it"
    );
    let slot = fixture.slots(execution).await.remove(0);
    assert_eq!(phase(&slot), EffectPhase::Resolved, "the unit was recorded");
    assert_eq!(fixture.gateway.call_count(), 1);
}

#[tokio::test]
async fn a_stateful_journaled_action_keeps_read_only_handles() {
    let fixture = JournalFixture::build(Ports::memory(), Kind::Stateful, None).await;
    let execution = fixture
        .start(&[write("order-11:7")], json!({ "swallow": true }))
        .await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    let refused = &receipts(&result)[0];
    assert_eq!(refused["sent"], "not_sent", "{refused}");
    assert_eq!(
        refused["detail"], "managed row effect requires execution-owner authority",
        "{refused}"
    );
    assert_eq!(fixture.gateway.call_count(), 0);
    assert!(fixture.slots(execution).await.is_empty(), "no journal");
}

#[tokio::test]
async fn a_journaled_action_under_its_journal_still_cannot_take_a_raw_lease() {
    let fixture = JournalFixture::new(Ports::memory()).await;
    let execution = fixture
        .start(&[write("order-12:7")], json!({ "raw_lease": true }))
        .await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    let output = &result.node_outputs[&node_key!("charge")];
    assert_eq!(output["raw_lease_refused"], true, "{output}");
    assert_eq!(receipts(&result), json!([1]), "the handle's write ran");
    assert_eq!(fixture.gateway.call_count(), 1);
}

#[tokio::test]
async fn generic_dispatch_refuses_a_journaled_action() {
    let fixture = JournalFixture::new(Ports::memory()).await;
    let factory = fixture
        .frozen
        .resolve_action(&action_key!("journal.charge"))
        .expect("the fixture's action");
    let registry = Arc::new(ActionRegistry::new());
    registry.register_factory(factory);
    let runtime = ActionRuntime::try_new(
        registry,
        Arc::new(InProcessRunner::new()),
        DataPassingPolicy::default(),
        MetricsRegistry::new(),
    )
    .unwrap();
    let node =
        NodeDefinition::new(node_key!("charge"), "Charge", "journal", "journal.charge").unwrap();
    let context = nebula_action::testing::TestContextBuilder::new().build();
    let result = runtime
        .execute_action_with_node(
            &node,
            None,
            json!({ "units": [write("order-13:7")] }),
            &context,
            None,
        )
        .await;
    assert!(
        matches!(
            result,
            Err(nebula_engine::RuntimeError::EffectRequiresOwner)
        ),
        "{result:?}"
    );
    assert_eq!(fixture.gateway.call_count(), 0);
}
