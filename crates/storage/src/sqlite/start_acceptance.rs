//! SQLite [`StartAcceptanceStore`] over the port-scoped schema.
//!
//! The reservation, execution, immutable contract bundle, Start command, and
//! live revision reference are committed in one transaction.

use std::time::Duration;

use nebula_storage_port::StorageError;
use nebula_storage_port::store::{
    StartAcceptanceStore, StartMaterialization, StartReservationMaintenance, StartRevisionRejection,
};
use sqlx::{Row, SqlitePool};

use crate::revision_catalog::ArtifactLifecycle;
use crate::sqlite::execution::{conn_err, insert_created_execution};

/// SQLite-backed owner of keyed start acceptance.
#[derive(Clone, Debug)]
pub struct SqliteStartAcceptanceStore {
    pool: SqlitePool,
}

impl SqliteStartAcceptanceStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl StartAcceptanceStore for SqliteStartAcceptanceStore {
    #[tracing::instrument(skip_all, err)]
    async fn lookup_trigger_start(
        &self,
        scope: &nebula_storage_port::Scope,
        key: &nebula_storage_port::dto::TriggerStartKey<'_>,
    ) -> Result<Option<String>, StorageError> {
        sqlx::query_scalar("SELECT execution_id FROM port_trigger_dedup_inbox WHERE workspace_id = ? AND org_id = ? AND trigger_id = ? AND event_id = ?").bind(&scope.workspace_id).bind(&scope.org_id).bind(key.trigger_id()).bind(key.event_id()).fetch_optional(&self.pool).await.map_err(conn_err)
    }

