//! `org_memberships` and `workspace_memberships`: explicit grants.
//!
//! Every mutation runs under `BEGIN IMMEDIATE`, which serializes its
//! invariant read with every writer. Removing an organization grant cascades
//! to the principal's workspace grants through the foreign key.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::{
    OrgMemberRemoveOutcome, OrgMemberUpsert, OrgMemberUpsertOutcome, OrgMembership,
    OrgMembershipRole, PrincipalKind, PrincipalOrgMembership, TenantMembershipSnapshot,
    WorkspaceMemberUpsert, WorkspaceMembership, WorkspaceMembershipRole,
};
use nebula_storage_port::store::MembershipStore;
use sqlx::sqlite::SqliteRow;
use sqlx::{SqliteConnection, SqlitePool};

use super::{now_micros, optional, required};
use crate::sql_error::storage_error;

/// SQLite-backed `org_memberships` + `workspace_memberships` store.
#[derive(Clone, Debug)]
pub struct SqliteMembershipStore {
    pool: SqlitePool,
}

impl SqliteMembershipStore {
    /// Wrap a pool whose schema was installed via [`crate::sqlite::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn unknown_value(column: &'static str) -> StorageError {
    StorageError::Corrupt(format!("column `{column}` holds an unknown value"))
}

// Fail closed: an unrecognized authz-domain value is corrupt, never coerced
// to a default.
fn principal_kind(row: &SqliteRow) -> Result<PrincipalKind, StorageError> {
    PrincipalKind::parse(&required::<String>(row, "principal_kind")?)
        .map_err(|_| unknown_value("principal_kind"))
}

pub(super) fn org_role(row: &SqliteRow) -> Result<OrgMembershipRole, StorageError> {
    OrgMembershipRole::parse(&required::<String>(row, "role")?).map_err(|_| unknown_value("role"))
}

fn workspace_role(row: &SqliteRow) -> Result<WorkspaceMembershipRole, StorageError> {
    WorkspaceMembershipRole::parse(&required::<String>(row, "role")?)
        .map_err(|_| unknown_value("role"))
}

async fn live_org_exists(
    connection: &mut SqliteConnection,
    org_id: &str,
) -> Result<bool, StorageError> {
    Ok(
        sqlx::query("SELECT id FROM orgs WHERE id = ?1 AND deleted_at IS NULL")
            .bind(org_id)
            .fetch_optional(connection)
            .await
            .map_err(storage_error)?
            .is_some(),
    )
}

/// `true` for a live workspace under `org_id` in a live organization.
async fn live_workspace_exists(
    connection: &mut SqliteConnection,
    org_id: &str,
    workspace_id: &str,
) -> Result<bool, StorageError> {
    Ok(sqlx::query(
        "SELECT w.id FROM workspaces w JOIN orgs o ON o.id = w.org_id \
         WHERE w.org_id = ?1 AND w.id = ?2 \
         AND w.deleted_at IS NULL AND o.deleted_at IS NULL",
    )
    .bind(org_id)
    .bind(workspace_id)
    .fetch_optional(connection)
    .await
    .map_err(storage_error)?
    .is_some())
}

async fn org_grants(
    connection: &mut SqliteConnection,
    org_id: &str,
) -> Result<Vec<SqliteRow>, StorageError> {
    sqlx::query("SELECT principal_kind, principal_id, role FROM org_memberships WHERE org_id = ?1")
        .bind(org_id)
        .fetch_all(connection)
        .await
        .map_err(storage_error)
}

