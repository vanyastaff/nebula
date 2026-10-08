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
    let tenants = nebula_storage::inmem::InMemoryIdentityDirectory::new();
    publication_contract(&tenants, &rows, &versions, &catalog, &catalog).await;
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
    let tenants = nebula_storage::sqlite::SqliteTenantProvisioningStore::new(pool.clone());
    let (scope, workflow_id, activation) =
        publication_contract(&tenants, &rows, &versions, &catalog, &catalog).await;
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

/// A workflow needs a live workspace: the foreign key proves existence, the
/// adapter proves liveness.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_workflow_needs_a_live_workspace() {
    use nebula_storage_port::store::WorkspaceStore;
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&pool).await.unwrap();
    let scope = Scope::new("gone-workspace", "gone-org");
    provision(
        &nebula_storage::sqlite::SqliteTenantProvisioningStore::new(pool.clone()),
        &scope,
    )
    .await;
    nebula_storage::sqlite::SqliteWorkspaceStore::new(pool.clone())
        .soft_delete(&scope.org_id, &scope.workspace_id)
        .await
        .unwrap();
    let rows = nebula_storage::sqlite::SqliteWorkflowStore::new(pool);
    let created = rows
        .create(
            &scope,
            WorkflowRecord {
                id: "in-deleted-workspace".into(),
                scope: scope.clone(),
                version: 1,
                slug: "in-deleted-workspace".into(),
            },
        )
        .await;
    assert!(matches!(
        created,
        Err(nebula_storage_port::StorageError::NotFound {
            entity: "workspace",
            ..
        })
    ));
}

/// The activation identity is all three columns or none, and the digests
/// are exactly 32 bytes, whatever the writer.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_schema_rejects_partial_or_malformed_activation() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    nebula_storage::sqlite::init_schema(&pool).await.unwrap();
    let scope = Scope::new("schema-workspace", "schema-org");
    provision(
        &nebula_storage::sqlite::SqliteTenantProvisioningStore::new(pool.clone()),
        &scope,
    )
    .await;
    let rows = nebula_storage::sqlite::SqliteWorkflowStore::new(pool.clone());
    let versions = nebula_storage::sqlite::SqliteWorkflowVersionStore::new(pool.clone());
    let row = WorkflowRecord {
        id: "draft".into(),
        scope: scope.clone(),
        version: 1,
        slug: "draft".into(),
    };
    let version = WorkflowVersionRecord {
        activation: None,
        workflow_id: "draft".into(),
        number: 1,
        published: true,
        pinned: false,
        definition: serde_json::json!({}),
    };
    rows.save_with_published_version(&scope, row, version, None)
        .await
        .unwrap();
    assert_eq!(
        versions
            .get(&scope, "draft", 1)
            .await
            .unwrap()
            .unwrap()
            .activation,
        None
    );
    for statement in [
        "UPDATE workflow_versions SET activation_workflow_version_id = 'only-one'",
        "UPDATE workflow_versions SET activation_workflow_version_id = 'v', \
         activation_executable_plan_id = zeroblob(31), activation_worker_flavor_id = zeroblob(32)",
        "UPDATE workflow_versions SET activation_workflow_version_id = 'v', \
         activation_executable_plan_id = zeroblob(32), activation_worker_flavor_id = 'text'",
    ] {
        assert!(
            sqlx::query(statement).execute(&pool).await.is_err(),
            "the schema must reject `{statement}`"
        );
    }
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
