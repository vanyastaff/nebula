//! Workflow activation identity on the PostgreSQL adapter.
//!
//! PostgreSQL evidence binary: every case needs a live database and fails loudly
//! without one, so a green run always means the cases ran. Excluded from the
//! default nextest profile and collected by the CI `postgres-conformance` job.
//! The in-memory and SQLite arms live in `workflow_activation`.

#![cfg(feature = "postgres")]

include!("support/workflow_activation_contracts.rs");

#[path = "support/postgres_schema.rs"]
mod postgres_schema;

#[tokio::test]
async fn postgres_publication_is_admitted_and_atomic() {
    // An evidence binary never passes without a database: absence is a loud
    // failure, not a silent skip.
    let url = std::env::var("DATABASE_URL").expect(
        "workflow_activation_postgres needs a live PostgreSQL: DATABASE_URL is unset or not Unicode",
    );
    let pool = postgres_schema::connect_with_private_schema(&url, "workflow_activation")
        .await
        .unwrap();
    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");
    MIGRATOR.run_to(46, &pool).await.unwrap();
    sqlx::query("INSERT INTO port_workflows (id,workspace_id,org_id,version,slug,deleted) VALUES ('legacy','ws','org',1,'legacy',FALSE)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO port_workflow_versions (workspace_id,org_id,workflow_id,number,published,pinned,definition) VALUES ('ws','org','legacy',1,TRUE,FALSE,'{}')").execute(&pool).await.unwrap();
    nebula_storage::postgres::init_schema(&pool).await.unwrap();
    let rows = nebula_storage::postgres::PgWorkflowStore::new(pool.clone());
    let versions = nebula_storage::postgres::PgWorkflowVersionStore::new(pool.clone());
    assert_eq!(
        versions
            .get(&Scope::new("ws", "org"), "legacy", 1)
            .await
            .unwrap()
            .unwrap()
            .activation,
        None
    );
    assert!(
        sqlx::query(
            "UPDATE port_workflow_versions SET activation = '{}' WHERE workflow_id = 'legacy'"
        )
        .execute(&pool)
        .await
        .is_err()
    );
    let catalog = nebula_storage::postgres::PgPlanFlavorCatalog::new(
        pool.clone(),
        &nebula_metrics::MetricsRegistry::new(),
    );
    let (scope, workflow_id, activation) =
        publication_contract(&rows, &versions, &catalog, &catalog).await;
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&pool)
        .await
        .unwrap();
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
    let versions = nebula_storage::postgres::PgWorkflowVersionStore::new(reopened);
    assert_eq!(
        versions
            .get_published(&scope, &workflow_id)
            .await
            .unwrap()
            .unwrap()
            .activation,
        Some(activation)
    );
}
