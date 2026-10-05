//! Storage-boundary oracle for atomic Control Start ownership transfer on the
//! PostgreSQL adapter.
//!
//! PostgreSQL evidence binary: every case needs a live database and fails loudly
//! without one, so a green run always means the cases ran. Excluded from the
//! default nextest profile and collected by the CI `postgres-conformance` job.
//! The in-memory and SQLite arms live in `control_start_handoff`.

#![cfg(feature = "postgres")]

#[path = "support/turn_recovery_oracle.rs"]
mod recovery_oracle;

#[path = "support/control_turn_oracle.rs"]
mod control_turn_oracle;

include!("support/control_start_handoff_oracle.rs");

#[path = "support/postgres_schema.rs"]
mod postgres_schema;

#[tokio::test]
async fn postgres_control_start_handoff() {
    use nebula_storage::postgres::*;
    // An evidence binary never passes without a database: absence is a loud
    // failure, not a silent skip.
    let url = std::env::var("DATABASE_URL").expect(
        "control_start_handoff_postgres needs a live PostgreSQL: DATABASE_URL is unset or not Unicode",
    );
    let pool = postgres_schema::connect_with_private_schema(&url, "control_start_handoff")
        .await
        .unwrap();
    init_schema(&pool).await.unwrap();
    let catalog = Arc::new(PgPlanFlavorCatalog::new(
        pool.clone(),
        &nebula_metrics::MetricsRegistry::new(),
    ));
    oracle(Ports {
        journal: Arc::new(PgJournalReader::new(pool.clone())),
        jobs: Arc::new(PgJobDispatchQueue::new(pool.clone())),
        execution: Arc::new(PgExecutionStore::new(pool.clone())),
        queue: Arc::new(PgControlQueue::new(pool.clone())),
        handoff: Arc::new(PgTurnHandoff::new(pool.clone())),
        recovery: Arc::new(PgTurnHandoff::new(pool.clone())),
        starts: Arc::new(PgStartAcceptanceStore::new(pool)),
        catalog: catalog.clone(),
        admin: catalog,
    })
    .await;
}
