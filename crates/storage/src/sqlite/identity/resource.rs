//! `resources`: the resource definitions a workspace stores; slug is unique
//! among live rows of a workspace.
//!
//! A resource belongs to its workspace: create checks the workspace is live
//! inside its `BEGIN IMMEDIATE` transaction (`NotFound` when it or its org is
//! missing or archived). The port carries `created_at` / `deleted_at` as
//! RFC 3339 text; they are stored as INTEGER microseconds and read back at
//! that precision.

use nebula_storage_port::dto::ResourceRow;
use nebula_storage_port::store::ResourceStore;
use nebula_storage_port::{Scope, StorageError};
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteRow;

use super::{
    cas_disambiguate_scoped, decode_text_instant, encode_text_instant, encode_version,
    ensure_live_workspace, instant, json, json_text, optional_instant, optional_json, required,
    soft_delete_scoped, version,
};
use crate::sql_error::{storage_error, storage_error_for};

/// Columns [`decode_resource`] reads.
const RESOURCE_COLUMNS: &str = "id, workspace_id, slug, display_name, kind, config, \
     credential_bindings, topology, resilience_override, created_at, created_by, version, \
     deleted_at";

/// SQLite-backed `resources` store.
#[derive(Clone, Debug)]
pub struct SqliteResourceStore {
    pool: SqlitePool,
}

impl SqliteResourceStore {
    /// Wrap a pool whose schema was installed via [`crate::sqlite::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

pub(super) fn decode_resource(row: &SqliteRow) -> Result<ResourceRow, StorageError> {
    Ok(ResourceRow {
        id: required(row, "id")?,
        workspace_id: required(row, "workspace_id")?,
        slug: required(row, "slug")?,
        display_name: required(row, "display_name")?,
        kind: required(row, "kind")?,
        config: json(row, "config")?,
        credential_bindings: json(row, "credential_bindings")?,
        topology: optional_json(row, "topology")?,
        resilience_override: optional_json(row, "resilience_override")?,
        created_at: decode_text_instant(instant(row, "created_at")?),
        created_by: required(row, "created_by")?,
        version: version(row)?,
        deleted_at: optional_instant(row, "deleted_at")?.map(decode_text_instant),
    })
}

fn credential_bindings_text(row: &ResourceRow) -> Result<String, StorageError> {
    serde_json::to_string(&row.credential_bindings).map_err(|_| {
        StorageError::Serialization("resource credential bindings do not serialize".into())
    })
}

#[async_trait::async_trait]
impl ResourceStore for SqliteResourceStore {
    async fn create(&self, scope: &Scope, row: ResourceRow) -> Result<(), StorageError> {
        let created_at = encode_text_instant("resource", &row.created_at, "created_at")?;
        let deleted_at = row
            .deleted_at
            .as_deref()
            .map(|value| encode_text_instant("resource", value, "deleted_at"))
            .transpose()?;
        let credential_bindings = credential_bindings_text(&row)?;
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        ensure_live_workspace(&mut tx, scope).await?;
        sqlx::query(
            "INSERT INTO resources (org_id, workspace_id, id, slug, display_name, kind, \
             config, credential_bindings, topology, resilience_override, created_at, \
             created_by, version, deleted_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&row.id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(json_text(&row.config))
        .bind(credential_bindings)
        .bind(row.topology.as_ref().map(json_text))
        .bind(row.resilience_override.as_ref().map(json_text))
        .bind(created_at)
        .bind(&row.created_by)
        .bind(encode_version(row.version)?)
        .bind(deleted_at)
        .execute(&mut *tx)
        .await
        .map_err(|error| storage_error_for("resource", error))?;
        tx.commit().await.map_err(storage_error)?;
        Ok(())
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ResourceRow>, StorageError> {
        let sql = format!(
            "SELECT {RESOURCE_COLUMNS} FROM resources \
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
            .map(decode_resource)
            .transpose()
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<ResourceRow>, StorageError> {
        let sql = format!(
            "SELECT {RESOURCE_COLUMNS} FROM resources \
             WHERE org_id = ? AND workspace_id = ? AND deleted_at IS NULL ORDER BY id"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&scope.org_id)
            .bind(&scope.workspace_id)
            .fetch_all(&self.pool)
            .await
            .map_err(storage_error)?
            .iter()
            .map(decode_resource)
            .collect()
    }

    async fn update(
        &self,
        scope: &Scope,
        row: ResourceRow,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE resources SET slug = ?, display_name = ?, kind = ?, \
             config = ?, credential_bindings = ?, topology = ?, resilience_override = ?, \
             version = ? \
             WHERE org_id = ? AND workspace_id = ? AND id = ? \
             AND deleted_at IS NULL AND version = ?",
        )
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(json_text(&row.config))
        .bind(credential_bindings_text(&row)?)
        .bind(row.topology.as_ref().map(json_text))
        .bind(row.resilience_override.as_ref().map(json_text))
        .bind(encode_version(row.version)?)
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&row.id)
        .bind(encode_version(expected_version)?)
        .execute(&self.pool)
        .await
        .map_err(|error| storage_error_for("resource", error))?;
        if res.rows_affected() > 0 {
            return Ok(());
        }
        cas_disambiguate_scoped(
            &self.pool,
            "resources",
            "resource",
            scope,
            &row.id,
            expected_version,
        )
        .await
    }

    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError> {
        soft_delete_scoped(&self.pool, "resources", "resource", scope, id).await
    }
}
