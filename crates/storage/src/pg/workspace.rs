//! Postgres implementation of [`WorkspaceRepo`].

use async_trait::async_trait;
use sqlx::{Pool, Postgres, types::Json};

use crate::{
    error::StorageError,
    pg::map_db_err,
    repos::WorkspaceRepo,
    rows::{WorkspaceMemberRow, WorkspaceRow},
};

/// Postgres-backed workspace repository.
#[derive(Clone)]
pub struct PgWorkspaceRepo {
    pool: Pool<Postgres>,
}

impl PgWorkspaceRepo {
    /// Construct from an existing pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }
}

type WsTuple = (
    Vec<u8>,                               // id
    Vec<u8>,                               // org_id
    String,                                // slug
    String,                                // display_name
    Option<String>,                        // description
    chrono::DateTime<chrono::Utc>,         // created_at
    Vec<u8>,                               // created_by
    bool,                                  // is_default
    Json<serde_json::Value>,               // settings
    i64,                                   // version
    Option<chrono::DateTime<chrono::Utc>>, // deleted_at
);

fn tuple_to_row(t: WsTuple) -> WorkspaceRow {
    WorkspaceRow {
        id: t.0,
        org_id: t.1,
        slug: t.2,
        display_name: t.3,
        description: t.4,
        created_at: t.5,
        created_by: t.6,
        is_default: t.7,
        settings: t.8.0,
        version: t.9,
        deleted_at: t.10,
    }
}

const SELECT_COLS: &str = "id, org_id, slug, display_name, description, created_at, created_by, is_default, settings, version, deleted_at";

#[async_trait]
impl WorkspaceRepo for PgWorkspaceRepo {
    async fn create(&self, ws: &WorkspaceRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO workspaces \
             (id, org_id, slug, display_name, description, created_at, created_by, \
              is_default, settings, version, deleted_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(&ws.id)
        .bind(&ws.org_id)
        .bind(&ws.slug)
        .bind(&ws.display_name)
        .bind(ws.description.as_deref())
        .bind(ws.created_at)
        .bind(&ws.created_by)
        .bind(ws.is_default)
        .bind(Json(&ws.settings))
        .bind(ws.version)
        .bind(ws.deleted_at)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("workspace", e))?;
        Ok(())
    }

    async fn get(&self, id: &[u8]) -> Result<Option<WorkspaceRow>, StorageError> {
        let sql =
            format!("SELECT {SELECT_COLS} FROM workspaces WHERE id = $1 AND deleted_at IS NULL");
        let row = sqlx::query_as::<_, WsTuple>(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("workspace", e))?;
        Ok(row.map(tuple_to_row))
    }

    async fn get_by_slug(
        &self,
        org_id: &[u8],
        slug: &str,
    ) -> Result<Option<WorkspaceRow>, StorageError> {
        let sql = format!(
            "SELECT {SELECT_COLS} FROM workspaces \
             WHERE org_id = $1 AND LOWER(slug) = LOWER($2) AND deleted_at IS NULL"
        );
        let row = sqlx::query_as::<_, WsTuple>(&sql)
            .bind(org_id)
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("workspace", e))?;
        Ok(row.map(tuple_to_row))
    }

    async fn get_default(&self, org_id: &[u8]) -> Result<Option<WorkspaceRow>, StorageError> {
        let sql = format!(
            "SELECT {SELECT_COLS} FROM workspaces \
             WHERE org_id = $1 AND is_default = TRUE AND deleted_at IS NULL"
        );
        let row = sqlx::query_as::<_, WsTuple>(&sql)
            .bind(org_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("workspace", e))?;
        Ok(row.map(tuple_to_row))
    }

    async fn list_for_org(&self, org_id: &[u8]) -> Result<Vec<WorkspaceRow>, StorageError> {
        let sql = format!(
            "SELECT {SELECT_COLS} FROM workspaces \
             WHERE org_id = $1 AND deleted_at IS NULL \
             ORDER BY is_default DESC, created_at"
        );
        let rows = sqlx::query_as::<_, WsTuple>(&sql)
            .bind(org_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| map_db_err("workspace", e))?;
        Ok(rows.into_iter().map(tuple_to_row).collect())
    }

    async fn update(&self, ws: &WorkspaceRow, expected_version: i64) -> Result<(), StorageError> {
        let rows = sqlx::query(
            "UPDATE workspaces SET \
                 slug = $2, display_name = $3, description = $4, is_default = $5, \
                 settings = $6, version = version + 1 \
             WHERE id = $1 AND version = $7 AND deleted_at IS NULL",
        )
        .bind(&ws.id)
        .bind(&ws.slug)
        .bind(&ws.display_name)
        .bind(ws.description.as_deref())
        .bind(ws.is_default)
        .bind(Json(&ws.settings))
        .bind(expected_version)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("workspace", e))?
        .rows_affected();

        if rows == 0 {
            let actual: Option<i64> =
                sqlx::query_scalar("SELECT version FROM workspaces WHERE id = $1")
                    .bind(&ws.id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| map_db_err("workspace", e))?;
            return match actual {
                Some(v) => Err(StorageError::conflict(
                    "workspace",
                    hex::encode(&ws.id),
                    expected_version,
                    v,
                )),
                None => Err(StorageError::not_found("workspace", hex::encode(&ws.id))),
            };
        }
        Ok(())
    }

    async fn soft_delete(&self, id: &[u8]) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE workspaces SET deleted_at = NOW(), version = version + 1 \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("workspace", e))?;
        Ok(())
    }

    async fn add_member(&self, member: &WorkspaceMemberRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO workspace_members \
             (workspace_id, principal_kind, principal_id, role, added_at, added_by) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&member.workspace_id)
        .bind(&member.principal_kind)
        .bind(&member.principal_id)
        .bind(&member.role)
        .bind(member.added_at)
        .bind(&member.added_by)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("workspace_member", e))?;
        Ok(())
    }

    async fn remove_member(
        &self,
        workspace_id: &[u8],
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<(), StorageError> {
        sqlx::query(
            "DELETE FROM workspace_members \
             WHERE workspace_id = $1 AND principal_kind = $2 AND principal_id = $3",
        )
        .bind(workspace_id)
        .bind(principal_kind)
        .bind(principal_id)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("workspace_member", e))?;
        Ok(())
    }

    async fn get_member_role(
        &self,
        workspace_id: &[u8],
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<Option<String>, StorageError> {
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM workspace_members \
             WHERE workspace_id = $1 AND principal_kind = $2 AND principal_id = $3",
        )
        .bind(workspace_id)
        .bind(principal_kind)
        .bind(principal_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| map_db_err("workspace_member", e))?;
        Ok(role)
    }

    async fn list_members(
        &self,
        workspace_id: &[u8],
    ) -> Result<Vec<WorkspaceMemberRow>, StorageError> {
        type T = (
            Vec<u8>,
            String,
            Vec<u8>,
            String,
            chrono::DateTime<chrono::Utc>,
            Vec<u8>,
        );
        let rows = sqlx::query_as::<_, T>(
            "SELECT workspace_id, principal_kind, principal_id, role, added_at, added_by \
             FROM workspace_members WHERE workspace_id = $1 ORDER BY added_at",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| map_db_err("workspace_member", e))?;

        Ok(rows
            .into_iter()
            .map(|t| WorkspaceMemberRow {
                workspace_id: t.0,
                principal_kind: t.1,
                principal_id: t.2,
                role: t.3,
                added_at: t.4,
                added_by: t.5,
            })
            .collect())
    }
}
