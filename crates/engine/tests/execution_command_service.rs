//! `ExecutionCommandService`: the §12.2 control contract, once, for every surface.
//!
//! Cases exercise each production SQLite and PostgreSQL adapter alongside the
//! in-memory reference adapter. Required PG evidence fails without its database.

#[path = "support/postgres_schema.rs"]
mod postgres_schema;

use std::sync::Arc;

use nebula_core::ExecutionId;
use nebula_engine::{ExecutionCommandError, ExecutionCommandService, Signal};
use nebula_metrics::{
    MetricsRegistry,
    naming::{NEBULA_ENGINE_EXECUTION_COMMAND_TOTAL, execution_command_outcome as outcome},
};
use nebula_storage_port::{
    Scope, StorageError,
    dto::ControlCommand,
    store::{ControlQueue, ExecutionStore},
};
use serde_json::json;

struct Fixture {
    service: ExecutionCommandService,
    producer: Arc<dyn nebula_storage_port::store::ResumeProducer>,
    store: Arc<dyn ExecutionStore>,
    queue: Arc<dyn ControlQueue>,
    scope: Scope,
}

impl Fixture {
    fn in_memory() -> Self {
        let store = Arc::new(nebula_storage::InMemoryExecutionStore::new());
        let queue = Arc::new(nebula_storage::InMemoryControlQueue::new(&store));
        let producer = Arc::new(store.resume_producer());
        Self::compose(store, queue, producer)
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
            Arc::new(nebula_storage::sqlite::SqliteControlQueue::new(
                pool.clone(),
            )),
            Arc::new(nebula_storage::sqlite::SqliteResumeProducer::new(pool)),
        )
    }

    async fn postgres() -> Option<Self> {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            assert!(
                std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
                "PostgreSQL command-service evidence requires DATABASE_URL"
            );
            return None;
        };
        let pool = postgres_schema::connect_with_private_schema(&url, "execution_commands")
            .await
            .expect("private PostgreSQL schema opens");
        nebula_storage::postgres::init_schema(&pool)
            .await
            .expect("port schema installs");
        Some(Self::compose(
            Arc::new(nebula_storage::postgres::PgExecutionStore::new(
                pool.clone(),
            )),
            Arc::new(nebula_storage::postgres::PgControlQueue::new(pool.clone())),
            Arc::new(nebula_storage::postgres::PgResumeProducer::new(pool)),
        ))
    }

    fn compose(
        store: Arc<dyn ExecutionStore>,
        queue: Arc<dyn ControlQueue>,
        producer: Arc<dyn nebula_storage_port::store::ResumeProducer>,
    ) -> Self {
        Self {
            service: ExecutionCommandService::new(Arc::clone(&store), Arc::clone(&queue)),
            store,
            queue,
            producer,
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

    async fn park_with_resume_tokens(
        &self,
        id: ExecutionId,
        rows: Vec<nebula_storage_port::dto::resume_token::ResumeTokenRow>,
    ) {
        use nebula_storage_port::{TransitionBatch, TransitionOutcome};
        let fencing = self
            .store
            .acquire_lease(
                &self.scope,
                &id.to_string(),
                "seed",
                std::time::Duration::from_secs(30),
            )
            .await
            .unwrap()
            .unwrap();
        let batch = TransitionBatch::builder()
            .scope(self.scope.clone())
            .execution_id(id.to_string())
            .expected_version(0)
            .fencing(fencing)
            .new_state(json!({"status":"paused", "workflow_id":"wf_1"}))
            .resume_tokens(rows)
            .build()
            .unwrap();
        assert!(matches!(
            self.store.commit(batch).await.unwrap(),
            TransitionOutcome::Applied { .. }
        ));
        self.store
            .release_lease(&self.scope, &id.to_string(), fencing)
            .await
            .unwrap();
    }

    /// The persisted `status` — read straight from the port, never the service.
    async fn status(&self, id: ExecutionId) -> serde_json::Value {
        self.store
            .get(&self.scope, &id.to_string())
            .await
            .expect("store is readable")
            .expect("row exists")
            .state["status"]
            .clone()
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

/// Run each case against the reference adapter and both deployment backends.
macro_rules! both_backends {
    ($name:ident, $body:ident) => {
        mod $name {
            use super::*;

            #[tokio::test]
            async fn in_memory() {
                $body(Fixture::in_memory()).await;
            }

            #[tokio::test]
            async fn postgres() {
                if let Some(fixture) = Fixture::postgres().await {
                    $body(fixture).await;
                }
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

/// Every signal enqueues one identity-targeted `Resume`: the approver is the
/// calling user, never a payload; nothing untargeted or webhook-shaped is ever
/// produced (an untargeted Resume would arm every signal wait, approval and
/// webhook gates included).
async fn signals_target_by_caller_authority(f: Fixture) {
    use nebula_core::{Principal, UserId};
    use nebula_storage_port::dto::ResumeTarget;

    let id = f.execution("paused").await;
    let context = nebula_core::W3cTraceContext::from_optional_headers(
        Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
        None,
    )
    .unwrap()
    .unwrap();
    let user = UserId::new();
    let caller = Principal::User(user);
    // A completion signal names an execution that has durably completed.
    let awaited = f.execution("completed").await;
    let approval = f
        .service
        .signal(
            &f.scope,
            id,
            &caller,
            Signal::Approval,
            Some(context.clone()),
        )
        .await
        .unwrap();
    let completed = f
        .service
        .signal(
            &f.scope,
            id,
            &caller,
            Signal::ExecutionCompleted {
                execution_id: awaited,
            },
            Some(context.clone()),
        )
        .await
        .unwrap();
    assert!(approval.enqueued && completed.enqueued);
    assert_eq!(f.status(id).await, "paused", "the service never writes");
    let messages = f.queue.claim_pending(&[8; 16], 100).await.unwrap();
    assert_eq!(messages.len(), 2);
    assert!(messages.iter().all(|claim| claim.msg.scope == f.scope
        && claim.msg.command == ControlCommand::Resume
        && claim.msg.w3c_traceparent.as_deref() == Some(context.traceparent())));
    let targets: Vec<_> = messages
        .iter()
        .map(|claim| claim.msg.resume_target.clone())
        .collect();
    assert!(targets.contains(&Some(ResumeTarget::Approval {
        approver: user.to_string()
    })));
    assert!(targets.contains(&Some(ResumeTarget::Execution {
        execution_id: awaited.to_string()
    })));

    // Only a user can approve; the refusal enqueues nothing.
    for principal in [
        Principal::System,
        Principal::ServiceAccount(nebula_core::ServiceAccountId::new()),
    ] {
        assert!(matches!(
            f.service
                .signal(&f.scope, id, &principal, Signal::Approval, None)
                .await,
            Err(ExecutionCommandError::ApproverNotUser)
        ));
    }
    let other = Scope::new("other", "other");
    assert!(matches!(
        f.service
            .signal(&other, id, &caller, Signal::Approval, None)
            .await,
        Err(ExecutionCommandError::NotFound(_))
    ));

    // A completion claim is a durable fact, never the caller's word: an
    // unknown execution and one still running are both refused.
    let unknown = ExecutionId::new();
    assert!(matches!(
        f.service
            .signal(
                &f.scope,
                id,
                &caller,
                Signal::ExecutionCompleted {
                    execution_id: unknown
                },
                None,
            )
            .await,
        Err(ExecutionCommandError::NotFound(missing)) if missing == unknown
    ));
    let running = f.execution("running").await;
    assert!(matches!(
        f.service
            .signal(
                &f.scope,
                id,
                &caller,
                Signal::ExecutionCompleted {
                    execution_id: running
                },
                None,
            )
            .await,
        Err(ExecutionCommandError::AwaitedNotTerminal { execution_id, .. }) if execution_id == running
    ));
    // The awaited execution must live in the caller's scope.
    assert!(matches!(
        f.service
            .signal(
                &other,
                id,
                &caller,
                Signal::ExecutionCompleted {
                    execution_id: awaited
                },
                None,
            )
            .await,
        Err(ExecutionCommandError::NotFound(missing)) if missing == awaited
    ));
    let done = f.execution("completed").await;
    assert!(matches!(
        f.service
            .signal(&f.scope, done, &caller, Signal::Approval, None)
            .await,
        Err(ExecutionCommandError::Terminal { verb: "signal", .. })
    ));
    assert!(
        f.commands().await.is_empty(),
        "refused commands enqueue nothing"
    );
}
both_backends!(
    signals_target_by_caller_authority_case,
    signals_target_by_caller_authority
);

async fn webhook_resume_consumes_only_valid_tokens_and_enqueues_once(f: Fixture) {
    use nebula_storage_port::dto::resume_token::{ResumeTokenRow, ResumeTokenWaitKind, TokenHash};
    let id = f.execution("paused").await;
    let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(2_000_000_000);
    let hashes = (1..=4)
        .map(|byte| TokenHash::try_from_bytes(vec![byte; 32]).unwrap())
        .collect::<Vec<_>>();
    // One token per parked node (the store keeps one per `(execution, node)`):
    // 0 is the valid webhook token; 1 is an approval token; 2 has expired;
    // 3 expires exactly at `now` (2033-05-18T03:33:20Z). A malformed expiry
    // cannot be stored by every backend; it is covered with a stub producer.
    let rows = hashes
        .iter()
        .enumerate()
        .map(|(index, hash)| {
            ResumeTokenRow::new(
                hash.clone(),
                f.scope.clone(),
                id.to_string(),
                format!("wait_{index}"),
                if index == 1 {
                    ResumeTokenWaitKind::Approval
                } else {
                    ResumeTokenWaitKind::Webhook
                },
                "callback".to_owned(),
                "2026-01-01T00:00:00Z".to_owned(),
                match index {
                    2 => Some("2020-01-01T00:00:00Z".to_owned()),
                    3 => Some("2033-05-18T03:33:20Z".to_owned()),
                    _ => None,
                },
            )
        })
        .collect();
    f.park_with_resume_tokens(id, rows).await;
    let metrics = MetricsRegistry::new();
    let service = f
        .service
        .clone()
        .with_resume_producer(f.producer.clone())
        .with_metrics(metrics.clone());
    for hash in &hashes[1..] {
        assert!(
            f.producer.peek(hash).await.unwrap().is_some(),
            "token was parked"
        );
        assert!(matches!(
            service.resume_webhook(hash, now, None).await,
            Err(ExecutionCommandError::ResumeTokenNotFound)
        ));
        assert!(
            f.producer.peek(hash).await.unwrap().is_some(),
            "refused token remains live"
        );
    }
    let forged = TokenHash::try_from_bytes(vec![0xEE; 32]).unwrap();
    assert!(matches!(
        service.resume_webhook(&forged, now, None).await,
        Err(ExecutionCommandError::ResumeTokenNotFound)
    ));
    let context = nebula_core::W3cTraceContext::from_optional_headers(
        Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
        None,
    )
    .unwrap()
    .unwrap();
    let attempts: [_; 2] = tokio::join!(
        service.resume_webhook(&hashes[0], now, Some(context.clone())),
        service.resume_webhook(&hashes[0], now, Some(context.clone())),
    )
    .into();
    assert_eq!(
        attempts
            .iter()
            .filter(|result| matches!(result, Ok(scope) if *scope == f.scope))
            .count(),
        1
    );
    assert_eq!(
        attempts
            .iter()
            .filter(|result| matches!(result, Err(ExecutionCommandError::ResumeTokenNotFound)))
            .count(),
        1
    );
    assert!(
        f.producer.peek(&hashes[0]).await.unwrap().is_none(),
        "the accepted token is burned"
    );
    // Bearer traffic never lands in an authenticated command's series.
    for command in ["resume", "signal"] {
        for outcome in [outcome::ENQUEUED, outcome::NOT_FOUND] {
            let labels = metrics
                .interner()
                .label_set(&[("command", command), ("outcome", outcome)]);
            assert_eq!(
                metrics
                    .counter_labeled(NEBULA_ENGINE_EXECUTION_COMMAND_TOTAL, &labels)
                    .unwrap()
                    .get(),
                0,
                "{command}/{outcome}"
            );
        }
    }
    // 3 refused rows + 1 forged bearer + 1 losing replay.
    for (outcome, expected) in [(outcome::ENQUEUED, 1), (outcome::NOT_FOUND, 5)] {
        let labels = metrics
            .interner()
            .label_set(&[("command", "resume_webhook"), ("outcome", outcome)]);
        assert_eq!(
            metrics
                .counter_labeled(NEBULA_ENGINE_EXECUTION_COMMAND_TOTAL, &labels)
                .unwrap()
                .get(),
            expected
        );
    }
    let claims = f.queue.claim_pending(&[9; 16], 100).await.unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].msg.execution_id, id.to_string());
    assert_eq!(claims[0].msg.scope, f.scope);
    assert_eq!(claims[0].msg.command, ControlCommand::Resume);
    assert_eq!(
        claims[0].msg.w3c_traceparent.as_deref(),
        Some(context.traceparent())
    );
    assert_eq!(
        claims[0].msg.resume_target,
        Some(nebula_storage_port::dto::ResumeTarget::Webhook {
            callback_id: "callback".to_owned()
        })
    );
    assert_eq!(f.status(id).await, "paused", "the service never writes");
}
both_backends!(
    webhook_resume_consumes_only_valid_tokens_and_enqueues_once_case,
    webhook_resume_consumes_only_valid_tokens_and_enqueues_once
);

/// A resume producer whose backend is down: `peek` returns `row` when set and
/// fails otherwise; the atomic consume always fails with a connection error.
#[derive(Debug)]
struct UnreachableProducer {
    row: Option<nebula_storage_port::dto::resume_token::ResumeTokenRow>,
}

#[async_trait::async_trait]
impl nebula_storage_port::store::ResumeProducer for UnreachableProducer {
    async fn peek(
        &self,
        _hash: &nebula_storage_port::dto::resume_token::TokenHash,
    ) -> Result<Option<nebula_storage_port::dto::resume_token::ResumeTokenRow>, StorageError> {
        match &self.row {
            Some(row) => Ok(Some(row.clone())),
            None => Err(StorageError::Connection("peek: backend down".to_owned())),
        }
    }

    async fn consume_and_enqueue_resume(
        &self,
        _hash: &nebula_storage_port::dto::resume_token::TokenHash,
        _resume_msg: &nebula_storage_port::dto::ControlMsg,
    ) -> Result<bool, StorageError> {
        Err(StorageError::Connection("consume: backend down".to_owned()))
    }
}

/// The storage fault behind a failed webhook resume stays reachable as the
/// typed `source()`, and each failure is counted under its own outcome.
#[tokio::test]
async fn webhook_resume_failures_keep_the_storage_cause() {
    use nebula_storage_port::dto::resume_token::{ResumeTokenRow, ResumeTokenWaitKind, TokenHash};
    use std::error::Error as _;

    let f = Fixture::in_memory();
    let metrics = MetricsRegistry::new();
    let hash = TokenHash::try_from_bytes(vec![1; 32]).unwrap();
    let now = std::time::SystemTime::now();

    let unwired = f.service.clone().with_metrics(metrics.clone());
    assert!(matches!(
        unwired.resume_webhook(&hash, now, None).await,
        Err(ExecutionCommandError::ResumeUnwired)
    ));

    let peek_down = f
        .service
        .clone()
        .with_metrics(metrics.clone())
        .with_resume_producer(Arc::new(UnreachableProducer { row: None }));
    let error = peek_down
        .resume_webhook(&hash, now, None)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, ExecutionCommandError::Store(StorageError::Connection(detail)) if detail == "peek: backend down"),
        "{error:?}"
    );
    assert!(
        error
            .source()
            .and_then(|cause| cause.downcast_ref::<StorageError>())
            .is_some(),
        "the storage error is the typed source"
    );
    assert!(
        !error.to_string().contains("backend down"),
        "the cause is reported once, as the source — not repeated in the message: {error}"
    );

    let row = ResumeTokenRow::new(
        hash.clone(),
        f.scope.clone(),
        ExecutionId::new().to_string(),
        "wait".to_owned(),
        ResumeTokenWaitKind::Webhook,
        "callback".to_owned(),
        "2026-01-01T00:00:00Z".to_owned(),
        None,
    );
    let consume_down = f
        .service
        .clone()
        .with_metrics(metrics.clone())
        .with_resume_producer(Arc::new(UnreachableProducer { row: Some(row) }));
    let error = consume_down
        .resume_webhook(&hash, now, None)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, ExecutionCommandError::QueueUnavailable(StorageError::Connection(detail)) if detail == "consume: backend down"),
        "{error:?}"
    );
    assert!(
        error
            .source()
            .and_then(|cause| cause.downcast_ref::<StorageError>())
            .is_some()
    );

    // A malformed expiry fails closed before the atomic consume is attempted
    // (that consume would surface as `QueueUnavailable`, not `NotFound`).
    let malformed = ResumeTokenRow::new(
        hash.clone(),
        f.scope.clone(),
        ExecutionId::new().to_string(),
        "wait".to_owned(),
        ResumeTokenWaitKind::Webhook,
        "callback".to_owned(),
        "2026-01-01T00:00:00Z".to_owned(),
        Some("not-a-timestamp".to_owned()),
    );
    let malformed_expiry = f
        .service
        .clone()
        .with_metrics(metrics.clone())
        .with_resume_producer(Arc::new(UnreachableProducer {
            row: Some(malformed),
        }));
    assert!(matches!(
        malformed_expiry.resume_webhook(&hash, now, None).await,
        Err(ExecutionCommandError::ResumeTokenNotFound)
    ));

    let count = |outcome: &str| {
        let labels = metrics
            .interner()
            .label_set(&[("command", "resume_webhook"), ("outcome", outcome)]);
        metrics
            .counter_labeled(NEBULA_ENGINE_EXECUTION_COMMAND_TOTAL, &labels)
            .unwrap()
            .get()
    };
    assert_eq!(count(outcome::UNAVAILABLE), 2, "unwired + consume down");
    assert_eq!(count(outcome::FAILED), 1, "peek read failure");
    assert_eq!(count(outcome::NOT_FOUND), 1, "malformed expiry");
    assert_eq!(count(outcome::ENQUEUED), 0);
    assert!(f.commands().await.is_empty());
}
