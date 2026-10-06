//! Seed a workflow definition straight into the in-memory workflow stores.
//!
//! A version belongs to its workflow row (`fk_workflow_versions__workflows`),
//! so seeding a version first ensures the row; later versions of the same
//! workflow reuse it.

use nebula_storage::{InMemoryExecutionStore, InMemoryWorkflowStore, InMemoryWorkflowVersionStore};
use nebula_storage_port::dto::{WorkflowRecord, WorkflowVersionRecord};
use nebula_storage_port::store::{WorkflowStore, WorkflowVersionStore};
use nebula_storage_port::{Scope, StorageError};

/// Ensure the workflow row of `record`, then create the version.
pub(crate) async fn save_version(
    versions: &InMemoryWorkflowVersionStore,
    scope: &Scope,
    record: WorkflowVersionRecord,
) -> Result<(), StorageError> {
    // Row creation never consults the revision catalog the execution store
    // supplies.
    let rows = InMemoryWorkflowStore::new_with_versions(versions, &InMemoryExecutionStore::new());
    let row = WorkflowRecord {
        id: record.workflow_id.clone(),
        scope: scope.clone(),
        version: 0,
        slug: record.workflow_id.clone(),
    };
    match rows.create(scope, row).await {
        Ok(()) | Err(StorageError::Duplicate { .. }) => {},
        Err(error) => return Err(error),
    }
    versions.create(scope, record).await
}
