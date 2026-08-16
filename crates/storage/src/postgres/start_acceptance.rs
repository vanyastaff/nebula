//! Postgres [`StartAcceptanceStore`] over the port-scoped schema.
//!
//! The reservation, the execution aggregate row, and the Start control row are
//! three statements in **one** transaction, so a start key can never be
//! reserved for an execution that does not exist, and an execution can never
//! exist without the Start command that drives it. The materialized path
//! composes the revision admission and the live reference row into the same
//! transaction.

use std::time::Duration;

use nebula_core::id::ExecutionId;
use nebula_storage_port::StorageError;
use nebula_storage_port::store::{
    KeyedStart, MaterializedKeyedStart, StartAcceptance, StartAcceptanceStore,
    StartMaterialization, StartRevisionRejection,
};
use sqlx::{PgPool, Row};

use super::execution::{conn_err, insert_created_execution};
use crate::revision_catalog::ArtifactLifecycle;

/// Postgres-backed owner of keyed start acceptance.
#[derive(Clone, Debug)]
pub struct PgStartAcceptanceStore {
    pool: PgPool,
}

impl PgStartAcceptanceStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl StartAcceptanceStore for PgStartAcceptanceStore {
    #[tracing::instrument(
        level = "debug",
        skip(self, start),
        fields(
            execution_id = start.execution_id,
            fingerprint_version = start.fingerprint.version(),
        )
    )]
    async fn accept_keyed_start(
        &self,
        start: &KeyedStart<'_>,
    ) -> Result<StartAcceptance, StorageError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let mut tx = self.pool.begin().await.map_err(conn_err)?;

        // First writer wins by PRIMARY KEY(workspace_id, org_id, start_key).
        // The scope columns are inside the key, so one tenant can neither
        // collide with nor probe another's reservations.
        let reserved = sqlx::query(
            "INSERT INTO port_start_key_reservations \
             (workspace_id, org_id, start_key, fingerprint_version, fingerprint, \
              execution_id, created_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (workspace_id, org_id, start_key) DO NOTHING",
        )
        .bind(&start.scope.workspace_id)
        .bind(&start.scope.org_id)
        .bind(start.start_key)
        .bind(i32::from(start.fingerprint.version()))
        .bind(start.fingerprint.digest().as_slice())
        .bind(start.execution_id)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?
        .rows_affected()
            == 1;

        if !reserved {
            // Read the incumbent back inside the same transaction: a separate
            // connection could observe a reservation this transaction has not
            // committed against, or miss one it has.
            //
            // `FOR SHARE` holds the row against a concurrent retention sweep
            // for the rest of this transaction, so the receipt cannot be
            // decided against a row that is being deleted underneath it.
            let row = sqlx::query(
                "SELECT fingerprint_version, fingerprint, execution_id \
                 FROM port_start_key_reservations \
                 WHERE workspace_id = $1 AND org_id = $2 AND start_key = $3 \
                 FOR SHARE",
            )
            .bind(&start.scope.workspace_id)
            .bind(&start.scope.org_id)
            .bind(start.start_key)
            .fetch_one(&mut *tx)
            .await
            .map_err(conn_err)?;

            let stored_version: i32 = row.try_get("fingerprint_version").map_err(conn_err)?;
            let stored_digest: Vec<u8> = row.try_get("fingerprint").map_err(conn_err)?;
            let execution_id: String = row.try_get("execution_id").map_err(conn_err)?;
            // Nothing was written on this path; commit only releases the read
            // transaction.
            tx.commit().await.map_err(conn_err)?;

            return Ok(crate::start_acceptance::replay_outcome(
                start,
                i64::from(stored_version),
                &stored_digest,
                execution_id,
            ));
        }

        insert_created_execution(
            &mut tx,
            start.scope,
            start.execution_id,
            start.execution.workflow_id,
            start.execution.initial_state,
        )
        .await?;

        let resume_target_json = start
            .command
            .resume_target
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        sqlx::query(
            "INSERT INTO port_control_queue \
             (id, execution_id, workspace_id, org_id, command, status, \
              w3c_traceparent, reclaim_count, resume_target) \
             VALUES ($1, $2, $3, $4, $5, 'Pending', $6, $7, $8)",
        )
        .bind(start.command.id.as_slice())
        .bind(&start.command.execution_id)
        .bind(&start.command.scope.workspace_id)
        .bind(&start.command.scope.org_id)
        .bind(start.command.command.as_str())
        .bind(start.command.w3c_traceparent.as_deref())
        .bind(i32::try_from(start.command.reclaim_count).unwrap_or(i32::MAX))
        .bind(resume_target_json)
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;

        tx.commit().await.map_err(conn_err)?;
        tracing::debug!(
            target: "nebula_storage::postgres",
            "start_acceptance: reserved key, created execution, enqueued Start"
        );
        Ok(StartAcceptance::Accepted {
            execution_id: start.execution_id.to_owned(),
        })
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, start),
        fields(
            execution_id = start.keyed.execution_id,
            fingerprint_version = start.keyed.fingerprint.version(),
        )
    )]
    async fn materialize_keyed_start(
        &self,
        start: &MaterializedKeyedStart<'_>,
    ) -> Result<StartMaterialization, StorageError> {
        let keyed = &start.keyed;
        // The reference row's CHECK admits only typed `exe_` ids; validate up
        // front so a malformed id fails closed before the transaction starts.
        keyed.execution_id.parse::<ExecutionId>().map_err(|_| {
            StorageError::Internal(
                "materialize_keyed_start: execution id is not a typed ExecutionId".to_owned(),
            )
        })?;

        let now_ms = chrono::Utc::now().timestamp_millis();
        let mut tx = self.pool.begin().await.map_err(conn_err)?;

        // First writer wins by PRIMARY KEY(workspace_id, org_id, start_key).
        let reserved = sqlx::query(
            "INSERT INTO port_start_key_reservations \
             (workspace_id, org_id, start_key, fingerprint_version, fingerprint, \
              execution_id, created_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (workspace_id, org_id, start_key) DO NOTHING",
        )
        .bind(&keyed.scope.workspace_id)
        .bind(&keyed.scope.org_id)
        .bind(keyed.start_key)
        .bind(i32::from(keyed.fingerprint.version()))
        .bind(keyed.fingerprint.digest().as_slice())
        .bind(keyed.execution_id)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?
        .rows_affected()
            == 1;

        if !reserved {
            // Read the incumbent back inside the same transaction. `FOR SHARE`
            // holds the row against a concurrent retention sweep for the rest
            // of this transaction.
            let row = sqlx::query(
                "SELECT fingerprint_version, fingerprint, execution_id \
                 FROM port_start_key_reservations \
                 WHERE workspace_id = $1 AND org_id = $2 AND start_key = $3 \
                 FOR SHARE",
            )
            .bind(&keyed.scope.workspace_id)
            .bind(&keyed.scope.org_id)
            .bind(keyed.start_key)
            .fetch_one(&mut *tx)
            .await
            .map_err(conn_err)?;

            let stored_version: i32 = row.try_get("fingerprint_version").map_err(conn_err)?;
            let stored_digest: Vec<u8> = row.try_get("fingerprint").map_err(conn_err)?;
            let execution_id: String = row.try_get("execution_id").map_err(conn_err)?;
            tx.commit().await.map_err(conn_err)?;

            return Ok(
                match crate::start_acceptance::replay_outcome(
                    keyed,
                    i64::from(stored_version),
                    &stored_digest,
                    execution_id,
                ) {
                    StartAcceptance::Replayed { execution_id } => {
                        StartMaterialization::Replayed { execution_id }
                    },
                    StartAcceptance::FingerprintMismatch => {
                        StartMaterialization::FingerprintMismatch
                    },
                    StartAcceptance::Accepted { .. } => {
                        unreachable!("a stored reservation can never replay as a fresh acceptance")
                    },
                },
            );
        }

        // Admit the exact pair under the commit lock. A revision that began
        // draining before this transaction committed loses the race: the
        // tentative reservation insert rolls back with everything else.
        let revisions = start.identity.revisions();
        if let Some(rejection) =
            admit_exact_pair(&mut tx, revisions.plan(), revisions.worker_flavor()).await?
        {
            tx.rollback().await.map_err(conn_err)?;
            tracing::debug!(
                target: "nebula_storage::postgres",
                rejection = ?rejection,
                "start_acceptance: exact revisions not admitted; nothing written"
            );
            return Ok(StartMaterialization::RevisionRejected(rejection));
        }

        insert_created_execution(
            &mut tx,
            keyed.scope,
            keyed.execution_id,
            keyed.execution.workflow_id,
            keyed.execution.initial_state,
        )
        .await?;

        let resume_target_json = keyed
            .command
            .resume_target
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        sqlx::query(
            "INSERT INTO port_control_queue \
             (id, execution_id, workspace_id, org_id, command, status, \
              w3c_traceparent, reclaim_count, resume_target) \
             VALUES ($1, $2, $3, $4, $5, 'Pending', $6, $7, $8)",
        )
        .bind(keyed.command.id.as_slice())
        .bind(&keyed.command.execution_id)
        .bind(&keyed.command.scope.workspace_id)
        .bind(&keyed.command.scope.org_id)
        .bind(keyed.command.command.as_str())
        .bind(keyed.command.w3c_traceparent.as_deref())
        .bind(i32::try_from(keyed.command.reclaim_count).unwrap_or(i32::MAX))
        .bind(resume_target_json)
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;

        sqlx::query(
            "INSERT INTO port_execution_revision_refs \
             (execution_id, execution_contract_bundle_id, executable_plan_id, \
              worker_flavor_id, reference_state) \
             VALUES ($1, $2, $3, $4, 'live')",
        )
        .bind(keyed.execution_id)
        .bind(start.identity.bundle_id().as_bytes().as_slice())
        .bind(revisions.plan().as_bytes().as_slice())
        .bind(revisions.worker_flavor().as_bytes().as_slice())
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?;

        tx.commit().await.map_err(conn_err)?;
        tracing::debug!(
            target: "nebula_storage::postgres",
            "start_acceptance: materialized start with a live revision reference"
        );
        Ok(StartMaterialization::Accepted {
            execution_id: keyed.execution_id.to_owned(),
        })
    }

    async fn evict_reservations_older_than(
        &self,
        retention: Duration,
    ) -> Result<u64, StorageError> {
        let cutoff_ms = chrono::Utc::now()
            .timestamp_millis()
            .saturating_sub(i64::try_from(retention.as_millis()).unwrap_or(i64::MAX));
        let deleted =
            sqlx::query("DELETE FROM port_start_key_reservations WHERE created_at_ms < $1")
                .bind(cutoff_ms)
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
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan_id: nebula_core::ExecutablePlanRevisionId,
    worker_flavor_id: nebula_core::WorkerFlavorRevisionId,
) -> Result<Option<StartRevisionRejection>, StorageError> {
    let Some(plan_row) = sqlx::query(
        "SELECT worker_flavor_id, lifecycle FROM port_executable_plan_revisions \
         WHERE executable_plan_id = $1",
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
        "SELECT lifecycle FROM port_worker_flavor_revisions WHERE worker_flavor_id = $1",
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
