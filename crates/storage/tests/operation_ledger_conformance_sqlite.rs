//! Operation-ledger conformance for the SQLite deployment backend.
//!
//! Every case runs against a fresh in-memory database whose schema comes from
//! the ordered migration catalog, so the adapter is exercised against exactly
//! the `CHECK` constraints migration 0045 installs.

#![cfg(feature = "sqlite")]

#[macro_use]
#[path = "support/operation_ledger_oracle.rs"]
mod oracle;

#[path = "support/execution_parents.rs"]
mod execution_parents;

use std::str::FromStr;

use execution_parents::SeedExecutionParents;
use nebula_storage::sqlite::{SqliteOperationLedger, init_schema};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// An isolated in-memory database with the ordered migration catalog applied.
///
/// The shared cache keeps every pooled connection on the same database; a
/// private `:memory:` connection would give each one its own empty schema.
async fn fresh_pool() -> SqlitePool {
    let database = format!("nebula-ledger-{}", uuid::Uuid::new_v4());
    let url = format!("sqlite:file:{database}?mode=memory&cache=shared");
    let options = SqliteConnectOptions::from_str(&url)
        .expect("in-memory SQLite URL must parse")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .expect("connect to in-memory SQLite");
    init_schema(&pool)
        .await
        .expect("apply the ordered SQLite migration catalog");
    for scope in [oracle::scope(), oracle::other_scope()] {
        pool.seed_execution_parents(&scope, oracle::WORKFLOW).await;
    }
    pool
}

async fn ledger() -> Option<(
    SqliteOperationLedger,
    nebula_storage::sqlite::SqliteExecutionStore,
)> {
    let pool = fresh_pool().await;
    Some((
        SqliteOperationLedger::new(pool.clone()),
        nebula_storage::sqlite::SqliteExecutionStore::new(pool),
    ))
}

operation_ledger_conformance_suite!(ledger());

#[tokio::test]
async fn terminal_evidence_commits_with_the_journal_and_survives_reopen() {
    use nebula_storage_port::dto::{
        AttemptGeneration, DestinationCapability, EffectSlotBinding, FrozenOutcomeEvidence,
        KnownOutcome, OperationAdvance, OperationCommand, OutcomeEvidenceSource,
        RequestFingerprint,
    };
    use nebula_storage_port::store::{ExecutionStore, OperationLedger};
    let directory = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(directory.path().join("ledger.db"))
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    init_schema(&pool).await.unwrap();
    let scope = oracle::scope();
    pool.seed_execution_parents(&scope, oracle::WORKFLOW).await;
    let executions = nebula_storage::sqlite::SqliteExecutionStore::new(pool.clone());
    executions
        .create(
            &scope,
            "reopen-execution",
            "workflow",
            serde_json::json!({"status":"Created"}),
        )
        .await
        .unwrap();
    let ledger = SqliteOperationLedger::new(pool.clone());
    let fence = executions
        .acquire_lease(
            &scope,
            "reopen-execution",
            "runner",
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
    let fresh = EffectSlotBinding {
        scope: &scope,
        execution_id: "reopen-execution",
        node_key: "node",
        occurrence: "fresh",
        attempt_generation: AttemptGeneration::new(1),
        fingerprint: RequestFingerprint::new(1, [0x11; 32]),
        destination: DestinationCapability::StableKey,
        contract: oracle::contract(DestinationCapability::StableKey),
        provider_key: None,
        concurrent_with: None,
        observation: false,
    };
    let fresh_slot = ledger
        .prepare(&fresh, fence)
        .await
        .unwrap()
        .operation()
        .slot_id();
    let call = match ledger
        .advance(
            &scope,
            fresh_slot,
            fence,
            &OperationCommand::GrantInvocation {
                expected_revision: 0,
            },
        )
        .await
        .unwrap()
    {
        OperationAdvance::Granted { call, .. } => call,
        _ => panic!("fresh permit"),
    };
    let evidence = FrozenOutcomeEvidence::v1_json(
        OutcomeEvidenceSource::Invocation(call),
        KnownOutcome::Succeeded,
        b"{\"output\":\"reopen-canary\"}".to_vec(),
    )
    .unwrap();
    let command = OperationCommand::RecordOutcome(evidence.clone());
    ledger
        .advance(&scope, fresh_slot, fence, &command)
        .await
        .unwrap();
    ledger
        .advance(&scope, fresh_slot, fence, &command)
        .await
        .unwrap();
    let journal_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM execution_journal WHERE execution_id = 'reopen-execution'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        journal_count, 1,
        "terminal and journal commit together; exact recommit appends nothing"
    );
    drop(ledger);
    drop(executions);
    pool.close().await;
    let reopened = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let stored = SqliteOperationLedger::new(reopened.clone())
        .read_exact(&scope, fresh_slot)
        .await
        .unwrap();
    assert_eq!(stored.protocol().unwrap().evidence(), Some(&evidence));
    assert!(!format!("{stored:?}").contains("reopen-canary"));
    reopened.close().await;
}
