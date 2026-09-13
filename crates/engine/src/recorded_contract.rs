//! Structural closure of the execution owner's persisted contract envelope.

use nebula_core::{
    ExecutablePlanRevisionId, ExecutionContractBundleId, PluginSetId, WorkerFlavorRevisionId,
    WorkflowVersionId,
};
use nebula_execution::{
    ExecutionBindingManifestV2, ExecutionContractBundle, ExecutionContractBundleV2,
    RecordedExecutionContractBundleV1, RecordedExecutionContractBundleV2,
};
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
    IntegrityV1(#[source] nebula_execution::ExecutionContractBundleIntegrityError),
    #[error("recorded version-two contract integrity failed")]
    IntegrityV2(#[source] nebula_execution::ExecutionContractBundleIntegrityErrorV2),
}

/// Structurally checked execution contract loaded from durable storage.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CheckedExecutionContract {
    /// Historical contract without a site-qualified binding manifest.
    V1(ExecutionContractBundle),
    /// Contract with an exact site-qualified binding manifest.
    V2(ExecutionContractBundleV2),
}

impl CheckedExecutionContract {
    /// Durable contract identity.
    #[must_use]
    pub const fn bundle_id(&self) -> ExecutionContractBundleId {
        match self {
            Self::V1(bundle) => bundle.bundle_id(),
            Self::V2(bundle) => bundle.bundle_id(),
        }
    }

    /// Exact executable-plan revision.
    #[must_use]
    pub const fn executable_plan_revision_id(&self) -> ExecutablePlanRevisionId {
        match self {
            Self::V1(bundle) => bundle.executable_plan_revision_id(),
            Self::V2(bundle) => bundle.executable_plan_revision_id(),
        }
    }

    /// Exact worker flavor revision.
    #[must_use]
    pub const fn worker_flavor_revision_id(&self) -> WorkerFlavorRevisionId {
        match self {
            Self::V1(bundle) => bundle.revisions().worker_flavor(),
            Self::V2(bundle) => bundle.revisions().worker_flavor(),
        }
    }

    /// Exact workflow version.
    #[must_use]
    pub const fn workflow_version_id(&self) -> WorkflowVersionId {
        match self {
            Self::V1(bundle) => bundle.revisions().workflow(),
            Self::V2(bundle) => bundle.revisions().workflow(),
        }
    }

    /// Exact plugin set used to compile the plan.
    #[must_use]
    pub const fn plugin_set_id(&self) -> PluginSetId {
        match self {
            Self::V1(bundle) => bundle.plugin_set_id(),
            Self::V2(bundle) => bundle.plugin_set_id(),
        }
    }

    /// Exact binding closure, when this is a V2 contract.
    #[must_use]
    pub const fn binding_manifest(&self) -> Option<&ExecutionBindingManifestV2> {
        match self {
            Self::V1(_) => None,
            Self::V2(bundle) => Some(bundle.binding_manifest()),
        }
    }
}

pub(crate) fn checked_bundle(
    scope: &Scope,
    execution_id: &str,
    stored: &StoredContractBundle,
) -> Result<CheckedExecutionContract, RecordedContractRejection> {
    if stored.scope() != scope || stored.execution_id() != execution_id {
        return Err(RecordedContractRejection::Identity);
    }
    let bundle = match stored.record().format() {
        ContractBundleFormat::V1Json => {
            let recorded: RecordedExecutionContractBundleV1 =
                serde_json::from_slice(stored.record().bytes())
                    .map_err(|_| RecordedContractRejection::Malformed)?;
            CheckedExecutionContract::V1(
                ExecutionContractBundle::try_from_recorded_v1(recorded)
                    .map_err(RecordedContractRejection::IntegrityV1)?,
            )
        },
        ContractBundleFormat::V2Json => {
            let recorded: RecordedExecutionContractBundleV2 =
                serde_json::from_slice(stored.record().bytes())
                    .map_err(|_| RecordedContractRejection::Malformed)?;
            CheckedExecutionContract::V2(
                ExecutionContractBundleV2::try_from_recorded_v2(recorded)
                    .map_err(RecordedContractRejection::IntegrityV2)?,
            )
        },
        _ => return Err(RecordedContractRejection::Malformed),
    };
    let org_id: nebula_core::OrgId = scope
        .org_id
        .parse()
        .map_err(|_| RecordedContractRejection::Identity)?;
    let workspace_id: nebula_core::WorkspaceId = scope
        .workspace_id
        .parse()
        .map_err(|_| RecordedContractRejection::Identity)?;
    let identity = stored.record().identity();
    let tenant_matches = match &bundle {
        CheckedExecutionContract::V1(bundle) => {
            bundle.org_id() == org_id && bundle.workspace_id() == workspace_id
        },
        CheckedExecutionContract::V2(bundle) => {
            bundle.org_id() == org_id && bundle.workspace_id() == workspace_id
        },
    };
    if !tenant_matches
        || bundle.bundle_id() != identity.bundle_id()
        || bundle.executable_plan_revision_id() != identity.revisions().plan()
        || bundle.worker_flavor_revision_id() != identity.revisions().worker_flavor()
    {
        return Err(RecordedContractRejection::Identity);
    }
    Ok(bundle)
}
