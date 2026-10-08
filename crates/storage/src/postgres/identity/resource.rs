//! `resources`: the resource definitions a workspace stores; slug is unique
//! among live rows of a workspace.
//!
//! A resource belongs to its workspace: create share-locks the live workspace
//! first (`NotFound` when it or its org is missing or archived). The port
//! carries `created_at` / `deleted_at` as RFC 3339 text; they are stored as
//! `TIMESTAMPTZ` and read back at microsecond precision.

use nebula_storage_port::dto::ResourceRow;
use nebula_storage_port::store::ResourceStore;
use nebula_storage_port::{Scope, StorageError};
use sqlx::PgPool;
use sqlx::postgres::PgRow;
use sqlx::types::Json;

use super::{
    cas_disambiguate_scoped, decode_text_instant, encode_text_instant, encode_version, json,
    lock_live_workspace, optional, optional_json, required, soft_delete_scoped, version,
};
use crate::sql_error::{storage_error, storage_error_for};

/// Columns [`decode_resource`] reads.
const RESOURCE_COLUMNS: &str = "id, workspace_id, slug, display_name, kind, config, \
     credential_bindings, topology, resilience_override, created_at, created_by, version, \
     deleted_at";

/// Postgres-backed `resources` store.
#[derive(Clone, Debug)]
pub struct PgResourceStore {
    pool: PgPool,
}

impl PgResourceStore {
    /// Wrap a pool whose schema was installed via [`crate::postgres::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn decode_resource(row: &PgRow) -> Result<ResourceRow, StorageError> {
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
        created_at: decode_text_instant(required(row, "created_at")?),
        created_by: required(row, "created_by")?,
        version: version(row)?,
        deleted_at: optional(row, "deleted_at")?.map(decode_text_instant),
    })
}

#[async_trait::async_trait]
impl ResourceStore for PgResourceStore {
    async fn create(&self, scope: &Scope, row: ResourceRow) -> Result<(), StorageError> {
        let created_at = encode_text_instant("resource", &row.created_at, "created_at")?;
        let deleted_at = row
            .deleted_at
            .as_deref()
            .map(|value| encode_text_instant("resource", value, "deleted_at"))
            .transpose()?;
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        lock_live_workspace(&mut tx, scope).await?;
        sqlx::query(
            "INSERT INTO resources (org_id, workspace_id, id, slug, display_name, kind, \
             config, credential_bindings, topology, resilience_override, created_at, \
             created_by, version, deleted_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&row.id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(Json(&row.config))
        .bind(Json(&row.credential_bindings))
        .bind(row.topology.as_ref().map(Json))
        .bind(row.resilience_override.as_ref().map(Json))
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
            .map(decode_resource)
            .transpose()
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<ResourceRow>, StorageError> {
        let sql = format!(
            "SELECT {RESOURCE_COLUMNS} FROM resources \
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
            "UPDATE resources SET slug = $1, display_name = $2, kind = $3, \
             config = $4, credential_bindings = $5, topology = $6, resilience_override = $7, \
             version = $8 \
             WHERE org_id = $9 AND workspace_id = $10 AND id = $11 \
             AND deleted_at IS NULL AND version = $12",
        )
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(Json(&row.config))
        .bind(Json(&row.credential_bindings))
        .bind(row.topology.as_ref().map(Json))
        .bind(row.resilience_override.as_ref().map(Json))
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
