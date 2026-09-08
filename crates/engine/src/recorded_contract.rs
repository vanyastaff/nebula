//! Structural closure of the execution owner's persisted contract envelope.

use nebula_execution::{ExecutionContractBundle, RecordedExecutionContractBundleV1};
use nebula_storage_port::{
    Scope,
    dto::{ContractBundleFormat, StoredContractBundle},
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum RecordedContractRejection {
    #[error("recorded contract identity does not match its execution")]
    Identity,
    #[error("recorded contract is malformed")]
    Malformed,
    #[error("recorded contract integrity failed")]
    Integrity(#[source] nebula_execution::ExecutionContractBundleIntegrityError),
}

pub(crate) fn checked_bundle(
    scope: &Scope,
    execution_id: &str,
    stored: &StoredContractBundle,
) -> Result<ExecutionContractBundle, RecordedContractRejection> {
    if stored.scope() != scope || stored.execution_id() != execution_id {
        return Err(RecordedContractRejection::Identity);
    }
    if stored.record().format() != ContractBundleFormat::V1Json {
        return Err(RecordedContractRejection::Malformed);
    }
    let recorded: RecordedExecutionContractBundleV1 =
        serde_json::from_slice(stored.record().bytes())
            .map_err(|_| RecordedContractRejection::Malformed)?;
    let bundle = ExecutionContractBundle::try_from_recorded_v1(recorded)
        .map_err(RecordedContractRejection::Integrity)?;
    let org_id: nebula_core::OrgId = scope
        .org_id
        .parse()
        .map_err(|_| RecordedContractRejection::Identity)?;
    let workspace_id: nebula_core::WorkspaceId = scope
        .workspace_id
        .parse()
        .map_err(|_| RecordedContractRejection::Identity)?;
    let identity = stored.record().identity();
    if bundle.org_id() != org_id
        || bundle.workspace_id() != workspace_id
        || bundle.bundle_id() != identity.bundle_id()
        || bundle.executable_plan_revision_id() != identity.revisions().plan()
        || bundle.revisions().worker_flavor() != identity.revisions().worker_flavor()
    {
        return Err(RecordedContractRejection::Identity);
    }
    Ok(bundle)
}