#[async_trait::async_trait]
impl MembershipStore for SqliteMembershipStore {
    #[tracing::instrument(skip_all)]
    async fn get_tenant_membership(
        &self,
        org_id: &str,
        workspace_id: Option<&str>,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<TenantMembershipSnapshot, StorageError> {
        // One statement, one snapshot: the org grant and the workspace grant
        // of a live workspace in a live org.
        let row = sqlx::query(
            "SELECT \
             (SELECT role FROM org_memberships \
              WHERE org_id = ?1 AND principal_kind = ?2 AND principal_id = ?3) AS org_role, \
             (SELECT m.role FROM workspace_memberships m \
              JOIN workspaces w ON w.org_id = m.org_id AND w.id = m.workspace_id \
              JOIN orgs o ON o.id = w.org_id \
              WHERE m.org_id = ?1 AND m.workspace_id = ?4 \
              AND m.principal_kind = ?2 AND m.principal_id = ?3 \
              AND w.deleted_at IS NULL AND o.deleted_at IS NULL) AS workspace_role",
        )
        .bind(org_id)
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .bind(workspace_id)
        .fetch_one(&self.pool)
        .await
        .map_err(storage_error)?;
        let org_role = optional::<String>(&row, "org_role")?
            .map(|role| OrgMembershipRole::parse(&role).map_err(|_| unknown_value("role")))
            .transpose()?;
        let workspace_role = optional::<String>(&row, "workspace_role")?
            .map(|role| WorkspaceMembershipRole::parse(&role).map_err(|_| unknown_value("role")))
            .transpose()?;
        Ok(TenantMembershipSnapshot {
            org_role,
            workspace_role,
        })
    }

    #[tracing::instrument(skip_all)]
    async fn list_orgs_for_principal(
        &self,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<Vec<PrincipalOrgMembership>, StorageError> {
        sqlx::query(
            "SELECT org_id, role FROM org_memberships \
             WHERE principal_kind = ?1 AND principal_id = ?2 ORDER BY org_id",
        )
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?
        .iter()
        .map(|row| {
            Ok(PrincipalOrgMembership {
                role: org_role(row)?,
                org_id: required(row, "org_id")?,
            })
        })
        .collect()
    }

    #[tracing::instrument(skip_all)]
    async fn list_org_members(&self, org_id: &str) -> Result<Vec<OrgMembership>, StorageError> {
        sqlx::query(
            "SELECT principal_kind, principal_id, role FROM org_memberships \
             WHERE org_id = ?1 ORDER BY principal_kind, principal_id",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?
        .iter()
        .map(|row| {
            Ok(OrgMembership {
                principal_kind: principal_kind(row)?,
                principal_id: required(row, "principal_id")?,
                role: org_role(row)?,
            })
        })
        .collect()
    }

    #[tracing::instrument(skip_all)]
    async fn list_workspace_members(
        &self,
        org_id: &str,
        workspace_id: &str,
    ) -> Result<Vec<WorkspaceMembership>, StorageError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        if !live_workspace_exists(&mut tx, org_id, workspace_id).await? {
            return Err(StorageError::not_found("workspace", workspace_id));
        }
        let rows = sqlx::query(
            "SELECT principal_kind, principal_id, role FROM workspace_memberships \
             WHERE org_id = ?1 AND workspace_id = ?2 ORDER BY principal_kind, principal_id",
        )
        .bind(org_id)
        .bind(workspace_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;
        rows.iter()
            .map(|row| {
                Ok(WorkspaceMembership {
                    role: workspace_role(row)?,
                    principal_kind: principal_kind(row)?,
                    principal_id: required(row, "principal_id")?,
                })
            })
            .collect()
    }

    #[tracing::instrument(skip_all)]
    async fn upsert_org_member_guarded(
        &self,
        request: OrgMemberUpsert,
    ) -> Result<OrgMemberUpsertOutcome, StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        if !live_org_exists(&mut tx, &request.org_id).await? {
            return Err(StorageError::not_found("org", request.org_id));
        }
        let mut privileged_other = false;
        for row in &org_grants(&mut tx, &request.org_id).await? {
            let other = principal_kind(row)? != request.principal_kind
                || required::<String>(row, "principal_id")? != request.principal_id;
            privileged_other |= org_role(row)?.is_privileged() && other;
        }
        if !request.role.is_privileged() && !privileged_other {
            return Ok(OrgMemberUpsertOutcome::WouldLockOut);
        }
        sqlx::query(
            "INSERT INTO org_memberships \
             (org_id, principal_kind, principal_id, role, added_by, added_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT (org_id, principal_kind, principal_id) DO UPDATE \
             SET role = excluded.role, added_by = excluded.added_by, added_at = excluded.added_at",
        )
        .bind(&request.org_id)
        .bind(request.principal_kind.as_str())
        .bind(&request.principal_id)
        .bind(request.role.as_str())
        .bind(&request.added_by)
        .bind(now_micros())
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;
        Ok(OrgMemberUpsertOutcome::Applied)
    }

    #[tracing::instrument(skip_all)]
    async fn remove_org_member_guarded(
        &self,
        org_id: &str,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<OrgMemberRemoveOutcome, StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        if !live_org_exists(&mut tx, org_id).await? {
            return Err(StorageError::not_found("org", org_id));
        }
        let mut found = false;
        let mut privileged_other = false;
        for row in &org_grants(&mut tx, org_id).await? {
            let role = org_role(row)?;
            let target = self::principal_kind(row)? == principal_kind
                && required::<String>(row, "principal_id")? == principal_id;
            found |= target;
            privileged_other |= role.is_privileged() && !target;
        }
        if !found {
            return Ok(OrgMemberRemoveOutcome::NotFound);
        }
        if !privileged_other {
            return Ok(OrgMemberRemoveOutcome::WouldLockOut);
        }
        // Cascades to the principal's workspace grants in this org.
        sqlx::query(
            "DELETE FROM org_memberships \
             WHERE org_id = ?1 AND principal_kind = ?2 AND principal_id = ?3",
        )
        .bind(org_id)
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;
        Ok(OrgMemberRemoveOutcome::Removed)
    }

    #[tracing::instrument(skip_all)]
    async fn upsert_workspace_member(
        &self,
        request: WorkspaceMemberUpsert,
    ) -> Result<(), StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        if !live_workspace_exists(&mut tx, &request.org_id, &request.workspace_id).await? {
            return Err(StorageError::not_found("workspace", request.workspace_id));
        }
        let org_grant = sqlx::query(
            "SELECT role FROM org_memberships \
             WHERE org_id = ?1 AND principal_kind = ?2 AND principal_id = ?3",
        )
        .bind(&request.org_id)
        .bind(request.principal_kind.as_str())
        .bind(&request.principal_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
        let Some(org_grant) = org_grant else {
            return Err(StorageError::not_found(
                "org membership",
                request.principal_id,
            ));
        };
        org_role(&org_grant)?;
        sqlx::query(
            "INSERT INTO workspace_memberships \
             (org_id, workspace_id, principal_kind, principal_id, role, added_by, added_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT (workspace_id, principal_kind, principal_id) DO UPDATE \
             SET role = excluded.role, added_by = excluded.added_by, added_at = excluded.added_at",
        )
        .bind(&request.org_id)
        .bind(&request.workspace_id)
        .bind(request.principal_kind.as_str())
        .bind(&request.principal_id)
        .bind(request.role.as_str())
        .bind(&request.added_by)
        .bind(now_micros())
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)
    }

    #[tracing::instrument(skip_all)]
    async fn remove_workspace_member(
        &self,
        org_id: &str,
        workspace_id: &str,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<bool, StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        if !live_workspace_exists(&mut tx, org_id, workspace_id).await? {
            return Ok(false);
        }
        let result = sqlx::query(
            "DELETE FROM workspace_memberships \
             WHERE org_id = ?1 AND workspace_id = ?2 \
             AND principal_kind = ?3 AND principal_id = ?4",
        )
        .bind(org_id)
        .bind(workspace_id)
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;
        Ok(result.rows_affected() != 0)
    }
}
