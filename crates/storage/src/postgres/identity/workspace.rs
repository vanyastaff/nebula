//! `workspaces`: scoped by parent org; slug is unique among active rows per
//! org, and an org has at most one active default workspace (both partial
//! unique indexes). Workspace-grant writes share-lock the live workspace row,
//! so they serialize with the row-level update and soft delete here.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::WorkspaceRow;
use nebula_storage_port::store::WorkspaceStore;
use sqlx::PgPool;
use sqlx::postgres::PgRow;
use sqlx::types::Json;

use super::{cas_failure, encode_version, json, optional, required, version};
use crate::sql_error::{is_foreign_key_violation, storage_error, storage_error_for};

/// Postgres-backed `workspaces` store.
#[derive(Clone, Debug)]
pub struct PgWorkspaceStore {
    pool: PgPool,
}

impl PgWorkspaceStore {
    /// Wrap a pool whose schema was installed via [`crate::postgres::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

pub(super) fn decode_workspace(row: &PgRow) -> Result<WorkspaceRow, StorageError> {
    Ok(WorkspaceRow {
        id: required(row, "id")?,
        org_id: required(row, "org_id")?,
        slug: required(row, "slug")?,
        display_name: required(row, "display_name")?,
        description: optional(row, "description")?,
        created_at: required(row, "created_at")?,
        created_by: required(row, "created_by")?,
        is_default: required(row, "is_default")?,
        settings: json(row, "settings")?,
        version: version(row)?,
        deleted_at: optional(row, "deleted_at")?,
    })
}

/// Insert `workspace` inside the caller's transaction — shared with tenant
/// provisioning. A taken id, active slug or active default is
/// `Duplicate { entity: "workspace", .. }`; a missing parent org is
/// `NotFound`.
pub(super) async fn insert_workspace<'c, E>(
    executor: E,
    workspace: &WorkspaceRow,
) -> Result<(), StorageError>
where
    E: sqlx::Executor<'c, Database = sqlx::Postgres>,
{
    sqlx::query(
        "INSERT INTO workspaces (id, org_id, slug, display_name, \
         description, created_at, created_by, is_default, settings, version, \
         deleted_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&workspace.id)
    .bind(&workspace.org_id)
    .bind(&workspace.slug)
    .bind(&workspace.display_name)
    .bind(&workspace.description)
    .bind(workspace.created_at)
    .bind(&workspace.created_by)
    .bind(workspace.is_default)
    .bind(Json(&workspace.settings))
    .bind(encode_version(workspace.version)?)
    .bind(workspace.deleted_at)
    .execute(executor)
    .await
    .map_err(|error| {
        if is_foreign_key_violation(&error) {
            StorageError::not_found("org", workspace.org_id.clone())
        } else {
            storage_error_for("workspace", error)
        }
    })?;
    Ok(())
}

#[async_trait::async_trait]
impl WorkspaceStore for PgWorkspaceStore {
    async fn create(&self, row: WorkspaceRow) -> Result<(), StorageError> {
        insert_workspace(&self.pool, &row).await
    }

    async fn get(&self, org_id: &str, id: &str) -> Result<Option<WorkspaceRow>, StorageError> {
        sqlx::query(
            "SELECT * FROM workspaces \
             WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL",
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .as_ref()
        .map(decode_workspace)
        .transpose()
    }

    async fn get_by_slug(
        &self,
        org_id: &str,
        slug: &str,
    ) -> Result<Option<WorkspaceRow>, StorageError> {
        sqlx::query(
            "SELECT * FROM workspaces \
             WHERE org_id = $1 AND slug = $2 AND deleted_at IS NULL",
        )
        .bind(org_id)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .as_ref()
        .map(decode_workspace)
        .transpose()
    }

    async fn list_for_org(&self, org_id: &str) -> Result<Vec<WorkspaceRow>, StorageError> {
        sqlx::query(
            "SELECT * FROM workspaces \
             WHERE org_id = $1 AND deleted_at IS NULL ORDER BY id",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?
        .iter()
        .map(decode_workspace)
        .collect()
    }

    async fn update(&self, row: WorkspaceRow, expected_version: u64) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE workspaces SET slug = $1, display_name = $2, \
             description = $3, is_default = $4, settings = $5, version = $6 \
             WHERE org_id = $7 AND id = $8 AND deleted_at IS NULL AND version = $9",
        )
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.description)
        .bind(row.is_default)
        .bind(Json(&row.settings))
        .bind(encode_version(row.version)?)
        .bind(&row.org_id)
        .bind(&row.id)
        .bind(encode_version(expected_version)?)
        .execute(&self.pool)
        .await
        .map_err(|error| storage_error_for("workspace", error))?;
        if res.rows_affected() > 0 {
            return Ok(());
        }
        let current = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM workspaces WHERE org_id = $1 AND id = $2",
        )
        .bind(&row.org_id)
        .bind(&row.id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        Err(cas_failure(current, "workspace", row.id, expected_version))
    }

    async fn soft_delete(&self, org_id: &str, id: &str) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE workspaces SET deleted_at = now() \
             WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL",
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        if res.rows_affected() == 0 {
            return Err(StorageError::not_found("workspace", id));
        }
        Ok(())
    }
}
