//! Operation-ledger conformance for the PostgreSQL deployment backend.
//!
//! PostgreSQL is a deployment backend, so its absence is a job failure, never a
//! silent substitution. With `NEBULA_REQUIRE_POSTGRES=1` and no `DATABASE_URL`,
//! every case fails; without it a developer without a database sees the cases
//! report the backend was unreachable and assert nothing.

#![cfg(feature = "postgres")]

#[macro_use]
#[path = "support/operation_ledger_oracle.rs"]
mod oracle;

use nebula_storage::postgres::{PgOperationLedger, init_schema};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::OnceCell;

static SCHEMA_READY: OnceCell<()> = OnceCell::const_new();

/// Connect to `DATABASE_URL` and apply the ordered migration catalog, or report
/// that PostgreSQL is unreachable.
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
        return;
    };
    let ledger = PgOperationLedger::new(pool.clone());
    let executions = nebula_storage::postgres::PgExecutionStore::new(pool.clone());
    let (_execution_id, slot, _fencing) = oracle::prepare_fresh(&ledger, &executions, 0xA4).await;
    let scope = oracle::scope();

    let mut writer = pool.begin().await.unwrap();
    sqlx::query(
        "SELECT slot_id FROM port_operation_ledger \
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
async fn legacy_upgrade_never_grants_and_terminal_evidence_survives_reopen() {
    use nebula_storage_port::dto::{
        AttemptGeneration, DestinationCapability, EffectSlotBinding, EffectSlotId,
        FrozenOutcomeEvidence, KnownOutcome, OperationAdvance, OperationCommand,
        OutcomeEvidenceSource, RequestFingerprint,
    };
    use nebula_storage_port::store::{ExecutionStore, OperationLedger};
    use std::borrow::Cow;
    use std::str::FromStr;
    let Some(admin) = pool().await else {
        return;
    };
    let schema = format!("ledger_upgrade_{}", uuid::Uuid::new_v4().simple());
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
    let mut previous = sqlx::migrate!("./migrations/postgres");
    previous.migrations = Cow::Owned(
        previous
            .migrations
            .iter()
            .filter(|migration| migration.version <= 48)
            .cloned()
            .collect(),
    );
    previous.run(&pool).await.unwrap();
    let scope = oracle::scope();
    let executions = nebula_storage::postgres::PgExecutionStore::new(pool.clone());
    executions
        .create(
            &scope,
            "upgrade-execution",
            "workflow",
            serde_json::json!({"status":"Created"}),
        )
        .await
        .unwrap();
    let slot = EffectSlotId::from_storage_bytes([0x91; 16]);
    sqlx::query("INSERT INTO port_operation_ledger(slot_id, workspace_id, org_id, execution_id, node_key, occurrence, attempt_generation, fingerprint_version, fingerprint, destination, operation_id, state, prepared_at_ms) VALUES($1,$2,$3,'upgrade-execution','node','legacy',0,1,$4,'stable_key',$5,'prepared',0)")
        .bind(slot.as_bytes().as_slice()).bind(&scope.workspace_id).bind(&scope.org_id).bind([0x11u8;32].as_slice()).bind([0x92u8;16].as_slice()).execute(&pool).await.unwrap();
    init_schema(&pool).await.unwrap();
    let ledger = PgOperationLedger::new(pool.clone());
    let fence = executions
        .acquire_lease(
            &scope,
            "upgrade-execution",
            "runner",
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
    let legacy = ledger.read_exact(&scope, slot).await.unwrap();
    assert!(legacy.protocol().is_none());
    assert!(
        ledger
            .advance(
                &scope,
                slot,
                fence,
                &OperationCommand::GrantInvocation {
                    expected_revision: 0
                }
            )
            .await
            .is_err()
    );
    let binding = EffectSlotBinding {
        scope: &scope,
        execution_id: "upgrade-execution",
        node_key: "node",
        occurrence: "legacy",
        attempt_generation: AttemptGeneration::new(1),
        fingerprint: RequestFingerprint::new(1, [0x11; 32]),
        destination: DestinationCapability::StableKey,
        contract: oracle::contract(DestinationCapability::StableKey),
    };
    assert_eq!(
        ledger
            .prepare(&binding, fence)
            .await
            .unwrap()
            .operation()
            .slot_id(),
        slot
    );
    assert!(
        ledger
            .read_exact(&scope, slot)
            .await
            .unwrap()
            .protocol()
            .is_none()
    );
    let fresh = EffectSlotBinding {
        occurrence: "fresh",
        ..binding
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
        "SELECT COUNT(*) FROM port_execution_journal WHERE execution_id = 'upgrade-execution'",
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
