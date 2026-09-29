//! `ExecutionCommandService`: the §12.2 control contract, once, for every surface.
//!
//! Runs over the in-memory port adapters. The SQLite/PostgreSQL adapters share the
//! same `ExecutionStore` / `ControlQueue` contracts (asserted by the storage
//! conformance matrix), so the service holds no backend-specific logic.

use std::sync::Arc;

use nebula_core::ExecutionId;
use nebula_engine::{ExecutionCommandError, ExecutionCommandService};
use nebula_storage::{InMemoryControlQueue, InMemoryExecutionStore};
use nebula_storage_port::{Scope, dto::ControlCommand, store::ExecutionStore};
use serde_json::json;

struct Fixture {
    service: ExecutionCommandService,
    store: Arc<InMemoryExecutionStore>,
    queue: Arc<InMemoryControlQueue>,
    scope: Scope,
}

impl Fixture {
    fn new() -> Self {
        let store = Arc::new(InMemoryExecutionStore::new());
        let queue = Arc::new(InMemoryControlQueue::new(&store));
        Self {
            service: ExecutionCommandService::new(store.clone(), queue.clone()),
            store,
            queue,
            scope: Scope::new("ws_a", "org_a"),
        }
    }

    async fn execution(&self, status: &str) -> ExecutionId {
        let id = ExecutionId::new();
        self.store
            .create(
                &self.scope,
                &id.to_string(),
                "wf_1",
                json!({ "status": status, "workflow_id": "wf_1" }),
            )
            .await
            .expect("seed execution");
        id
    }

    fn commands(&self) -> Vec<ControlCommand> {
        self.queue
            .snapshot()
            .into_iter()
            .map(|(msg, _)| msg.command)
            .collect()
    }
}

#[tokio::test]
async fn cancel_running_enqueues_exactly_one_cancel_and_writes_nothing() {
    let f = Fixture::new();
    let id = f.execution("running").await;
    let receipt = f
        .service
        .cancel(&f.scope, id, None)
        .await
        .expect("accepted");
    assert!(receipt.enqueued);
    assert_eq!(receipt.execution_state["status"], "running");
    assert_eq!(f.commands(), vec![ControlCommand::Cancel]);
    let stored = f
        .store
        .get(&f.scope, &id.to_string())
        .await
        .unwrap()
        .expect("row");
    assert_eq!(
        stored.state["status"], "running",
        "the service never writes"
    );
}

#[tokio::test]
async fn duplicate_cancel_on_cancelling_enqueues_nothing() {
    let f = Fixture::new();
    let id = f.execution("cancelling").await;
    let receipt = f
        .service
        .cancel(&f.scope, id, None)
        .await
        .expect("accepted");
    assert!(!receipt.enqueued);
    assert_eq!(receipt.execution_state["status"], "cancelling");
    assert!(f.commands().is_empty());
}

#[tokio::test]
async fn terminal_state_is_refused_for_both_commands() {
    let f = Fixture::new();
    for status in ["completed", "failed", "cancelled", "timed_out"] {
        let id = f.execution(status).await;
        let cancel = f.service.cancel(&f.scope, id, None).await;
        assert!(
            matches!(&cancel, Err(ExecutionCommandError::Terminal { verb: "cancel", status: s }) if s == status),
            "{status}: {cancel:?}"
        );
        let terminate = f.service.terminate(&f.scope, id, None).await;
        assert!(
            matches!(
                &terminate,
                Err(ExecutionCommandError::Terminal {
                    verb: "terminate",
                    ..
                })
            ),
            "{status}: {terminate:?}"
        );
    }
    assert!(f.commands().is_empty());
}

#[tokio::test]
async fn unknown_and_foreign_tenant_executions_are_not_found() {
    let f = Fixture::new();
    let missing = f.service.cancel(&f.scope, ExecutionId::new(), None).await;
    assert!(matches!(missing, Err(ExecutionCommandError::NotFound(_))));

    let id = f.execution("running").await;
    let other = Scope::new("ws_b", "org_b");
    let foreign = f.service.cancel(&other, id, None).await;
    assert!(matches!(foreign, Err(ExecutionCommandError::NotFound(_))));
    assert!(f.commands().is_empty());
}

#[tokio::test]
async fn terminate_enqueues_terminate_even_when_cancelling() {
    let f = Fixture::new();
    let id = f.execution("cancelling").await;
    let receipt = f
        .service
        .terminate(&f.scope, id, None)
        .await
        .expect("accepted");
    assert!(receipt.enqueued);
    assert_eq!(f.commands(), vec![ControlCommand::Terminate]);
}

#[tokio::test]
async fn every_outcome_is_counted_by_command_and_outcome() {
    use nebula_metrics::{
        MetricsRegistry,
        naming::{NEBULA_ENGINE_EXECUTION_COMMAND_TOTAL, execution_command_outcome as outcome},
    };
    let f = Fixture::new();
    let metrics = MetricsRegistry::new();
    let service = f.service.clone().with_metrics(metrics.clone());
    let running = f.execution("running").await;
    let cancelling = f.execution("cancelling").await;
    let done = f.execution("completed").await;

    service.cancel(&f.scope, running, None).await.unwrap();
    service.cancel(&f.scope, cancelling, None).await.unwrap();
    service.cancel(&f.scope, done, None).await.unwrap_err();
    service
        .terminate(&f.scope, ExecutionId::new(), None)
        .await
        .unwrap_err();

    let count = |command: &str, outcome: &str| {
        let labels = metrics
            .interner()
            .label_set(&[("command", command), ("outcome", outcome)]);
        metrics
            .counter_labeled(NEBULA_ENGINE_EXECUTION_COMMAND_TOTAL, &labels)
            .unwrap()
            .get()
    };
    assert_eq!(count("cancel", outcome::ENQUEUED), 1);
    assert_eq!(count("cancel", outcome::DUPLICATE), 1);
    assert_eq!(count("cancel", outcome::TERMINAL), 1);
    assert_eq!(count("terminate", outcome::NOT_FOUND), 1);
    assert_eq!(count("terminate", outcome::ENQUEUED), 0);
}
