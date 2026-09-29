// Publication and draft-collision contracts shared by `workflow_activation`
// (in-memory + SQLite) and `workflow_activation_postgres`. Textually `include!`d
// at each binary's crate root.

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
