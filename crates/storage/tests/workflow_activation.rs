use nebula_core::{ExecutablePlanRevisionId, WorkerFlavorRevisionId, WorkflowVersionId};
use nebula_storage_port::dto::{PlanFlavorRevisionIds, WorkflowActivation, WorkflowVersionRecord};
use nebula_storage_port::dto::{
    PlanFlavorRevisionRecord, RevisionRecordBytes, WorkerFlavorRevisionRecord, WorkflowRecord,
};
use nebula_storage_port::store::{WorkflowPublicationError, WorkflowStore, WorkflowVersionStore};
use nebula_storage_port::{
    PlanFlavorCatalogAdmin, PlanFlavorCatalogWriter, PlanFlavorRevisionTarget, Scope,
};

async fn publication_contract(
    rows: &dyn WorkflowStore,
    versions: &dyn WorkflowVersionStore,
    writer: &dyn PlanFlavorCatalogWriter,
    admin: &dyn PlanFlavorCatalogAdmin,
) -> (Scope, String, WorkflowActivation) {
    draft_collision_contract(rows, versions, writer).await;
    let scope = Scope::new("activation-workspace", "activation-org");
    let workflow_id = nebula_core::WorkflowId::new();
    let workflow_revision = WorkflowVersionId::new();
    let activation = WorkflowActivation::new(
        workflow_revision,
        PlanFlavorRevisionIds::new(
            ExecutablePlanRevisionId::from_bytes([7; 32]),
            WorkerFlavorRevisionId::from_bytes([8; 32]),
        ),
    );
    let mut row = WorkflowRecord {
        id: workflow_id.to_string(),
        scope: scope.clone(),
        version: 1,
        slug: workflow_id.to_string(),
        deleted: false,
    };
    let mut version = WorkflowVersionRecord {
        activation: None,
        workflow_id: row.id.clone(),
        number: 1,
        published: true,
        pinned: false,
        definition: serde_json::json!({"kept":"original"}),
    };
    rows.save_with_published_version(&scope, row.clone(), version.clone(), None)
        .await
        .unwrap();
    row.version = 2;
    version.number = 2;
    version.activation = Some(activation);
    assert!(matches!(
        rows.publish_activated_version(&scope, row.clone(), version.clone(), 1)
            .await,
        Err(WorkflowPublicationError::RevisionNotAdmitted)
    ));
    assert_eq!(rows.get(&scope, &row.id).await.unwrap().unwrap().version, 1);
    assert_eq!(versions.list(&scope, &row.id).await.unwrap().len(), 1);

    let pair = PlanFlavorRevisionRecord::graph_v1_json(activation.revisions().plan(),
        RevisionRecordBytes::try_from_vec(serde_json::to_vec(&serde_json::json!({"workflow_version_id":workflow_revision,"manifest":{"workflow_id":workflow_id}})).unwrap()).unwrap(),
        WorkerFlavorRevisionRecord::v1_json(activation.revisions().worker_flavor(), RevisionRecordBytes::try_from_vec(b"{}".to_vec()).unwrap()));
    writer.insert(&pair).await.unwrap();
    let mut wrong = version.clone();
    wrong.activation = Some(WorkflowActivation::new(
        WorkflowVersionId::new(),
        activation.revisions(),
    ));
    assert!(matches!(
        rows.publish_activated_version(&scope, row.clone(), wrong, 1)
            .await,
        Err(WorkflowPublicationError::InvalidPublication)
    ));
    assert_eq!(rows.get(&scope, &row.id).await.unwrap().unwrap().version, 1);
    assert_eq!(versions.list(&scope, &row.id).await.unwrap().len(), 1);

    let foreign_scope = Scope::new("foreign-workspace", "foreign-org");
    let mut foreign_row = row.clone();
    foreign_row.scope = foreign_scope.clone();
    assert!(matches!(
        rows.publish_activated_version(&foreign_scope, foreign_row, version.clone(), 1)
            .await,
        Err(WorkflowPublicationError::Storage(
            nebula_storage_port::StorageError::NotFound { .. }
        ))
    ));
    assert!(
        rows.save_with_published_version(&scope, row.clone(), version.clone(), Some(1))
            .await
            .is_err()
    );
    assert!(versions.create(&scope, version.clone()).await.is_err());
    assert_eq!(rows.get(&scope, &row.id).await.unwrap().unwrap().version, 1);
    assert_eq!(versions.list(&scope, &row.id).await.unwrap().len(), 1);

    let (first, second) = tokio::join!(
        rows.publish_activated_version(&scope, row.clone(), version.clone(), 1),
        rows.publish_activated_version(&scope, row.clone(), version.clone(), 1),
    );
    assert!(matches!(
        (&first, &second),
        (
            Ok(()),
            Err(WorkflowPublicationError::Storage(
                nebula_storage_port::StorageError::Conflict { .. }
            ))
        ) | (
            Err(WorkflowPublicationError::Storage(
                nebula_storage_port::StorageError::Conflict { .. }
            )),
            Ok(())
        )
    ));
    assert_eq!(
        versions.get(&scope, &row.id, 2).await.unwrap(),
        Some(version.clone())
    );
    assert_eq!(
        versions
            .get(&Scope::new("other", "other"), &row.id, 2)
            .await
            .unwrap(),
        None
    );
    assert!(matches!(
        rows.publish_activated_version(&scope, row.clone(), version.clone(), 1)
            .await,
        Err(WorkflowPublicationError::Storage(
            nebula_storage_port::StorageError::Conflict { .. }
        ))
    ));
    assert_eq!(versions.list(&scope, &row.id).await.unwrap().len(), 2);
    let mut reused_row = row.clone();
    reused_row.version = 3;
    let mut reused_version = version.clone();
    reused_version.number = 3;
    assert!(matches!(
        rows.publish_activated_version(&scope, reused_row, reused_version, 2)
            .await,
        Err(WorkflowPublicationError::InvalidPublication)
    ));
    admin
        .begin_drain(PlanFlavorRevisionTarget::ExecutablePlan(
            activation.revisions().plan(),
        ))
        .await
        .unwrap();
    row.version = 3;
    version.number = 3;
    assert!(matches!(
        rows.publish_activated_version(&scope, row.clone(), version, 2)
            .await,
        Err(WorkflowPublicationError::RevisionNotAdmitted)
    ));
    assert_eq!(rows.get(&scope, &row.id).await.unwrap().unwrap().version, 2);
    assert_eq!(versions.list(&scope, &row.id).await.unwrap().len(), 2);
    (scope, row.id, activation)
}

