//! Finished-execution retention on every backend.
//!
//! The sweep deletes terminal executions whose `finished_at` is older than
//! the retention, oldest first and at most `limit` per call, across every
//! tenant, with the rows recorded beneath them. Live executions, recent
//! terminal ones and ones still holding a revision in an unexpired rollback
//! window stay.

use std::time::Duration;

use nebula_core::{ExecutionId, OrgId, WorkspaceId};
use nebula_storage_port::dto::{ControlCommand, ControlMsg, JournalEntry};
use nebula_storage_port::store::{ExecutionStore, StartMaterialization};
use nebula_storage_port::{
    ExecutionListing, ExecutionListingStatus as Status, ExecutionReferenceTransition, Scope,
    TransitionBatch, TransitionOutcome,
};

use super::{
    Backend, install_materialized_pair, materialize_execution_reference, scope_a, scope_b,
    seed_scope_and_workflow,
};

const DAY: Duration = Duration::from_hours(24);

/// A fixed instant in 2025, `seconds` apart: long past any test retention.
fn long_ago(seconds: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp_micros(1_759_665_600_000_000 + seconds * 1_000_000)
        .expect("fixed instant is in range")
}

/// Create `id` in `scope` and commit `listing` as its first snapshot, with
/// `extra` applied to the batch.
async fn create_with_listing(
    store: &dyn ExecutionStore,
    scope: &Scope,
    id: &str,
    listing: ExecutionListing,
    extra: impl FnOnce(TransitionBatch) -> TransitionBatch,
) {
    store
        .create(scope, id, "wf_retention", serde_json::json!({}))
        .await
        .expect("create");
    let token = store
        .acquire_lease(scope, id, "retention", Duration::from_secs(30))
        .await
        .expect("acquire_lease")
        .expect("a fresh execution's lease is free");
    let batch = TransitionBatch::new(
        scope.clone(),
        id,
        0,
        token,
        serde_json::json!({"status": listing.status().as_str()}),
        listing,
    );
    match store.commit(extra(batch)).await.expect("commit") {
        TransitionOutcome::Applied { .. } => {},
        other => panic!("the fixture commit must apply, got {other:?}"),
    }
}

fn finished(status: Status, at: chrono::DateTime<chrono::Utc>) -> ExecutionListing {
    ExecutionListing::new(status, Some(at), Some(at))
}

