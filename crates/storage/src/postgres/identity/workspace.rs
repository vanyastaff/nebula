//! `port_workspaces`: scoped by parent org; slug is unique among active rows
//! per org, and an org has at most one active default workspace. Every
//! mutation takes the org and workspace-id advisory locks first.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::WorkspaceRow;
use nebula_storage_port::store::WorkspaceStore;
use sqlx::postgres::PgRow;
use sqlx::types::Json;
use sqlx::{PgConnection, PgPool};

use super::{
    cas_failure, encode_version, json, lock_workspace_identity, lock_workspace_org, now_rfc3339,
    optional, required, version,
};
use crate::sql_error::{storage_error, storage_error_for};

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
/// provisioning. The caller holds the advisory locks.
pub(super) async fn insert_workspace(
    connection: &mut PgConnection,
    workspace: &WorkspaceRow,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO port_workspaces (id, org_id, slug, display_name, \
         description, created_at, created_by, is_default, settings, version, \
         deleted_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&workspace.id)
    .bind(&workspace.org_id)
    .bind(&workspace.slug)
    .bind(&workspace.display_name)
    .bind(&workspace.description)
    .bind(&workspace.created_at)
    .bind(&workspace.created_by)
    .bind(workspace.is_default)
    .bind(Json(&workspace.settings))
    .bind(encode_version(workspace.version)?)
    .bind(&workspace.deleted_at)
    .execute(connection)
    .await
    .map_err(|error| storage_error_for("workspace", error))?;
    Ok(())
}

async fn reject_second_active_default(
    connection: &mut PgConnection,
    row: &WorkspaceRow,
) -> Result<(), StorageError> {
    if !row.is_default || row.deleted_at.is_some() {
        return Ok(());
    }
    let existing = sqlx::query_scalar::<_, String>(
        "SELECT id FROM port_workspaces \
         WHERE org_id = $1 AND is_default = TRUE AND deleted_at IS NULL AND id <> $2",
    )
    .bind(&row.org_id)
    .bind(&row.id)
    .fetch_optional(connection)
    .await
    .map_err(storage_error)?;
    if let Some(existing_id) = existing {
        return Err(StorageError::Duplicate {
            entity: "workspace",
            detail: format!(
                "organization {} already has active default workspace {existing_id}",
                row.org_id
            ),
        });
    }
    Ok(())
}

#[async_trait::async_trait]
impl WorkspaceStore for PgWorkspaceStore {
    async fn create(&self, row: WorkspaceRow) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        lock_workspace_org(&mut tx, &row.org_id).await?;
        lock_workspace_identity(&mut tx, &row.id).await?;
        reject_second_active_default(&mut tx, &row).await?;
        insert_workspace(&mut tx, &row).await?;
        tx.commit().await.map_err(storage_error)
    }

    async fn get(&self, org_id: &str, id: &str) -> Result<Option<WorkspaceRow>, StorageError> {
        sqlx::query(
            "SELECT * FROM port_workspaces \
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
            "SELECT * FROM port_workspaces \
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
            "SELECT * FROM port_workspaces \
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
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        lock_workspace_org(&mut tx, &row.org_id).await?;
        lock_workspace_identity(&mut tx, &row.id).await?;
        let res = sqlx::query(
            "UPDATE port_workspaces SET slug = $1, display_name = $2, \
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
        .execute(&mut *tx)
        .await
        .map_err(|error| storage_error_for("workspace", error))?;
        if res.rows_affected() > 0 {
            reject_second_active_default(&mut tx, &row).await?;
            return tx.commit().await.map_err(storage_error);
        }
        let current = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM port_workspaces WHERE org_id = $1 AND id = $2",
        )
        .bind(&row.org_id)
        .bind(&row.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
        Err(cas_failure(current, "workspace", row.id, expected_version))
    }

    async fn soft_delete(&self, org_id: &str, id: &str) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        lock_workspace_org(&mut tx, org_id).await?;
        lock_workspace_identity(&mut tx, id).await?;
        let res = sqlx::query(
            "UPDATE port_workspaces SET deleted_at = $1 \
             WHERE org_id = $2 AND id = $3 AND deleted_at IS NULL",
        )
        .bind(now_rfc3339())
        .bind(org_id)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        if res.rows_affected() == 0 {
            return Err(StorageError::not_found("workspace", id));
        }
        tx.commit().await.map_err(storage_error)
    }
}
