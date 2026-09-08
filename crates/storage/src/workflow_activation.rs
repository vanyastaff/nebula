//! Identity checks shared by workflow-owner publication transactions.

use nebula_core::{WorkflowId, WorkflowVersionId};
use nebula_storage_port::Scope;
use nebula_storage_port::dto::{WorkflowActivation, WorkflowRecord, WorkflowVersionRecord};
use nebula_storage_port::store::WorkflowPublicationError;
use serde::Deserialize;

pub(crate) fn validate_publication(
    scope: &Scope,
    row: &WorkflowRecord,
    version: &WorkflowVersionRecord,
    expected_version: u64,
) -> Result<WorkflowActivation, WorkflowPublicationError> {
    let activation = version
        .activation
        .ok_or(WorkflowPublicationError::InvalidPublication)?;
    let next = expected_version
        .checked_add(1)
        .ok_or(WorkflowPublicationError::InvalidPublication)?;
    if row.scope != *scope
        || row.id != version.workflow_id
        || row.deleted
        || !version.published
        || row.version != next
        || u64::from(version.number) != next
    {
        return Err(WorkflowPublicationError::InvalidPublication);
    }
    Ok(activation)
}

/// Decode only the relational header. Full hash/contract checks belong to the installer.
pub(crate) fn validate_plan_identity(
    bytes: &[u8],
    workflow_id: &str,
    activation: WorkflowActivation,
) -> Result<(), WorkflowPublicationError> {
    #[derive(Deserialize)]
    struct Header {
        workflow_version_id: WorkflowVersionId,
        manifest: Manifest,
    }
    #[derive(Deserialize)]
    struct Manifest {
        workflow_id: WorkflowId,
    }
    let header: Header =
        serde_json::from_slice(bytes).map_err(|_| WorkflowPublicationError::InvalidPublication)?;
    let workflow_id: WorkflowId = workflow_id
        .parse()
        .map_err(|_| WorkflowPublicationError::InvalidPublication)?;
    if header.workflow_version_id != activation.workflow_version_id()
        || header.manifest.workflow_id != workflow_id
    {
        return Err(WorkflowPublicationError::InvalidPublication);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::{ExecutablePlanRevisionId, WorkerFlavorRevisionId};
    use nebula_storage_port::PlanFlavorRevisionIds;

    #[test]
    fn duplicate_identity_headers_and_wrong_workflow_fail_without_payload_diagnostics() {
        let workflow = WorkflowId::new();
        let revision = WorkflowVersionId::new();
        let activation = WorkflowActivation::new(
            revision,
            PlanFlavorRevisionIds::new(
                ExecutablePlanRevisionId::from_bytes([3; 32]),
                WorkerFlavorRevisionId::from_bytes([4; 32]),
            ),
        );
        let duplicate = format!(
            "{{\"workflow_version_id\":\"{revision}\",\"workflow_version_id\":\"{revision}\",\"manifest\":{{\"workflow_id\":\"{workflow}\"}},\"private\":\"secret-canary\"}}"
        );
        let error = validate_plan_identity(duplicate.as_bytes(), &workflow.to_string(), activation)
            .unwrap_err();
        assert!(!format!("{error:?}: {error}").contains("secret-canary"));
        let other = serde_json::to_vec(&serde_json::json!({"workflow_version_id":revision,"manifest":{"workflow_id":WorkflowId::new()}})).unwrap();
        assert!(matches!(
            validate_plan_identity(&other, &workflow.to_string(), activation),
            Err(WorkflowPublicationError::InvalidPublication)
        ));
    }
}
