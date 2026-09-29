//! Workflow activation identity on the in-memory and SQLite adapters.
//!
//! The PostgreSQL arm lives in `workflow_activation_postgres` (an evidence binary
//! that needs a live database).

include!("support/workflow_activation_contracts.rs");

#[tokio::test]
async fn in_memory_publication_is_admitted_and_atomic() {
    let execution = nebula_storage::InMemoryExecutionStore::new();
    let versions = nebula_storage::InMemoryWorkflowVersionStore::new();
    let rows = nebula_storage::InMemoryWorkflowStore::new_with_versions(&versions, &execution);
    let catalog = execution.plan_flavor_catalog();
    publication_contract(&rows, &versions, &catalog, &catalog).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_publication_is_admitted_and_atomic() {
    let directory = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(directory.path().join("workflow.db"))
        .create_if_missing(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&pool).await.unwrap();
    let rows = nebula_storage::sqlite::SqliteWorkflowStore::new(pool.clone());
    let versions = nebula_storage::sqlite::SqliteWorkflowVersionStore::new(pool.clone());
    let catalog = nebula_storage::sqlite::SqlitePlanFlavorCatalog::new(
        pool.clone(),
        &nebula_metrics::MetricsRegistry::new(),
    );
    let (scope, workflow_id, activation) =
        publication_contract(&rows, &versions, &catalog, &catalog).await;
    pool.close().await;
    let reopened = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let versions = nebula_storage::sqlite::SqliteWorkflowVersionStore::new(reopened);
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

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_upgrade_preserves_legacy_absence_and_rejects_partial_activation() {
    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    MIGRATOR.run_to(46, &pool).await.unwrap();
    sqlx::query("INSERT INTO port_workflows (id,workspace_id,org_id,version,slug,deleted) VALUES ('legacy','ws','org',1,'legacy',0)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO port_workflow_versions (workspace_id,org_id,workflow_id,number,published,pinned,definition) VALUES ('ws','org','legacy',1,1,0,'{}')").execute(&pool).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let versions = nebula_storage::sqlite::SqliteWorkflowVersionStore::new(pool.clone());
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
}

#[test]
fn legacy_workflow_version_has_no_activation_and_partial_identity_is_rejected() {
    let mut wire = serde_json::json!({"workflow_id":"workflow", "number":1, "published":true, "pinned":false, "definition":{}});
    let legacy: WorkflowVersionRecord = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(legacy.activation, None);
    wire["activation"] = serde_json::json!({"workflow_version_id":WorkflowVersionId::new()});
    assert!(serde_json::from_value::<WorkflowVersionRecord>(wire).is_err());
}

#[test]
fn activation_roundtrip_preserves_the_complete_exact_identity() {
    let activation = WorkflowActivation::new(
        WorkflowVersionId::new(),
        PlanFlavorRevisionIds::new(
            ExecutablePlanRevisionId::from_bytes([1; 32]),
            WorkerFlavorRevisionId::from_bytes([2; 32]),
        ),
    );
    let wire = serde_json::to_vec(&activation).unwrap();
    let decoded: WorkflowActivation = serde_json::from_slice(&wire).unwrap();
    assert_eq!(decoded, activation);
    assert_eq!(decoded.revisions(), activation.revisions());
}
