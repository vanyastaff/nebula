//! Execution-owner envelope checks and original-attempt recovery commitments.

use nebula_core::{
    ExecutablePlanRevisionId, ExecutionContractBundleId, ExecutionId, OrgId, PluginSetId,
    WorkerFlavorRevisionId, WorkflowId, WorkflowVersionId, WorkspaceId,
};
use nebula_storage_port::dto::{ControlCommand, MaterializedStart, StoredContractBundle};
use nebula_storage_port::store::StartMaterializationError;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub(crate) struct StoredStart {
    pub(crate) bundle: StoredContractBundle,
    pub(crate) commitment: [u8; 32],
}

#[derive(Deserialize)]
pub(crate) struct BundleHeader {
    bundle_id: ExecutionContractBundleId,
    org_id: OrgId,
    workspace_id: WorkspaceId,
    executable_plan_revision_id: ExecutablePlanRevisionId,
    plugin_set_id: PluginSetId,
    revisions: Revisions,
}
#[derive(Deserialize)]
struct Revisions {
    workflow: WorkflowVersionId,
    worker_flavor: WorkerFlavorRevisionId,
}

pub(crate) fn validate_envelope(
    start: &MaterializedStart<'_>,
) -> Result<BundleHeader, StartMaterializationError> {
    #[derive(Deserialize)]
    struct InitialState {
        execution_id: ExecutionId,
        workflow_id: WorkflowId,
        workflow_version_number: u32,
        executable_plan_revision_id: ExecutablePlanRevisionId,
        worker_flavor_revision_id: WorkerFlavorRevisionId,
        status: String,
        version: u64,
        node_states: serde_json::Map<String, serde_json::Value>,
        created_at: chrono::DateTime<chrono::Utc>,
        updated_at: chrono::DateTime<chrono::Utc>,
        #[serde(default)]
        started_at: Option<serde_json::Value>,
        #[serde(default)]
        completed_at: Option<serde_json::Value>,
        total_output_bytes: u64,
        total_retries: u32,
        #[serde(default)]
        terminated_by: Option<serde_json::Value>,
        #[serde(default)]
        lease_holder: Option<serde_json::Value>,
        #[serde(default)]
        lease_expires_at: Option<serde_json::Value>,
    }
    let invalid = || StartMaterializationError::InvalidEnvelope;
    let execution_id: ExecutionId = start.execution_id().parse().map_err(|_| invalid())?;
    let org_id: OrgId = start.scope().org_id.parse().map_err(|_| invalid())?;
    let workspace_id: WorkspaceId = start.scope().workspace_id.parse().map_err(|_| invalid())?;
    let workflow_id: WorkflowId = start
        .execution()
        .workflow_id
        .parse()
        .map_err(|_| invalid())?;
    let state: InitialState =
        serde_json::from_value(start.execution().initial_state.clone()).map_err(|_| invalid())?;
    let bundle: BundleHeader =
        serde_json::from_slice(start.bundle().bytes()).map_err(|_| invalid())?;
    let identity = start.bundle().identity();
    let command = start.command();
    if state.execution_id != execution_id
        || state.workflow_id != workflow_id
        || state.workflow_version_number == 0
        || state.status != "created"
        || state.version != 0
        || !state.node_states.is_empty()
        || state.created_at != state.updated_at
        || state.started_at.is_some()
        || state.completed_at.is_some()
        || state.total_output_bytes != 0
        || state.total_retries != 0
        || state.terminated_by.is_some()
        || state.lease_holder.is_some()
        || state.lease_expires_at.is_some()
        || state.executable_plan_revision_id != identity.revisions().plan()
        || state.worker_flavor_revision_id != identity.revisions().worker_flavor()
        || bundle.bundle_id != identity.bundle_id()
        || bundle.executable_plan_revision_id != identity.revisions().plan()
        || bundle.revisions.worker_flavor != identity.revisions().worker_flavor()
        || bundle.org_id != org_id
        || bundle.workspace_id != workspace_id
        || command.command != ControlCommand::Start
        || command.execution_id != start.execution_id()
        || command.scope != *start.scope()
        || command.reclaim_count != 0
        || command.resume_target.is_some()
        || start.idempotency().is_some_and(|key| key.key().is_empty())
        || start
            .trigger()
            .is_some_and(|key| key.trigger_id().is_empty() || key.event_id().is_empty())
    {
        return Err(invalid());
    }
    Ok(bundle)
}