    #[tracing::instrument(skip_all, fields(execution_id = %start.execution_id()), err)]
    async fn materialize_start(
        &self,
        start: &nebula_storage_port::dto::MaterializedStart<'_>,
    ) -> Result<StartMaterialization, nebula_storage_port::store::StartMaterializationError> {
        use crate::start_materialization::sql_error;
        use nebula_storage_port::store::StartMaterializationError;
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(sql_error)?;

        if let Some(key) = start.trigger() {
            let inserted = sqlx::query("INSERT INTO port_trigger_dedup_inbox (workspace_id, org_id, trigger_id, event_id, execution_id, created_at) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT (workspace_id, org_id, trigger_id, event_id) DO NOTHING")
                .bind(&start.scope().workspace_id).bind(&start.scope().org_id).bind(key.trigger_id()).bind(key.event_id()).bind(start.execution_id()).bind(chrono::Utc::now().to_rfc3339())
                .execute(&mut *transaction).await.map_err(sql_error)?.rows_affected();
            if inserted == 0 {
                let execution_id = sqlx::query_scalar("SELECT execution_id FROM port_trigger_dedup_inbox WHERE workspace_id = ? AND org_id = ? AND trigger_id = ? AND event_id = ?")
                    .bind(&start.scope().workspace_id).bind(&start.scope().org_id).bind(key.trigger_id()).bind(key.event_id()).fetch_one(&mut *transaction).await.map_err(sql_error)?;
                return Ok(StartMaterialization::Replayed { execution_id });
            }
        }
        if let Some(key) = start.idempotency() {
            let inserted = sqlx::query("INSERT INTO port_start_key_reservations (workspace_id, org_id, start_key, fingerprint_version, fingerprint, execution_id, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)) ON CONFLICT (workspace_id, org_id, start_key) DO NOTHING")
                .bind(&start.scope().workspace_id).bind(&start.scope().org_id).bind(key.key())
                .bind(i64::from(key.fingerprint().version())).bind(key.fingerprint().digest().as_slice())
                .bind(start.execution_id())
                .execute(&mut *transaction).await.map_err(sql_error)?.rows_affected();
            if inserted == 0 {
                let stored = sqlx::query("SELECT fingerprint_version, fingerprint, execution_id FROM port_start_key_reservations WHERE workspace_id = ? AND org_id = ? AND start_key = ?")
                    .bind(&start.scope().workspace_id).bind(&start.scope().org_id).bind(key.key())
                    .fetch_one(&mut *transaction).await.map_err(sql_error)?;
                let version: i64 = stored.try_get("fingerprint_version").map_err(sql_error)?;
                let fingerprint: Vec<u8> = stored.try_get("fingerprint").map_err(sql_error)?;
                let execution_id = stored.try_get("execution_id").map_err(sql_error)?;
                return Ok(
                    if version == i64::from(key.fingerprint().version())
                        && fingerprint == key.fingerprint().digest().as_slice()
                    {
                        StartMaterialization::Replayed { execution_id }
                    } else {
                        StartMaterialization::FingerprintMismatch
                    },
                );
            }
        }
        let header = crate::start_materialization::validate_envelope(start)?;
        let commitment = crate::start_materialization::commitment(start)?;

        if let Some(stored) = sqlx::query("SELECT workspace_id, org_id, commitment_format, commitment FROM port_execution_contract_bundles WHERE execution_id = ?")
            .bind(start.execution_id()).fetch_optional(&mut *transaction).await.map_err(sql_error)? {
            let workspace: String = stored.try_get("workspace_id").map_err(sql_error)?;
            let org: String = stored.try_get("org_id").map_err(sql_error)?;
            let format: String = stored.try_get("commitment_format").map_err(sql_error)?;
            let original: Vec<u8> = stored.try_get("commitment").map_err(sql_error)?;
            return if workspace == start.scope().workspace_id && org == start.scope().org_id && format == "v1_sha256" && original == commitment.as_slice() {
                Ok(StartMaterialization::Replayed { execution_id: start.execution_id().to_owned() })
            } else { Err(StartMaterializationError::MaterializationConflict) };
        }
        let ids = start.bundle().identity().revisions();
        if let Some(rejection) =
            admit_exact_pair(&mut transaction, ids.plan(), ids.worker_flavor()).await?
        {
            return Ok(StartMaterialization::RevisionRejected(rejection));
        }
        let plan: Vec<u8> = sqlx::query_scalar(
            "SELECT record_bytes FROM port_executable_plan_revisions WHERE executable_plan_id = ?",
        )
        .bind(ids.plan().as_bytes().as_slice())
        .fetch_one(&mut *transaction)
        .await
        .map_err(sql_error)?;
        crate::start_materialization::validate_catalog_header(start, &header, &plan)?;
        insert_created_execution(
            &mut transaction,
            start.scope(),
            start.execution_id(),
            start.execution().workflow_id,
            start.execution().initial_state,
        )
        .await
        .map_err(|error| match error {
            StorageError::Duplicate { .. } => StartMaterializationError::MaterializationConflict,
            other => StartMaterializationError::Storage(other),
        })?;
        sqlx::query("INSERT INTO port_control_queue (id, execution_id, workspace_id, org_id, command, status, w3c_traceparent, reclaim_count, resume_target) VALUES (?, ?, ?, ?, 'Start', 'Pending', ?, 0, NULL)")
            .bind(start.command().id.as_slice()).bind(start.execution_id()).bind(&start.scope().workspace_id).bind(&start.scope().org_id)
            .bind(start.command().w3c_traceparent.as_deref()).execute(&mut *transaction).await.map_err(sql_error)?;
        sqlx::query("INSERT INTO port_execution_contract_bundles (execution_id, workspace_id, org_id, bundle_id, executable_plan_id, worker_flavor_id, record_format, record_bytes, commitment_format, commitment) VALUES (?, ?, ?, ?, ?, ?, 'v1_json', ?, 'v1_sha256', ?)")
            .bind(start.execution_id()).bind(&start.scope().workspace_id).bind(&start.scope().org_id)
            .bind(start.bundle().identity().bundle_id().as_bytes().as_slice()).bind(ids.plan().as_bytes().as_slice()).bind(ids.worker_flavor().as_bytes().as_slice())
            .bind(start.bundle().bytes()).bind(commitment.as_slice()).execute(&mut *transaction).await.map_err(sql_error)?;
        sqlx::query("INSERT INTO port_execution_revision_refs (execution_id, execution_contract_bundle_id, executable_plan_id, worker_flavor_id, reference_state) VALUES (?, ?, ?, ?, 'live')")
            .bind(start.execution_id()).bind(start.bundle().identity().bundle_id().as_bytes().as_slice())
            .bind(ids.plan().as_bytes().as_slice()).bind(ids.worker_flavor().as_bytes().as_slice())
            .execute(&mut *transaction).await.map_err(sql_error)?;
        transaction
            .commit()
            .await
            .map_err(|_| StartMaterializationError::OutcomeUnknown)?;
        Ok(StartMaterialization::Accepted {
            execution_id: start.execution_id().to_owned(),
        })
    }

    async fn lookup_start(
        &self,
        scope: &nebula_storage_port::Scope,
        key: &str,
    ) -> Result<Option<nebula_storage_port::dto::StartReservation>, StorageError> {
        let row = sqlx::query("SELECT fingerprint_version, fingerprint, execution_id FROM port_start_key_reservations WHERE workspace_id = ? AND org_id = ? AND start_key = ?")
            .bind(&scope.workspace_id).bind(&scope.org_id).bind(key).fetch_optional(&self.pool).await.map_err(conn_err)?;
        row.map(|row| {
            let version = row
                .try_get::<i64, _>("fingerprint_version")
                .map_err(conn_err)?
                .try_into()
                .map_err(|_| {
                    StorageError::Internal("invalid stored start fingerprint version".into())
                })?;
            let digest = row
                .try_get::<Vec<u8>, _>("fingerprint")
                .map_err(conn_err)?
                .try_into()
                .map_err(|_| {
                    StorageError::Internal("invalid stored start fingerprint size".into())
                })?;
            Ok(nebula_storage_port::dto::StartReservation::new(
                nebula_storage_port::store::StartFingerprint::new(version, digest),
                row.try_get("execution_id").map_err(conn_err)?,
            ))
        })
        .transpose()
    }

