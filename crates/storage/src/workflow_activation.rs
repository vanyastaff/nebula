//! Identity checks shared by workflow-owner publication transactions.

#[cfg(any(test, feature = "sqlite", feature = "postgres"))]
use nebula_core::{ExecutablePlanRevisionId, WorkerFlavorRevisionId};
use nebula_core::{WorkflowId, WorkflowVersionId};
#[cfg(any(test, feature = "sqlite", feature = "postgres"))]
use nebula_storage_port::PlanFlavorRevisionIds;
use nebula_storage_port::Scope;
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use nebula_storage_port::StorageError;
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
        || !version.published
        || row.version != next
        || u64::from(version.number) != next
    {
        return Err(WorkflowPublicationError::InvalidPublication);
    }
    Ok(activation)
}

/// An activation identity as its three `workflow_versions` columns.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) struct ActivationColumns {
    pub(crate) workflow_version: Option<String>,
    pub(crate) executable_plan: Option<Vec<u8>>,
    pub(crate) worker_flavor: Option<Vec<u8>>,
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
impl ActivationColumns {
    pub(crate) fn encode(activation: Option<WorkflowActivation>) -> Self {
        let Some(activation) = activation else {
            return Self {
                workflow_version: None,
                executable_plan: None,
                worker_flavor: None,
            };
        };
        let revisions = activation.revisions();
        Self {
            workflow_version: Some(activation.workflow_version_id().to_string()),
            executable_plan: Some(revisions.plan().as_bytes().to_vec()),
            worker_flavor: Some(revisions.worker_flavor().as_bytes().to_vec()),
        }
    }

    /// The activation, or `None` for a version saved without one. A partial
    /// or malformed identity is corrupt.
    pub(crate) fn decode(self) -> Result<Option<WorkflowActivation>, StorageError> {
        let (workflow_version_id, plan, flavor) = match (
            self.workflow_version,
            self.executable_plan,
            self.worker_flavor,
        ) {
            (None, None, None) => return Ok(None),
            (Some(version), Some(plan), Some(flavor)) => (version, plan, flavor),
            _ => return Err(corrupt_activation()),
        };
        let workflow_version_id: WorkflowVersionId = workflow_version_id
            .parse()
            .map_err(|_| corrupt_activation())?;
        let plan = <[u8; 32]>::try_from(plan.as_slice()).map_err(|_| corrupt_activation())?;
        let flavor = <[u8; 32]>::try_from(flavor.as_slice()).map_err(|_| corrupt_activation())?;
        Ok(Some(WorkflowActivation::new(
            workflow_version_id,
            PlanFlavorRevisionIds::new(
                ExecutablePlanRevisionId::from_bytes(plan),
                WorkerFlavorRevisionId::from_bytes(flavor),
            ),
        )))
    }
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn corrupt_activation() -> StorageError {
    StorageError::Corrupt("workflow version activation columns do not decode".into())
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
