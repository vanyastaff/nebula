//! Start-authority materialization contract on the PostgreSQL adapter.
//!
//! PostgreSQL evidence binary: every case needs a live database and fails loudly
//! without one, so a green run always means the cases ran. Excluded from the
//! default nextest profile and collected by the CI runtime-authority evidence
//! step. The in-memory and SQLite arms live in `start_materialization`.

#![cfg(feature = "postgres")]

#[path = "support/start_materialization_common.rs"]
mod common;
#[path = "support/execution_parents.rs"]
#[expect(
    dead_code,
    reason = "the oracle seeds through store handles, not through a raw pool"
)]
mod execution_parents;
#[path = "support/start_materialization_oracle.rs"]
mod oracle;

use common::write_observations;
use nebula_storage_port::store::StartAcceptanceStore;

#[path = "support/postgres_schema.rs"]
mod postgres_schema;

#[tokio::test]
async fn postgres_materialization_contract() {
    // An evidence binary never passes without a database: absence is a loud
    // failure, not a silent skip.
    let url = std::env::var("DATABASE_URL").expect(
        "start_materialization_postgres needs a live PostgreSQL: DATABASE_URL is unset or not Unicode",
    );
    let pool = postgres_schema::connect_with_private_schema(&url, "start_materialization")
        .await
        .unwrap();
    nebula_storage::postgres::init_schema(&pool).await.unwrap();
    let starts = nebula_storage::postgres::PgStartAcceptanceStore::new(pool.clone());
    let executions = nebula_storage::postgres::PgExecutionStore::new(pool.clone());
    let queue = nebula_storage::postgres::PgControlQueue::new(pool.clone());
    let catalog = nebula_storage::postgres::PgPlanFlavorCatalog::new(
        pool.clone(),
        &nebula_metrics::MetricsRegistry::new(),
    );
    let parents = oracle::Parents::new(Some((
        std::sync::Arc::new(nebula_storage::postgres::PgTenantProvisioningStore::new(
            pool.clone(),
        )),
        std::sync::Arc::new(nebula_storage::postgres::PgWorkflowStore::new(pool.clone())),
    )));
    let evidence = oracle::run(&starts, &executions, &queue, &catalog, &catalog, &parents).await;
    write_observations(
        "postgresql",
        "NEBULA_START_AUTHORITY_POSTGRES_OBSERVATIONS_PATH",
        evidence.observations,
    );
    let stored = evidence.stored;
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&pool)
        .await
        .unwrap();
    oracle::trigger_replay(&starts, &executions, &queue, &catalog, &catalog, &parents).await;
    pool.close().await;
    let options = url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .unwrap()
        .options([("search_path", schema)]);
    let reopened = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let starts = nebula_storage::postgres::PgStartAcceptanceStore::new(reopened);
    assert_eq!(
        starts
            .read_contract_bundle(stored.scope(), stored.execution_id())
            .await
            .unwrap(),
        Some(stored)
    );
}