async fn draft_collision_contract(
    rows: &dyn WorkflowStore,
    versions: &dyn WorkflowVersionStore,
    writer: &dyn PlanFlavorCatalogWriter,
) {
    let scope = Scope::new("draft-workspace", "draft-org");
    let workflow_id = nebula_core::WorkflowId::new();
    let revision = WorkflowVersionId::new();
    let activation = WorkflowActivation::new(
        revision,
        PlanFlavorRevisionIds::new(
            ExecutablePlanRevisionId::from_bytes([9; 32]),
            WorkerFlavorRevisionId::from_bytes([10; 32]),
        ),
    );
    let original = WorkflowRecord {
        id: workflow_id.to_string(),
        scope: scope.clone(),
        version: 1,
        slug: workflow_id.to_string(),
        deleted: false,
    };
    rows.create(&scope, original.clone()).await.unwrap();
    let draft = WorkflowVersionRecord {
        activation: None,
        workflow_id: original.id.clone(),
        number: 2,
        published: false,
        pinned: false,
        definition: serde_json::json!({"preserved":"draft"}),
    };
    versions.create(&scope, draft.clone()).await.unwrap();
    let pair = PlanFlavorRevisionRecord::graph_v1_json(activation.revisions().plan(),
        RevisionRecordBytes::try_from_vec(serde_json::to_vec(&serde_json::json!({"workflow_version_id":revision,"manifest":{"workflow_id":workflow_id}})).unwrap()).unwrap(),
        WorkerFlavorRevisionRecord::v1_json(activation.revisions().worker_flavor(), RevisionRecordBytes::try_from_vec(b"{}".to_vec()).unwrap()));
    writer.insert(&pair).await.unwrap();
    let mut row = original.clone();
    row.version = 2;
    let mut activated = draft.clone();
    activated.activation = Some(activation);
    activated.published = true;
    assert!(matches!(
        rows.publish_activated_version(&scope, row, activated, 1)
            .await,
        Err(WorkflowPublicationError::Storage(
            nebula_storage_port::StorageError::Duplicate {
                entity: "workflow_version",
                ..
            }
        ))
    ));
    assert_eq!(
        rows.get(&scope, &original.id).await.unwrap(),
        Some(original.clone())
    );
    assert_eq!(
        versions.list(&scope, &original.id).await.unwrap(),
        vec![draft]
    );
}

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

#[cfg(feature = "postgres")]
#[path = "support/postgres_schema.rs"]
mod postgres_schema;

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_publication_is_admitted_and_atomic() {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            assert!(
                std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
                "required PostgreSQL publication evidence needs DATABASE_URL"
            );
            return;
        },
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("configured PostgreSQL URL must be Unicode")
        },
    };
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
