//! `port_resources`: workspace-scoped; slug is unique among active rows per
//! workspace scope.

use nebula_storage_port::dto::ResourceRow;
use nebula_storage_port::store::ResourceStore;
use nebula_storage_port::{Scope, StorageError};
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteRow;

use super::{
    cas_disambiguate_scoped, encode_version, json, json_text, optional, optional_json, required,
    soft_delete_scoped, version,
};
use crate::sql_error::{storage_error, storage_error_for};

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
        created_at: required(row, "created_at")?,
        created_by: required(row, "created_by")?,
        version: version(row)?,
        deleted_at: optional(row, "deleted_at")?,
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
        sqlx::query(
            "INSERT INTO port_resources (id, workspace_id, org_id, slug, \
             display_name, kind, config, credential_bindings, topology, \
             resilience_override, created_at, created_by, version, deleted_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(json_text(&row.config))
        .bind(credential_bindings_text(&row)?)
        .bind(row.topology.as_ref().map(json_text))
        .bind(row.resilience_override.as_ref().map(json_text))
        .bind(&row.created_at)
        .bind(&row.created_by)
        .bind(encode_version(row.version)?)
        .bind(&row.deleted_at)
        .execute(&self.pool)
        .await
        .map_err(|error| storage_error_for("resource", error))?;
        Ok(())
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ResourceRow>, StorageError> {
        sqlx::query(
            "SELECT * FROM port_resources \
             WHERE workspace_id = ? AND org_id = ? AND id = ? AND deleted_at IS NULL",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .as_ref()
        .map(decode_resource)
        .transpose()
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<ResourceRow>, StorageError> {
        sqlx::query(
            "SELECT * FROM port_resources \
             WHERE workspace_id = ? AND org_id = ? AND deleted_at IS NULL ORDER BY id",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
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
            "UPDATE port_resources SET slug = ?, display_name = ?, kind = ?, \
             config = ?, credential_bindings = ?, topology = ?, resilience_override = ?, \
             version = ? \
             WHERE workspace_id = ? AND org_id = ? AND id = ? \
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
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
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
            "port_resources",
            "resource",
            scope,
            &row.id,
            expected_version,
        )
        .await
    }

    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError> {
        soft_delete_scoped(&self.pool, "port_resources", "resource", scope, id).await
    }
}
