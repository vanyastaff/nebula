//! `ExecutionCommandService`: the §12.2 control contract, once, for every surface.
//!
//! Every case runs against the in-memory and the SQLite port adapters. The
//! PostgreSQL adapter shares the same `ExecutionStore` / `ControlQueue` contracts
//! (asserted by the storage conformance matrix); it is not exercised here because
//! this binary must run without a database.

use std::sync::Arc;

use nebula_core::ExecutionId;
use nebula_engine::{ExecutionCommandError, ExecutionCommandService};
use nebula_metrics::{
    MetricsRegistry,
    naming::{NEBULA_ENGINE_EXECUTION_COMMAND_TOTAL, execution_command_outcome as outcome},
};
use nebula_storage_port::{
    Scope,
    dto::ControlCommand,
    store::{ControlQueue, ExecutionStore},
};
use serde_json::json;

struct Fixture {
    service: ExecutionCommandService,
    store: Arc<dyn ExecutionStore>,
    queue: Arc<dyn ControlQueue>,
    scope: Scope,
}

impl Fixture {
    fn in_memory() -> Self {
        let store = Arc::new(nebula_storage::InMemoryExecutionStore::new());
        let queue = Arc::new(nebula_storage::InMemoryControlQueue::new(&store));
        Self::compose(store, queue)
    }

    async fn sqlite() -> Self {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory SQLite opens");
        nebula_storage::sqlite::init_schema(&pool)
            .await
            .expect("port schema installs");
        Self::compose(
            Arc::new(nebula_storage::sqlite::SqliteExecutionStore::new(
                pool.clone(),
            )),
            Arc::new(nebula_storage::sqlite::SqliteControlQueue::new(pool)),
        )
    }

    fn compose(store: Arc<dyn ExecutionStore>, queue: Arc<dyn ControlQueue>) -> Self {
        Self {
            service: ExecutionCommandService::new(Arc::clone(&store), Arc::clone(&queue)),
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

    /// Every command durably enqueued so far, read back through the port.
    async fn commands(&self) -> Vec<ControlCommand> {
        self.queue
            .claim_pending(&[7; 16], 100)
            .await
            .expect("queue is readable")
            .into_iter()
            .map(|claim| claim.msg.command)
            .collect()
    }
}

/// Run one case body against both adapters as two named tests.
macro_rules! both_backends {
    ($name:ident, $body:ident) => {
        mod $name {
            use super::*;

            #[tokio::test]
            async fn in_memory() {
                $body(Fixture::in_memory()).await;
            }

            #[tokio::test]
            async fn sqlite() {
                $body(Fixture::sqlite().await).await;
            }
        }
    };
}

async fn cancel_running_enqueues_one_cancel_and_writes_nothing(f: Fixture) {
    let id = f.execution("running").await;
    let receipt = f
        .service
        .cancel(&f.scope, id, None)
        .await
        .expect("accepted");
    assert!(receipt.enqueued);
    assert_eq!(receipt.execution_state["status"], "running");
    assert_eq!(f.commands().await, vec![ControlCommand::Cancel]);
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
both_backends!(
    cancel_running_enqueues_exactly_one_cancel_and_writes_nothing,
    cancel_running_enqueues_one_cancel_and_writes_nothing
);

async fn duplicate_cancel_on_cancelling_enqueues_nothing(f: Fixture) {
    let id = f.execution("cancelling").await;
    let receipt = f
        .service
        .cancel(&f.scope, id, None)
        .await
        .expect("accepted");
    assert!(!receipt.enqueued);
    assert_eq!(receipt.execution_state["status"], "cancelling");
    assert!(f.commands().await.is_empty());
}
both_backends!(
    duplicate_cancel_on_cancelling_enqueues_nothing_case,
    duplicate_cancel_on_cancelling_enqueues_nothing
);

async fn terminal_state_is_refused_for_both_commands(f: Fixture) {
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
    assert!(f.commands().await.is_empty());
}
both_backends!(
    terminal_state_is_refused_for_both_commands_case,
    terminal_state_is_refused_for_both_commands
);

async fn unknown_and_foreign_tenant_executions_are_not_found(f: Fixture) {
    let missing = f.service.cancel(&f.scope, ExecutionId::new(), None).await;
    assert!(matches!(missing, Err(ExecutionCommandError::NotFound(_))));

    let id = f.execution("running").await;
    let other = Scope::new("ws_b", "org_b");
    let foreign = f.service.cancel(&other, id, None).await;
    assert!(matches!(foreign, Err(ExecutionCommandError::NotFound(_))));
    assert!(f.commands().await.is_empty());
}
both_backends!(
    unknown_and_foreign_tenant_executions_are_not_found_case,
    unknown_and_foreign_tenant_executions_are_not_found
);

async fn terminate_enqueues_terminate_even_when_cancelling(f: Fixture) {
    let id = f.execution("cancelling").await;
    let receipt = f
        .service
        .terminate(&f.scope, id, None)
        .await
        .expect("accepted");
    assert!(receipt.enqueued);
    assert_eq!(f.commands().await, vec![ControlCommand::Terminate]);
}
both_backends!(
    terminate_enqueues_terminate_even_when_cancelling_case,
    terminate_enqueues_terminate_even_when_cancelling
);

async fn every_outcome_is_counted_by_command_and_outcome(f: Fixture) {
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
both_backends!(
    every_outcome_is_counted_by_command_and_outcome_case,
    every_outcome_is_counted_by_command_and_outcome
);
