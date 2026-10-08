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
    nebula_storage::postgres::init_schema(&pool).await.unwrap();
    let tenants = nebula_storage::postgres::PgTenantProvisioningStore::new(pool.clone());
    let rows = nebula_storage::postgres::PgWorkflowStore::new(pool.clone());
    let versions = nebula_storage::postgres::PgWorkflowVersionStore::new(pool.clone());
    let catalog = nebula_storage::postgres::PgPlanFlavorCatalog::new(
        pool.clone(),
        &nebula_metrics::MetricsRegistry::new(),
    );
    let (scope, workflow_id, activation) =
        publication_contract(&tenants, &rows, &versions, &catalog, &catalog).await;
    // A workflow needs a live workspace: the foreign key proves existence,
    // the adapter proves liveness.
    let gone = Scope::new("gone-workspace", "gone-org");
    provision(&tenants, &gone).await;
    nebula_storage_port::store::WorkspaceStore::soft_delete(
        &nebula_storage::postgres::PgWorkspaceStore::new(pool.clone()),
        &gone.org_id,
        &gone.workspace_id,
    )
    .await
    .unwrap();
    assert!(matches!(
        rows.create(
            &gone,
            WorkflowRecord {
                id: "in-deleted-workspace".into(),
                scope: gone.clone(),
                version: 1,
                slug: "in-deleted-workspace".into(),
            },
        )
        .await,
        Err(nebula_storage_port::StorageError::NotFound {
            entity: "workspace",
            ..
        })
    ));
    // The activation identity is all three columns or none, and the digests
    // are exactly 32 bytes, whatever the writer.
    for statement in [
        "UPDATE workflow_versions SET activation_workflow_version_id = NULL",
        "UPDATE workflow_versions SET activation_executable_plan_id = '\\x00'::bytea \
         WHERE activation_executable_plan_id IS NOT NULL",
    ] {
        assert!(
            sqlx::query(statement).execute(&pool).await.is_err(),
            "the schema must reject `{statement}`"
        );
    }
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