pub(crate) fn validate_catalog_header(
    start: &MaterializedStart<'_>,
    bundle: &BundleHeader,
    bytes: &[u8],
) -> Result<(), StartMaterializationError> {
    #[derive(Deserialize)]
    struct Plan {
        claimed_id: ExecutablePlanRevisionId,
        workflow_version_id: WorkflowVersionId,
        worker_flavor_revision_id: WorkerFlavorRevisionId,
        plugin_set_id: PluginSetId,
        manifest: Manifest,
    }
    #[derive(Deserialize)]
    struct Manifest {
        workflow_id: WorkflowId,
    }
    let plan: Plan =
        serde_json::from_slice(bytes).map_err(|_| StartMaterializationError::InvalidEnvelope)?;
    let workflow_id: WorkflowId = start
        .execution()
        .workflow_id
        .parse()
        .map_err(|_| StartMaterializationError::InvalidEnvelope)?;
    if plan.claimed_id != bundle.executable_plan_revision_id
        || plan.workflow_version_id != bundle.revisions.workflow
        || plan.worker_flavor_revision_id != bundle.revisions.worker_flavor
        || plan.plugin_set_id != bundle.plugin_set_id
        || plan.manifest.workflow_id != workflow_id
    {
        return Err(StartMaterializationError::InvalidEnvelope);
    }
    Ok(())
}

/// V1 commits to every original field; recursive key sorting is independent of
/// serde_json's preserve_order feature. Opaque bundle bytes retain exact spelling.
pub(crate) fn commitment(
    start: &MaterializedStart<'_>,
) -> Result<[u8; 32], StartMaterializationError> {
    let key = start
        .trigger()
        .map(|key| serde_json::json!(["trigger", key.trigger_id(), key.event_id()]))
        .or_else(|| {
            start.idempotency().map(|key| {
                serde_json::json!([
                    key.key(),
                    key.fingerprint().version(),
                    key.fingerprint().digest()
                ])
            })
        });
    let mut header = serde_json::json!([
        start.scope(),
        start.execution_id(),
        start.execution().workflow_id,
        start.execution().initial_state,
        start.command(),
        key,
        "v1_json",
        start.bundle().identity().bundle_id(),
        start.bundle().identity().revisions().plan(),
        start.bundle().identity().revisions().worker_flavor(),
    ]);
    sort_keys(&mut header);
    let bytes =
        serde_json::to_vec(&header).map_err(|_| StartMaterializationError::InvalidEnvelope)?;
    let mut digest = Sha256::new();
    if start.trigger().is_some() {
        digest.update(b"nebula.trigger-start-materialization.v1\0");
    } else {
        digest.update(b"nebula.start-materialization.v1\0");
    }
    digest.update(
        u64::try_from(bytes.len())
            .map_err(|_| StartMaterializationError::InvalidEnvelope)?
            .to_be_bytes(),
    );
    digest.update(bytes);
    digest.update(
        u64::try_from(start.bundle().bytes().len())
            .map_err(|_| StartMaterializationError::InvalidEnvelope)?
            .to_be_bytes(),
    );
    digest.update(start.bundle().bytes());
    Ok(digest.finalize().into())
}

fn sort_keys(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            fields.sort_keys();
            for value in fields.values_mut() {
                sort_keys(value);
            }
        },
        serde_json::Value::Array(values) => {
            for value in values {
                sort_keys(value);
            }
        },
        _ => {},
    }
}

