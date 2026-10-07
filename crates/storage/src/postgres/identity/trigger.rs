//! `triggers`: workspace-scoped workflow triggers.
//!
//! A trigger belongs to its workflow: create and update share-lock the live
//! workflow row first (`NotFound` when it is missing or deleted). The port
//! carries `created_at` / `deleted_at` as RFC 3339 text; they are stored as
//! `TIMESTAMPTZ` and read back at microsecond precision.

use chrono::{DateTime, SecondsFormat, Utc};
use nebula_storage_port::dto::TriggerRow;
use nebula_storage_port::store::TriggerStore;
use nebula_storage_port::{MicrosInstant, Scope, StorageError};
use sqlx::postgres::PgRow;
use sqlx::types::Json;
use sqlx::{PgPool, Postgres, Transaction};

use super::{cas_disambiguate_scoped, encode_version, json, optional, required, version};
use crate::sql_error::{storage_error, storage_error_for};

/// Columns [`decode_trigger`] reads.
const TRIGGER_COLUMNS: &str = "id, workspace_id, workflow_id, slug, display_name, kind, \
     config, state, run_as, webhook_path, created_at, created_by, version, deleted_at";

/// Postgres-backed `triggers` store.
#[derive(Clone, Debug)]
pub struct PgTriggerStore {
    pool: PgPool,
}

impl PgTriggerStore {
    /// Wrap a pool whose schema was installed via [`crate::postgres::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// A port instant (RFC 3339 text) at storage precision.
fn encode_instant(value: &str, column: &'static str) -> Result<DateTime<Utc>, StorageError> {
    DateTime::parse_from_rfc3339(value)
        .map(|instant| MicrosInstant::floor(instant.with_timezone(&Utc)).to_datetime())
        .map_err(|_| StorageError::InvalidInput(format!("trigger `{column}` is not RFC 3339")))
}

/// A stored instant as the port's RFC 3339 text.
fn decode_instant(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

fn decode_trigger(row: &PgRow) -> Result<TriggerRow, StorageError> {
    Ok(TriggerRow {
        id: required(row, "id")?,
        workspace_id: required(row, "workspace_id")?,
        workflow_id: required(row, "workflow_id")?,
        slug: required(row, "slug")?,
        display_name: required(row, "display_name")?,
        kind: required(row, "kind")?,
        config: json(row, "config")?,
        state: required(row, "state")?,
        run_as: optional(row, "run_as")?,
        webhook_path: optional(row, "webhook_path")?,
        created_at: decode_instant(required(row, "created_at")?),
        created_by: required(row, "created_by")?,
        version: version(row)?,
        deleted_at: optional(row, "deleted_at")?.map(decode_instant),
    })
}

/// Share-lock the live workflow `workflow_id` in `scope`, so a concurrent
/// archive serializes with the trigger write; missing or deleted is
/// `NotFound`.
async fn lock_live_workflow(
    tx: &mut Transaction<'_, Postgres>,
    scope: &Scope,
    workflow_id: &str,
) -> Result<(), StorageError> {
    let live: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM workflows \
         WHERE org_id = $1 AND workspace_id = $2 AND id = $3 AND deleted_at IS NULL \
         FOR SHARE",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(workflow_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage_error)?;
    live.map(|_| ())
        .ok_or_else(|| StorageError::not_found("workflow", workflow_id))
}

#[async_trait::async_trait]
impl TriggerStore for PgTriggerStore {
    async fn create(&self, scope: &Scope, row: TriggerRow) -> Result<(), StorageError> {
        let created_at = encode_instant(&row.created_at, "created_at")?;
        let deleted_at = row
            .deleted_at
            .as_deref()
            .map(|value| encode_instant(value, "deleted_at"))
            .transpose()?;
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        lock_live_workflow(&mut tx, scope, &row.workflow_id).await?;
        sqlx::query(
            "INSERT INTO triggers (org_id, workspace_id, id, workflow_id, \
             slug, display_name, kind, config, state, run_as, webhook_path, \
             created_at, created_by, version, deleted_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&row.id)
        .bind(&row.workflow_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(Json(&row.config))
        .bind(&row.state)
        .bind(&row.run_as)
        .bind(&row.webhook_path)
        .bind(created_at)
        .bind(&row.created_by)
        .bind(encode_version(row.version)?)
        .bind(deleted_at)
        .execute(&mut *tx)
        .await
        .map_err(|error| storage_error_for("trigger", error))?;
        tx.commit().await.map_err(storage_error)?;
        Ok(())
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<TriggerRow>, StorageError> {
        let sql = format!(
            "SELECT {TRIGGER_COLUMNS} FROM triggers \
             WHERE org_id = $1 AND workspace_id = $2 AND id = $3 AND deleted_at IS NULL"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&scope.org_id)
            .bind(&scope.workspace_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage_error)?
            .as_ref()
            .map(decode_trigger)
            .transpose()
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<TriggerRow>, StorageError> {
        let sql = format!(
            "SELECT {TRIGGER_COLUMNS} FROM triggers \
             WHERE org_id = $1 AND workspace_id = $2 AND deleted_at IS NULL \
             ORDER BY id COLLATE \"C\""
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&scope.org_id)
            .bind(&scope.workspace_id)
            .fetch_all(&self.pool)
            .await
            .map_err(storage_error)?
            .iter()
            .map(decode_trigger)
            .collect()
    }

    async fn update(
        &self,
        scope: &Scope,
        row: TriggerRow,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        lock_live_workflow(&mut tx, scope, &row.workflow_id).await?;
        let res = sqlx::query(
            "UPDATE triggers SET workflow_id = $1, slug = $2, \
             display_name = $3, kind = $4, config = $5, state = $6, \
             run_as = $7, webhook_path = $8, version = $9 \
             WHERE org_id = $10 AND workspace_id = $11 AND id = $12 \
             AND deleted_at IS NULL AND version = $13",
        )
        .bind(&row.workflow_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(Json(&row.config))
        .bind(&row.state)
        .bind(&row.run_as)
        .bind(&row.webhook_path)
        .bind(encode_version(row.version)?)
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&row.id)
        .bind(encode_version(expected_version)?)
        .execute(&mut *tx)
        .await
        .map_err(|error| storage_error_for("trigger", error))?;
        tx.commit().await.map_err(storage_error)?;
        if res.rows_affected() > 0 {
            return Ok(());
        }
        cas_disambiguate_scoped(
            &self.pool,
            "triggers",
            "trigger",
            scope,
            &row.id,
            expected_version,
        )
        .await
    }

    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE triggers SET deleted_at = now() \
             WHERE org_id = $1 AND workspace_id = $2 AND id = $3 AND deleted_at IS NULL",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        if res.rows_affected() > 0 {
            Ok(())
        } else {
            Err(StorageError::not_found("trigger", id))
        }
    }
}
