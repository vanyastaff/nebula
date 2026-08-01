//! Postgres [`StartAcceptanceStore`] over the port-scoped schema.
//!
//! The reservation, the execution aggregate row, and the Start control row are
//! three statements in **one** transaction, so a start key can never be
//! reserved for an execution that does not exist, and an execution can never
//! exist without the Start command that drives it.

use std::time::Duration;

use nebula_storage_port::StorageError;
use nebula_storage_port::store::{KeyedStart, StartAcceptance, StartAcceptanceStore};
use sqlx::{PgPool, Row};

use super::execution::{conn_err, insert_created_execution};

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
