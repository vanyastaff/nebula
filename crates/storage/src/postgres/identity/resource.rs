//! `port_resources`: workspace-scoped; slug is unique among active rows per
//! workspace scope.

use nebula_storage_port::dto::ResourceRow;
use nebula_storage_port::store::ResourceStore;
use nebula_storage_port::{Scope, StorageError};
use sqlx::PgPool;
use sqlx::postgres::PgRow;
use sqlx::types::Json;

use super::{
    cas_disambiguate_scoped, encode_version, json, optional, optional_json, required,
    soft_delete_scoped, version,
};
use crate::sql_error::{storage_error, storage_error_for};

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
        created_at: required(row, "created_at")?,
        created_by: required(row, "created_by")?,
        version: version(row)?,
        deleted_at: optional(row, "deleted_at")?,
    })
}

#[async_trait::async_trait]
impl ResourceStore for PgResourceStore {
    async fn create(&self, scope: &Scope, row: ResourceRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO port_resources (id, workspace_id, org_id, slug, \
             display_name, kind, config, credential_bindings, topology, \
             resilience_override, created_at, created_by, version, deleted_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(&row.id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.kind)
        .bind(Json(&row.config))
        .bind(Json(&row.credential_bindings))
        .bind(row.topology.as_ref().map(Json))
        .bind(row.resilience_override.as_ref().map(Json))
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
             WHERE workspace_id = $1 AND org_id = $2 AND id = $3 AND deleted_at IS NULL",
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
             WHERE workspace_id = $1 AND org_id = $2 AND deleted_at IS NULL ORDER BY id",
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
            "UPDATE port_resources SET slug = $1, display_name = $2, kind = $3, \
             config = $4, credential_bindings = $5, topology = $6, resilience_override = $7, \
             version = $8 \
             WHERE workspace_id = $9 AND org_id = $10 AND id = $11 \
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
