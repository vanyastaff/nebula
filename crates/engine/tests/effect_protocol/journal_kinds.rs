//! Which action kinds the node effect journal records: only stateless
//! actions. A control or stateful action of the default (`Journaled`)
//! contract keeps read-only handles on a durable turn, and a refused write
//! says why. (An agent action cannot be compiled into a durable plan;
//! `resource_integration` covers its refusal.)

use super::{
    journal_fixture::*,
    restart::{Backend, Database},
    *,
};

/// Runs one execution of a node of `kind` whose action swallows the refusal
/// of a single write, and checks the write was refused `Permanent` /
/// `NotSent` with `detail`, before any provider call and with no slot.
async fn assert_write_refused(ports: Ports, kind: Kind, detail: &str) {
    let fixture = JournalFixture::build(ports, kind, None).await;
    let execution = fixture
        .start(&[write("order-24:7")], json!({ "swallow": true }))
        .await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    let refused = &receipts(&result)[0];
    assert_eq!(refused["kind"], "permanent", "{kind:?}: {refused}");
    assert_eq!(refused["sent"], "not_sent", "{kind:?}: {refused}");
    assert_eq!(refused["detail"], detail, "{kind:?}: {refused}");
    assert_eq!(
        fixture.gateway.call_count(),
        0,
        "{kind:?}: no provider call"
    );
    assert!(
        fixture.slots(execution).await.is_empty(),
        "{kind:?}: no journal"
    );
}

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_journaled_control_action_is_refused_a_write_saying_why(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    assert_write_refused(
        database.ports(),
        Kind::Control,
        "control actions decide flow and must not cause effects; move effects to a stateless \
         action",
    )
    .await;
}

#[tokio::test]
async fn a_read_only_control_action_keeps_read_only_handles() {
    // Like the built-in If, Switch and Filter: a `ReadOnly` contract.
    assert_write_refused(
        Ports::memory(),
        Kind::ReadOnlyControl,
        "managed row effect requires execution-owner authority",
    )
    .await;
}

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_journaled_stateful_action_keeps_read_only_handles_saying_why(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    assert_write_refused(
        database.ports(),
        Kind::Stateful,
        "stateful effects are journaled per iteration in a later release",
    )
    .await;
}
