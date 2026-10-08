//! Operation-ledger conformance for the PostgreSQL deployment backend.
//!
//! PostgreSQL is a deployment backend, so its absence is a job failure, never a
//! silent substitution. With `NEBULA_REQUIRE_POSTGRES=1` and no `DATABASE_URL`,
//! every case fails; without it a developer without a database still sees every
//! case fail loudly naming the unreachable backend. A green run of this runner
//! therefore always means the cases ran against a live database — a run that
//! asserts nothing cannot look green.

#![cfg(feature = "postgres")]

#[macro_use]
#[path = "support/operation_ledger_oracle.rs"]
mod oracle;

#[path = "support/execution_parents.rs"]
mod execution_parents;

use execution_parents::SeedExecutionParents;
use nebula_storage::postgres::{PgOperationLedger, init_schema};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::OnceCell;

static SCHEMA_READY: OnceCell<()> = OnceCell::const_new();

/// Connect to `DATABASE_URL` and apply the ordered migration catalog, or return
/// `None`, which every case turns into a loud failure naming the backend.
///
/// The oracle folds a per-process namespace into every execution identity, so
/// cases share one database without meeting an earlier run's slots.
async fn pool() -> Option<PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            assert_ne!(
                std::env::var("NEBULA_REQUIRE_POSTGRES").as_deref(),
                Ok("1"),
                "DATABASE_URL must be set when NEBULA_REQUIRE_POSTGRES=1: \
                 PostgreSQL is a deployment backend and is never substituted"
            );
            return None;
        },
        Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
    };

    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    SCHEMA_READY
        .get_or_init(|| async {
            init_schema(&pool)
                .await
                .expect("apply the ordered PostgreSQL migration catalog");
            for scope in [oracle::scope(), oracle::other_scope()] {
                pool.seed_execution_parents(&scope, oracle::WORKFLOW).await;
            }
        })
        .await;
    Some(pool)
}

async fn ledger() -> Option<(
    PgOperationLedger,
    nebula_storage::postgres::PgExecutionStore,
)> {
    pool().await.map(|pool| {
        (
            PgOperationLedger::new(pool.clone()),
            nebula_storage::postgres::PgExecutionStore::new(pool),
        )
    })
}

operation_ledger_conformance_suite!(ledger());

#[tokio::test]
async fn exact_read_does_not_wait_for_a_concurrent_row_writer() {
    use nebula_storage_port::store::OperationLedger;

    let Some(pool) = pool().await else {
        panic!(
            "exact_read_does_not_wait_for_a_concurrent_row_writer: backend unreachable — the \
             case cannot run and must fail rather than pass unchecked; reach the backend (set \
             DATABASE_URL for postgres) or run without this feature"
        );
    };
    let ledger = PgOperationLedger::new(pool.clone());
    let executions = nebula_storage::postgres::PgExecutionStore::new(pool.clone());
    let (_execution_id, slot, _fencing) = oracle::prepare_fresh(&ledger, &executions, 0xA4).await;
    let scope = oracle::scope();

    let mut writer = pool.begin().await.unwrap();
    sqlx::query(
        "SELECT slot_id FROM operation_ledger \
         WHERE workspace_id = $1 AND org_id = $2 AND slot_id = $3 FOR UPDATE",
    )
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .bind(slot.as_bytes().as_slice())
    .fetch_one(&mut *writer)
    .await
    .unwrap();

    let observed = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        ledger.read_exact(&scope, slot),
    )
    .await
    .expect("an exact observation must not wait for a concurrent row writer")
    .unwrap();
    assert_eq!(observed.operation().slot_id(), slot);
    writer.rollback().await.unwrap();
}

#[tokio::test]
async fn terminal_evidence_commits_with_the_journal_and_survives_reopen() {
    use nebula_storage_port::dto::{
        AttemptGeneration, DestinationCapability, EffectSlotBinding, FrozenOutcomeEvidence,
        KnownOutcome, OperationAdvance, OperationCommand, OutcomeEvidenceSource,
        RequestFingerprint,
    };
    use nebula_storage_port::store::{ExecutionStore, OperationLedger};
    use std::str::FromStr;
    let Some(admin) = pool().await else {
        panic!(
            "terminal_evidence_commits_with_the_journal_and_survives_reopen: backend \
             unreachable — the case cannot run and must fail rather than pass unchecked; \
             reach the backend (set DATABASE_URL for postgres) or run without this feature"
        );
    };
    let schema = format!("ledger_reopen_{}", uuid::Uuid::new_v4().simple());
    // The identifier contains only the fixed prefix and generated UUID hex.
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let url = std::env::var("DATABASE_URL").unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&url)
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options.clone())
        .await
        .unwrap();
    init_schema(&pool).await.unwrap();
    let scope = oracle::scope();
    pool.seed_execution_parents(&scope, oracle::WORKFLOW).await;
    let executions = nebula_storage::postgres::PgExecutionStore::new(pool.clone());
    executions
        .create(
            &scope,
            "reopen-execution",
            "workflow",
            serde_json::json!({"status":"Created"}),
        )
        .await
        .unwrap();
    let ledger = PgOperationLedger::new(pool.clone());
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
    let reopened = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let stored = PgOperationLedger::new(reopened.clone())
        .read_exact(&scope, fresh_slot)
        .await
        .unwrap();
    assert_eq!(stored.protocol().unwrap().evidence(), Some(&evidence));
    assert!(!format!("{stored:?}").contains("reopen-canary"));
    reopened.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
}
