//! `port_workspaces`: scoped by parent org; slug is unique among active rows
//! per org, and an org has at most one active default workspace.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::WorkspaceRow;
use nebula_storage_port::store::WorkspaceStore;
use sqlx::sqlite::SqliteRow;
use sqlx::{SqliteConnection, SqlitePool};

use super::{
    cas_failure, encode_version, flag, json, json_text, now_rfc3339, optional, required, version,
};
use crate::sql_error::{storage_error, storage_error_for};

/// SQLite-backed `workspaces` store.
#[derive(Clone, Debug)]
pub struct SqliteWorkspaceStore {
    pool: SqlitePool,
}

impl SqliteWorkspaceStore {
    /// Wrap a pool whose schema was installed via [`crate::sqlite::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

pub(super) fn decode_workspace(row: &SqliteRow) -> Result<WorkspaceRow, StorageError> {
    Ok(WorkspaceRow {
        id: required(row, "id")?,
        org_id: required(row, "org_id")?,
        slug: required(row, "slug")?,
        display_name: required(row, "display_name")?,
        description: optional(row, "description")?,
        created_at: required(row, "created_at")?,
        created_by: required(row, "created_by")?,
        is_default: flag(row, "is_default")?,
        settings: json(row, "settings")?,
        version: version(row)?,
        deleted_at: optional(row, "deleted_at")?,
    })
}

/// Insert `workspace` inside the caller's transaction — shared with tenant
/// provisioning.
pub(super) async fn insert_workspace(
    connection: &mut SqliteConnection,
    workspace: &WorkspaceRow,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO port_workspaces (id, org_id, slug, display_name, \
         description, created_at, created_by, is_default, settings, version, \
         deleted_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&workspace.id)
    .bind(&workspace.org_id)
    .bind(&workspace.slug)
    .bind(&workspace.display_name)
    .bind(&workspace.description)
    .bind(&workspace.created_at)
    .bind(&workspace.created_by)
    .bind(i64::from(workspace.is_default))
    .bind(json_text(&workspace.settings))
    .bind(encode_version(workspace.version)?)
    .bind(&workspace.deleted_at)
    .execute(connection)
    .await
    .map_err(|error| storage_error_for("workspace", error))?;
    Ok(())
}

async fn reject_second_active_default(
    connection: &mut SqliteConnection,
    row: &WorkspaceRow,
) -> Result<(), StorageError> {
    if !row.is_default || row.deleted_at.is_some() {
        return Ok(());
    }
    let existing = sqlx::query_scalar::<_, String>(
        "SELECT id FROM port_workspaces \
         WHERE org_id = ?1 AND is_default = 1 AND deleted_at IS NULL AND id <> ?2",
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
impl WorkspaceStore for SqliteWorkspaceStore {
    async fn create(&self, row: WorkspaceRow) -> Result<(), StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        reject_second_active_default(&mut tx, &row).await?;
        insert_workspace(&mut tx, &row).await?;
        tx.commit().await.map_err(storage_error)
    }

    async fn get(&self, org_id: &str, id: &str) -> Result<Option<WorkspaceRow>, StorageError> {
        sqlx::query(
            "SELECT * FROM port_workspaces \
             WHERE org_id = ? AND id = ? AND deleted_at IS NULL",
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
             WHERE org_id = ? AND slug = ? AND deleted_at IS NULL",
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
             WHERE org_id = ? AND deleted_at IS NULL ORDER BY id",
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
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        let res = sqlx::query(
            "UPDATE port_workspaces SET slug = ?, display_name = ?, \
             description = ?, is_default = ?, settings = ?, version = ? \
             WHERE org_id = ? AND id = ? AND deleted_at IS NULL AND version = ?",
        )
        .bind(&row.slug)
        .bind(&row.display_name)
        .bind(&row.description)
        .bind(i64::from(row.is_default))
        .bind(json_text(&row.settings))
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
            "SELECT version FROM port_workspaces WHERE org_id = ? AND id = ?",
        )
        .bind(&row.org_id)
        .bind(&row.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
        Err(cas_failure(current, "workspace", row.id, expected_version))
    }

    async fn soft_delete(&self, org_id: &str, id: &str) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE port_workspaces SET deleted_at = ? \
             WHERE org_id = ? AND id = ? AND deleted_at IS NULL",
        )
        .bind(now_rfc3339())
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        if res.rows_affected() > 0 {
            Ok(())
        } else {
            Err(StorageError::not_found("workspace", id))
        }
    }
}
