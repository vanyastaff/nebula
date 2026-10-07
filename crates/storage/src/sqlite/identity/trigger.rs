//! `triggers`: workspace-scoped workflow triggers.
//!
//! A trigger belongs to its workflow: create and update check the workflow is
//! live inside the same write transaction (`NotFound` when it is missing or
//! deleted). The port carries `created_at` / `deleted_at` as RFC 3339 text;
//! they are stored as INTEGER microseconds and read back at that precision.

use chrono::{DateTime, SecondsFormat, Utc};
use nebula_storage_port::dto::TriggerRow;
use nebula_storage_port::store::TriggerStore;
use nebula_storage_port::{Scope, StorageError};
use sqlx::sqlite::SqliteRow;
use sqlx::{Sqlite, SqlitePool, Transaction};

use super::{
    cas_disambiguate_scoped, encode_instant, encode_version, instant, json, json_text, now_micros,
    optional, optional_instant, required, version,
};
use crate::sql_error::{storage_error, storage_error_for};

/// Columns [`decode_trigger`] reads.
const TRIGGER_COLUMNS: &str = "id, workspace_id, workflow_id, slug, display_name, kind, \
     config, state, run_as, webhook_path, created_at, created_by, version, deleted_at";

/// SQLite-backed `triggers` store.
#[derive(Clone, Debug)]
pub struct SqliteTriggerStore {
    pool: SqlitePool,
}

impl SqliteTriggerStore {
    /// Wrap a pool whose schema was installed via [`crate::sqlite::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

/// A port instant (RFC 3339 text) as INTEGER microseconds.
fn encode_text_instant(value: &str, column: &'static str) -> Result<i64, StorageError> {
    DateTime::parse_from_rfc3339(value)
        .map(|instant| encode_instant(instant.with_timezone(&Utc)))
        .map_err(|_| StorageError::InvalidInput(format!("trigger `{column}` is not RFC 3339")))
}

/// A stored instant as the port's RFC 3339 text.
fn decode_text_instant(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

pub(super) fn decode_trigger(row: &SqliteRow) -> Result<TriggerRow, StorageError> {
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
        created_at: decode_text_instant(instant(row, "created_at")?),
        created_by: required(row, "created_by")?,
        version: version(row)?,
        deleted_at: optional_instant(row, "deleted_at")?.map(decode_text_instant),
    })
}

/// Check the workflow `workflow_id` is live in `scope` inside a write
/// transaction; missing or deleted is `NotFound`.
async fn ensure_live_workflow(
    tx: &mut Transaction<'_, Sqlite>,
    scope: &Scope,
    workflow_id: &str,
) -> Result<(), StorageError> {
    let live: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM workflows \
         WHERE org_id = ? AND workspace_id = ? AND id = ? AND deleted_at IS NULL",
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
impl TriggerStore for SqliteTriggerStore {
    async fn create(&self, scope: &Scope, row: TriggerRow) -> Result<(), StorageError> {
        let created_at = encode_text_instant(&row.created_at, "created_at")?;
        let deleted_at = row
            .deleted_at
            .as_deref()
            .map(|value| encode_text_instant(value, "deleted_at"))
            .transpose()?;
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        ensure_live_workflow(&mut tx, scope, &row.workflow_id).await?;
        sqlx::query(
            "INSERT INTO triggers (org_id, workspace_id, id, workflow_id, \
             slug, display_name, kind, config, state, run_as, webhook_path, \
             created_at, created_by, version, deleted_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&row.id)
        .bind(&row.workflow_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(json_text(&row.config))
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
             WHERE org_id = ? AND workspace_id = ? AND id = ? AND deleted_at IS NULL"
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
             WHERE org_id = ? AND workspace_id = ? AND deleted_at IS NULL ORDER BY id"
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
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        ensure_live_workflow(&mut tx, scope, &row.workflow_id).await?;
        let res = sqlx::query(
            "UPDATE triggers SET workflow_id = ?, slug = ?, \
             display_name = ?, kind = ?, config = ?, state = ?, run_as = ?, \
             webhook_path = ?, version = ? WHERE org_id = ? AND workspace_id = ? \
             AND id = ? AND deleted_at IS NULL AND version = ?",
        )
        .bind(&row.workflow_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(json_text(&row.config))
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
            "UPDATE triggers SET deleted_at = ? \
             WHERE org_id = ? AND workspace_id = ? AND id = ? AND deleted_at IS NULL",
        )
        .bind(now_micros())
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
