//! Storage-boundary oracle for atomic Control Start ownership transfer on the
//! in-memory and SQLite adapters.
//!
//! The PostgreSQL arm lives in `control_start_handoff_postgres` (an evidence
//! binary that needs a live database).

#[path = "support/turn_recovery_oracle.rs"]
mod recovery_oracle;

#[path = "support/control_turn_oracle.rs"]
mod control_turn_oracle;

include!("support/control_start_handoff_oracle.rs");

#[tokio::test]
async fn in_memory_control_start_handoff() {
    use nebula_storage::inmem::*;
    let execution = Arc::new(InMemoryExecutionStore::new());
    let catalog = Arc::new(execution.plan_flavor_catalog());
    oracle(Ports {
        journal: Arc::new(InMemoryJournalReader::new(&execution)),
        jobs: Arc::new(InMemoryJobDispatchQueue::new(&execution)),
        queue: Arc::new(InMemoryControlQueue::new(&execution)),
        handoff: Arc::new(InMemoryTurnHandoff::new(&execution)),
        recovery: Arc::new(InMemoryTurnHandoff::new(&execution)),
        starts: Arc::new(InMemoryStartAcceptanceStore::new(&execution)),
        execution,
        catalog: catalog.clone(),
        admin: catalog,
        parents: None,
    })
    .await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_control_start_handoff() {
    use nebula_storage::sqlite::*;
    let directory = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(directory.path().join("handoff.db"))
        .create_if_missing(true)
        .busy_timeout(Duration::from_secs(10));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    init_schema(&pool).await.unwrap();
    let catalog = Arc::new(SqlitePlanFlavorCatalog::new(
        pool.clone(),
        &nebula_metrics::MetricsRegistry::new(),
    ));
    oracle(Ports {
        journal: Arc::new(SqliteJournalReader::new(pool.clone())),
        jobs: Arc::new(SqliteJobDispatchQueue::new(pool.clone())),
        execution: Arc::new(SqliteExecutionStore::new(pool.clone())),
        queue: Arc::new(SqliteControlQueue::new(pool.clone())),
        handoff: Arc::new(SqliteTurnHandoff::new(pool.clone())),
        recovery: Arc::new(SqliteTurnHandoff::new(pool.clone())),
        starts: Arc::new(SqliteStartAcceptanceStore::new(pool.clone())),
        catalog: catalog.clone(),
        admin: catalog,
        parents: Some((
            Arc::new(SqliteTenantProvisioningStore::new(pool.clone())),
            Arc::new(SqliteWorkflowStore::new(pool)),
        )),
    })
    .await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_refusals_survive_observation_faults() {
    use nebula_storage::sqlite::*;
    let directory = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(directory.path().join("faults.db"))
        .create_if_missing(true)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(10));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    init_schema(&pool).await.unwrap();
    let catalog = Arc::new(SqlitePlanFlavorCatalog::new(
        pool.clone(),
        &nebula_metrics::MetricsRegistry::new(),
    ));
    let ports = Ports {
        journal: Arc::new(SqliteJournalReader::new(pool.clone())),
        jobs: Arc::new(SqliteJobDispatchQueue::new(pool.clone())),
        execution: Arc::new(SqliteExecutionStore::new(pool.clone())),
        queue: Arc::new(SqliteControlQueue::new(pool.clone())),
        handoff: Arc::new(SqliteTurnHandoff::new(pool.clone())),
        recovery: Arc::new(SqliteTurnHandoff::new(pool.clone())),
        starts: Arc::new(SqliteStartAcceptanceStore::new(pool.clone())),
        catalog: catalog.clone(),
        admin: catalog,
        parents: Some((
            Arc::new(SqliteTenantProvisioningStore::new(pool.clone())),
            Arc::new(SqliteWorkflowStore::new(pool.clone())),
        )),
    };
    control_turn_oracle::refusals_survive_observation_faults(&ports, async |fault| {
        use control_turn_oracle::ObservationFault;
        let statements: &[&'static str] = match fault {
            ObservationFault::FailReceiptWrite => &[
                "CREATE TRIGGER fault_fail_receipt BEFORE INSERT ON execution_control_observation_receipts BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END",
            ],
            ObservationFault::FailJournalWrite => &[
                "CREATE TRIGGER fault_fail_journal BEFORE INSERT ON execution_journal BEGIN SELECT RAISE(ABORT, 'injected journal failure'); END",
            ],
            // A deferred foreign-key violation fails only at COMMIT, after
            // every statement of the refusal transaction succeeded.
            ObservationFault::LoseCommitAcknowledgement => &[
                "CREATE TABLE IF NOT EXISTS fault_parent (id INTEGER PRIMARY KEY)",
                "CREATE TABLE IF NOT EXISTS fault_child (parent INTEGER REFERENCES fault_parent(id) DEFERRABLE INITIALLY DEFERRED)",
                "CREATE TRIGGER fault_lose_commit AFTER INSERT ON execution_journal BEGIN INSERT INTO fault_child VALUES (999); END",
            ],
            // CHECK constraints are bypassed on this one connection only, so
            // the stored snapshot stays present but undecodable.
            ObservationFault::CorruptReceiptSnapshot => &[
                "PRAGMA ignore_check_constraints = ON",
                "UPDATE execution_control_observation_receipts SET expected_flavor_id = X'01' WHERE outcome = 'flavor-mismatch'",
                "PRAGMA ignore_check_constraints = OFF",
            ],
            ObservationFault::Clear => &[
                "DROP TRIGGER IF EXISTS fault_fail_receipt",
                "DROP TRIGGER IF EXISTS fault_lose_commit",
                "DROP TRIGGER IF EXISTS fault_fail_journal",
            ],
        };
        let mut connection = pool.acquire().await.unwrap();
        for statement in statements {
            sqlx::query(*statement)
                .execute(&mut *connection)
                .await
                .unwrap();
        }
    })
    .await;
}