#[cfg(feature = "postgres")]
pub(crate) fn execution_lock_id(execution_id: &str) -> i64 {
    let mut digest = Sha256::new();
    digest.update(b"nebula.start-execution-lock.v1\0");
    digest.update(execution_id.as_bytes());
    let digest = digest.finalize();
    let mut prefix = [0; 8];
    // SHA256 always returns exactly 32 bytes.
    prefix.copy_from_slice(&digest[..8]);
    i64::from_be_bytes(prefix)
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn sql_error(error: sqlx::Error) -> StartMaterializationError {
    match error {
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            StartMaterializationError::MaterializationConflict
        },
        sqlx::Error::RowNotFound => {
            StartMaterializationError::Storage(nebula_storage_port::StorageError::not_found(
                "start materialization dependency",
                "required row",
            ))
        },
        sqlx::Error::Database(_)
        | sqlx::Error::Decode(_)
        | sqlx::Error::ColumnDecode { .. }
        | sqlx::Error::ColumnNotFound(_)
        | sqlx::Error::TypeNotFound { .. } => {
            StartMaterializationError::Storage(nebula_storage_port::StorageError::Serialization(
                "start materialization statement or stored row is invalid".into(),
            ))
        },
        _ => StartMaterializationError::Storage(nebula_storage_port::StorageError::Connection(
            "start materialization backend operation failed".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_storage_port::dto::{
        ContractBundleRecord, ControlMsg, MaterializedStart, NewExecution, StartKey,
        TriggerStartKey,
    };
    use nebula_storage_port::store::{StartContractIdentity, StartFingerprint};
    use nebula_storage_port::{PlanFlavorRevisionIds, Scope};

    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    #[test]
    fn sql_error_distinguishes_missing_rows_from_backend_unavailability() {
        assert!(matches!(
            sql_error(sqlx::Error::RowNotFound),
            StartMaterializationError::Storage(nebula_storage_port::StorageError::NotFound { .. })
        ));
        assert!(matches!(
            sql_error(sqlx::Error::PoolClosed),
            StartMaterializationError::Storage(nebula_storage_port::StorageError::Connection(_))
        ));
    }

    #[test]
    fn caller_commitment_v1_stays_stable_and_trigger_origin_is_distinct() {
        let scope = Scope::new("workspace", "org");
        let state = serde_json::json!({"input":{"z":2,"a":1}});
        let command = ControlMsg {
            id: [3; 16],
            execution_id: "execution".into(),
            command: ControlCommand::Start,
            scope: scope.clone(),
            w3c_traceparent: None,
            reclaim_count: 0,
            resume_target: None,
        };
        let identity = StartContractIdentity::new(
            ExecutionContractBundleId::from_bytes([4; 16]),
            PlanFlavorRevisionIds::new(
                ExecutablePlanRevisionId::from_bytes([5; 32]),
                WorkerFlavorRevisionId::from_bytes([6; 32]),
            ),
        );
        let bundle = ContractBundleRecord::v1_json(identity, b"{}".to_vec()).unwrap();
        let caller = MaterializedStart::new(
            &scope,
            Some(StartKey::new("event", StartFingerprint::new(2, [7; 32]))),
            "execution",
            NewExecution::new("workflow", &state),
            &command,
            &bundle,
        );
        let trigger = MaterializedStart::for_trigger(
            &scope,
            TriggerStartKey::new("trigger", "event"),
            "execution",
            NewExecution::new("workflow", &state),
            &command,
            &bundle,
        );
        assert_eq!(
            commitment(&caller).unwrap(),
            [
                88, 14, 131, 241, 91, 114, 132, 164, 50, 1, 156, 142, 68, 183, 183, 100, 217, 54,
                131, 178, 116, 220, 160, 207, 25, 35, 73, 120, 24, 57, 196, 15
            ]
        );
        assert_ne!(commitment(&caller).unwrap(), commitment(&trigger).unwrap());
    }
}