    async fn read_contract_bundle(
        &self,
        scope: &nebula_storage_port::Scope,
        execution_id: &str,
    ) -> Result<Option<nebula_storage_port::dto::StoredContractBundle>, StorageError> {
        let row = sqlx::query("SELECT bundle_id, executable_plan_id, worker_flavor_id, record_format, record_bytes FROM port_execution_contract_bundles WHERE execution_id = ? AND workspace_id = ? AND org_id = ?")
            .bind(execution_id).bind(&scope.workspace_id).bind(&scope.org_id).fetch_optional(&self.pool).await.map_err(conn_err)?;
        row.map(|row| {
            let invalid =
                || StorageError::Internal("invalid stored execution contract bundle".into());
            let format: String = row.try_get("record_format").map_err(conn_err)?;
            if format != "v1_json" {
                return Err(invalid());
            }
            let bundle: [u8; 16] = row
                .try_get::<Vec<u8>, _>("bundle_id")
                .map_err(conn_err)?
                .try_into()
                .map_err(|_| invalid())?;
            let plan: [u8; 32] = row
                .try_get::<Vec<u8>, _>("executable_plan_id")
                .map_err(conn_err)?
                .try_into()
                .map_err(|_| invalid())?;
            let flavor: [u8; 32] = row
                .try_get::<Vec<u8>, _>("worker_flavor_id")
                .map_err(conn_err)?
                .try_into()
                .map_err(|_| invalid())?;
            let bytes = row.try_get("record_bytes").map_err(conn_err)?;
            let identity = nebula_storage_port::store::StartContractIdentity::new(
                nebula_core::ExecutionContractBundleId::from_bytes(bundle),
                nebula_storage_port::PlanFlavorRevisionIds::new(
                    nebula_core::ExecutablePlanRevisionId::from_bytes(plan),
                    nebula_core::WorkerFlavorRevisionId::from_bytes(flavor),
                ),
            );
            let record = nebula_storage_port::dto::ContractBundleRecord::v1_json(identity, bytes)
                .map_err(|_| invalid())?;
            Ok(nebula_storage_port::dto::StoredContractBundle::new(
                scope.clone(),
                execution_id.to_owned(),
                record,
            ))
        })
        .transpose()
    }
}

#[async_trait::async_trait]
impl StartReservationMaintenance for SqliteStartAcceptanceStore {
    async fn evict_reservations_older_than(
        &self,
        retention: Duration,
    ) -> Result<u64, StorageError> {
        let retention_ms = i64::try_from(retention.as_millis()).unwrap_or(i64::MAX);
        let deleted = sqlx::query(
            "DELETE FROM port_start_key_reservations \
             WHERE created_at_ms < (CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER) - ?)",
        )
                .bind(retention_ms)
                .execute(&self.pool)
                .await
                .map_err(conn_err)?
                .rows_affected();
        Ok(deleted)
    }
}

/// Admit one exact plan/flavor pair inside the caller's transaction.
///
/// Returns `Ok(None)` when both revisions exist and are Active and the plan
/// is pinned to the requested flavor. Otherwise returns the typed rejection —
/// the caller rolls the transaction back, so a rejection never leaves a
/// reservation, execution, command, or reference row behind.
async fn admit_exact_pair(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    plan_id: nebula_core::ExecutablePlanRevisionId,
    worker_flavor_id: nebula_core::WorkerFlavorRevisionId,
) -> Result<Option<StartRevisionRejection>, StorageError> {
    let Some(plan_row) = sqlx::query(
        "SELECT worker_flavor_id, lifecycle FROM port_executable_plan_revisions \
         WHERE executable_plan_id = ?",
    )
    .bind(plan_id.as_bytes().as_slice())
    .fetch_optional(&mut **tx)
    .await
    .map_err(conn_err)?
    else {
        return Ok(Some(StartRevisionRejection::PlanUnavailable));
    };

    let pinned_flavor: Vec<u8> = plan_row.try_get("worker_flavor_id").map_err(conn_err)?;
    if pinned_flavor != worker_flavor_id.as_bytes().as_slice() {
        return Ok(Some(StartRevisionRejection::PairNotAdmitted));
    }
    let plan_lifecycle: String = plan_row.try_get("lifecycle").map_err(conn_err)?;
    match ArtifactLifecycle::from_text(&plan_lifecycle) {
        Some(ArtifactLifecycle::Active) => {},
        // Draining, deleted, or a lifecycle value this binary cannot decode:
        // fail closed either way.
        _ => return Ok(Some(StartRevisionRejection::PairNotAdmitted)),
    }

    let Some(flavor_row) = sqlx::query(
        "SELECT lifecycle FROM port_worker_flavor_revisions WHERE worker_flavor_id = ?",
    )
    .bind(worker_flavor_id.as_bytes().as_slice())
    .fetch_optional(&mut **tx)
    .await
    .map_err(conn_err)?
    else {
        return Ok(Some(StartRevisionRejection::WorkerFlavorUnavailable));
    };
    let flavor_lifecycle: String = flavor_row.try_get("lifecycle").map_err(conn_err)?;
    match ArtifactLifecycle::from_text(&flavor_lifecycle) {
        Some(ArtifactLifecycle::Active) => Ok(None),
        _ => Ok(Some(StartRevisionRejection::PairNotAdmitted)),
    }
}