/// Old terminal executions go, oldest first and in bounded batches, in
/// every tenant, with their journal and control-queue rows; live and recent
/// ones stay.
pub(crate) async fn assert_retention_deletes_only_old_terminal_executions(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let retention = backend.execution_retention().await;
    let (a, b) = (scope_a(), scope_b());
    seed_scope_and_workflow(backend, &a, "wf_retention").await;
    seed_scope_and_workflow(backend, &b, "wf_retention").await;

    let cancel = ControlMsg {
        id: [0x5a; 16],
        execution_id: "exe_ret_oldest".into(),
        command: ControlCommand::Cancel,
        scope: a.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    create_with_listing(
        store.as_ref(),
        &a,
        "exe_ret_oldest",
        finished(Status::Failed, long_ago(10)),
        |batch| {
            batch
                .with_outbox(vec![cancel])
                .with_journal(vec![JournalEntry {
                    seq: None,
                    payload: serde_json::json!({"event": "failed"}),
                }])
        },
    )
    .await;
    create_with_listing(
        store.as_ref(),
        &b,
        "exe_ret_other_tenant",
        finished(Status::TimedOut, long_ago(20)),
        |batch| batch,
    )
    .await;
    create_with_listing(
        store.as_ref(),
        &a,
        "exe_ret_old",
        finished(Status::Completed, long_ago(30)),
        |batch| batch,
    )
    .await;
    create_with_listing(
        store.as_ref(),
        &a,
        "exe_ret_recent",
        finished(Status::Cancelled, chrono::Utc::now()),
        |batch| batch,
    )
    .await;
    create_with_listing(
        store.as_ref(),
        &a,
        "exe_ret_running",
        ExecutionListing::new(Status::Running, Some(long_ago(0)), None),
        |batch| batch,
    )
    .await;
    store
        .create(&a, "exe_ret_created", "wf_retention", serde_json::json!({}))
        .await
        .expect("create");

    assert_eq!(
        retention
            .delete_finished_executions(DAY, 2)
            .await
            .expect("first batch"),
        2,
        "[{}] a batch deletes at most `limit` rows",
        backend.name()
    );
    assert!(
        store.get(&a, "exe_ret_oldest").await.unwrap().is_none()
            && store
                .get(&b, "exe_ret_other_tenant")
                .await
                .unwrap()
                .is_none(),
        "[{}] the oldest finished executions go first, in every tenant",
        backend.name()
    );
    assert!(
        store.get(&a, "exe_ret_old").await.unwrap().is_some(),
        "[{}] a younger row waits for the next batch",
        backend.name()
    );
    assert_eq!(
        retention
            .delete_finished_executions(DAY, 2)
            .await
            .expect("second batch"),
        1,
        "[{}]",
        backend.name()
    );
    assert_eq!(
        retention
            .delete_finished_executions(DAY, 2)
            .await
            .expect("third batch"),
        0,
        "[{}] nothing else is old and terminal",
        backend.name()
    );

    for kept in ["exe_ret_recent", "exe_ret_running", "exe_ret_created"] {
        assert!(
            store.get(&a, kept).await.unwrap().is_some(),
            "[{}] {kept} must survive the sweep",
            backend.name()
        );
    }
    let journal = backend
        .journal_reader()
        .await
        .get_journal(&a, "exe_ret_oldest")
        .await
        .expect("read the journal");
    assert!(
        journal.is_empty(),
        "[{}] the journal goes with its execution",
        backend.name()
    );
    let claims = backend
        .control_queue()
        .await
        .claim_pending(&[0x01; 16], 10)
        .await
        .expect("claim");
    assert!(
        claims.is_empty(),
        "[{}] queued commands go with their execution, got {} claims",
        backend.name(),
        claims.len()
    );
}

/// An execution whose revision sits in an unexpired rollback window stays
/// until the window ends; one that released its reference goes.
pub(crate) async fn assert_retention_keeps_unexpired_rollback_windows(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let retention = backend.execution_retention().await;
    // The contract bundle names typed tenant ids.
    let scope = Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string());
    let mut ids = Vec::new();
    for (seed, transition) in [
        (
            0x7a,
            ExecutionReferenceTransition::RetainRollback {
                window_id: [0x7a; 16],
                retain_until: chrono::Utc::now() + chrono::TimeDelta::hours(1),
            },
        ),
        (0x7b, ExecutionReferenceTransition::ReleaseLive),
    ] {
        let id = ExecutionId::new().to_string();
        let record = install_materialized_pair(backend, seed).await;
        let accepted = materialize_execution_reference(backend, &scope, &id, &record, seed).await;
        assert!(matches!(accepted, StartMaterialization::Accepted { .. }));
        let token = store
            .acquire_lease(&scope, &id, "retention", Duration::from_secs(30))
            .await
            .expect("acquire lease")
            .expect("a fresh execution's lease is free");
        let batch = TransitionBatch::new(
            scope.clone(),
            id.clone(),
            0,
            token,
            serde_json::json!({"status": "completed"}),
            finished(Status::Completed, long_ago(0)),
        )
        .with_reference_transition(transition);
        assert!(matches!(
            store.commit(batch).await.expect("terminal commit"),
            TransitionOutcome::Applied { .. }
        ));
        ids.push(id);
    }

    assert_eq!(
        retention
            .delete_finished_executions(DAY, 10)
            .await
            .expect("sweep"),
        1,
        "[{}] only the released execution is deleted",
        backend.name()
    );
    assert!(
        store.get(&scope, &ids[0]).await.unwrap().is_some(),
        "[{}] an unexpired rollback window keeps its execution",
        backend.name()
    );
    assert!(
        store.get(&scope, &ids[1]).await.unwrap().is_none(),
        "[{}] a released execution past retention is deleted",
        backend.name()
    );
}
