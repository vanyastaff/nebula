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
        starts: Arc::new(SqliteStartAcceptanceStore::new(pool)),
        catalog: catalog.clone(),
        admin: catalog,
    })
    .await;
}
