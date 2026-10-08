//! `org_memberships` and `workspace_memberships`: explicit grants.
//!
//! Guarded organization mutations lock the live org row before reading the
//! owner/administrator invariant, so a READ COMMITTED snapshot includes every
//! preceding lock holder's commit. Workspace-grant writes lock the org row
//! and the live workspace row, so they serialize with guarded removals and
//! with workspace soft deletes. Removing an organization grant cascades to
//! the principal's workspace grants through the foreign key.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::{
    OrgMemberRemoveOutcome, OrgMemberUpsert, OrgMemberUpsertOutcome, OrgMembership,
    OrgMembershipRole, PrincipalKind, PrincipalOrgMembership, TenantMembershipSnapshot,
    WorkspaceMemberUpsert, WorkspaceMembership, WorkspaceMembershipRole,
};
use nebula_storage_port::store::MembershipStore;
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool};

use super::required;
use crate::sql_error::storage_error;

/// Postgres-backed `org_memberships` + `workspace_memberships` store.
#[derive(Clone, Debug)]
pub struct PgMembershipStore {
    pool: PgPool,
}

impl PgMembershipStore {
    /// Wrap a pool whose schema was installed via [`crate::postgres::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn unknown_value(column: &'static str) -> StorageError {
    StorageError::Corrupt(format!("column `{column}` holds an unknown value"))
}

// Fail closed: an unrecognized authz-domain value is corrupt, never coerced
// to a default.
fn principal_kind(row: &PgRow) -> Result<PrincipalKind, StorageError> {
    PrincipalKind::parse(&required::<String>(row, "principal_kind")?)
        .map_err(|_| unknown_value("principal_kind"))
}

pub(super) fn org_role(row: &PgRow) -> Result<OrgMembershipRole, StorageError> {
    OrgMembershipRole::parse(&required::<String>(row, "role")?).map_err(|_| unknown_value("role"))
}

fn workspace_role(row: &PgRow) -> Result<WorkspaceMembershipRole, StorageError> {
    WorkspaceMembershipRole::parse(&required::<String>(row, "role")?)
        .map_err(|_| unknown_value("role"))
}

/// Lock the live org row for the organization critical section; `false` when
/// it does not exist or is deleted.
///
/// `FOR NO KEY UPDATE` serializes every membership writer of the org with each
/// other and with org updates and soft deletes, without blocking the
/// `FOR KEY SHARE` locks foreign-key checks take on `orgs`.
async fn lock_live_org(connection: &mut PgConnection, org_id: &str) -> Result<bool, StorageError> {
    Ok(
        sqlx::query("SELECT id FROM orgs WHERE id = $1 AND deleted_at IS NULL FOR NO KEY UPDATE")
            .bind(org_id)
            .fetch_optional(connection)
            .await
            .map_err(storage_error)?
            .is_some(),
    )
}

/// Share-lock the live workspace under `org_id` against a concurrent soft
/// delete; `false` when it does not exist, is deleted or has another parent.
async fn lock_live_workspace(
    connection: &mut PgConnection,
    org_id: &str,
    workspace_id: &str,
) -> Result<bool, StorageError> {
    Ok(sqlx::query(
        "SELECT id FROM workspaces \
         WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL FOR SHARE",
    )
    .bind(org_id)
    .bind(workspace_id)
    .fetch_optional(connection)
    .await
    .map_err(storage_error)?
    .is_some())
}

#[async_trait::async_trait]
impl MembershipStore for PgMembershipStore {
    #[tracing::instrument(skip_all)]
    async fn get_tenant_membership(
        &self,
        org_id: &str,
        workspace_id: Option<&str>,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<TenantMembershipSnapshot, StorageError> {
        // One statement, one snapshot: the org grant in a live org and the
        // workspace grant of a live workspace in a live org.
        let row = sqlx::query(
            "SELECT \
             (SELECT m.role FROM org_memberships m \
              JOIN orgs o ON o.id = m.org_id AND o.deleted_at IS NULL \
              WHERE m.org_id = $1 AND m.principal_kind = $2 AND m.principal_id = $3) \
              AS org_role, \
             (SELECT m.role FROM workspace_memberships m \
              JOIN workspaces w ON w.org_id = m.org_id AND w.id = m.workspace_id \
              JOIN orgs o ON o.id = w.org_id \
              WHERE m.org_id = $1 AND m.workspace_id = $4 \
              AND m.principal_kind = $2 AND m.principal_id = $3 \
              AND w.deleted_at IS NULL AND o.deleted_at IS NULL) AS workspace_role",
        )
        .bind(org_id)
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .bind(workspace_id)
        .fetch_one(&self.pool)
        .await
        .map_err(storage_error)?;
        let org_role = super::optional::<String>(&row, "org_role")?
            .map(|role| OrgMembershipRole::parse(&role).map_err(|_| unknown_value("role")))
            .transpose()?;
        let workspace_role = super::optional::<String>(&row, "workspace_role")?
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
            "SELECT m.org_id, m.role FROM org_memberships m \
             JOIN orgs o ON o.id = m.org_id AND o.deleted_at IS NULL \
             WHERE m.principal_kind = $1 AND m.principal_id = $2 \
             ORDER BY m.org_id COLLATE \"C\"",
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
        // One statement, one snapshot: no row ⇒ the org is missing or
        // deleted; one row with a NULL principal ⇒ a live org without grants.
        let rows = sqlx::query(
            "SELECT m.principal_kind, m.principal_id, m.role FROM orgs o \
             LEFT JOIN org_memberships m ON m.org_id = o.id \
             WHERE o.id = $1 AND o.deleted_at IS NULL \
             ORDER BY m.principal_kind COLLATE \"C\", m.principal_id COLLATE \"C\"",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?;
        if rows.is_empty() {
            return Err(StorageError::not_found("org", org_id));
        }
        let mut members = Vec::with_capacity(rows.len());
        for row in &rows {
            if super::optional::<String>(row, "principal_id")?.is_none() {
                continue;
            }
            members.push(OrgMembership {
                principal_kind: principal_kind(row)?,
                principal_id: required(row, "principal_id")?,
                role: org_role(row)?,
            });
        }
        Ok(members)
    }

    #[tracing::instrument(skip_all)]
    async fn list_workspace_members(
        &self,
        org_id: &str,
        workspace_id: &str,
    ) -> Result<Vec<WorkspaceMembership>, StorageError> {
        // One statement, one snapshot: no row ⇒ the workspace is missing,
        // deleted, under a deleted org or under another org.
        let rows = sqlx::query(
            "SELECT m.principal_kind, m.principal_id, m.role \
             FROM workspaces w JOIN orgs o ON o.id = w.org_id \
             LEFT JOIN workspace_memberships m \
               ON m.org_id = w.org_id AND m.workspace_id = w.id \
             WHERE w.org_id = $1 AND w.id = $2 \
               AND w.deleted_at IS NULL AND o.deleted_at IS NULL \
             ORDER BY m.principal_kind COLLATE \"C\", m.principal_id COLLATE \"C\"",
        )
        .bind(org_id)
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?;
        if rows.is_empty() {
            return Err(StorageError::not_found("workspace", workspace_id));
        }
        let mut members = Vec::with_capacity(rows.len());
        for row in &rows {
            if super::optional::<String>(row, "principal_id")?.is_none() {
                continue;
            }
            members.push(WorkspaceMembership {
                role: workspace_role(row)?,
                principal_kind: principal_kind(row)?,
                principal_id: required(row, "principal_id")?,
            });
        }
        Ok(members)
    }

    #[tracing::instrument(skip_all)]
    async fn upsert_org_member_guarded(
        &self,
        request: OrgMemberUpsert,
    ) -> Result<OrgMemberUpsertOutcome, StorageError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        if !lock_live_org(&mut tx, &request.org_id).await? {
            return Err(StorageError::not_found("org", request.org_id));
        }
        let grants = sqlx::query(
            "SELECT principal_kind, principal_id, role FROM org_memberships WHERE org_id = $1",
        )
        .bind(&request.org_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage_error)?;
        let mut privileged_other = false;
        for row in &grants {
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
             VALUES ($1, $2, $3, $4, $5, now()) \
             ON CONFLICT (org_id, principal_kind, principal_id) DO UPDATE \
             SET role = excluded.role, added_by = excluded.added_by, added_at = excluded.added_at",
        )
        .bind(&request.org_id)
        .bind(request.principal_kind.as_str())
        .bind(&request.principal_id)
        .bind(request.role.as_str())
        .bind(&request.added_by)
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
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        if !lock_live_org(&mut tx, org_id).await? {
            return Err(StorageError::not_found("org", org_id));
        }
        let grants = sqlx::query(
            "SELECT principal_kind, principal_id, role FROM org_memberships WHERE org_id = $1",
        )
        .bind(org_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage_error)?;
        let mut found = false;
        let mut privileged_other = false;
        for row in &grants {
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
             WHERE org_id = $1 AND principal_kind = $2 AND principal_id = $3",
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
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        if !lock_live_org(&mut tx, &request.org_id).await?
            || !lock_live_workspace(&mut tx, &request.org_id, &request.workspace_id).await?
        {
            return Err(StorageError::not_found("workspace", request.workspace_id));
        }
        let org_grant = sqlx::query(
            "SELECT role FROM org_memberships \
             WHERE org_id = $1 AND principal_kind = $2 AND principal_id = $3",
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
             VALUES ($1, $2, $3, $4, $5, $6, now()) \
             ON CONFLICT (workspace_id, principal_kind, principal_id) DO UPDATE \
             SET role = excluded.role, added_by = excluded.added_by, added_at = excluded.added_at",
        )
        .bind(&request.org_id)
        .bind(&request.workspace_id)
        .bind(request.principal_kind.as_str())
        .bind(&request.principal_id)
        .bind(request.role.as_str())
        .bind(&request.added_by)
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
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        if !lock_live_org(&mut tx, org_id).await?
            || !lock_live_workspace(&mut tx, org_id, workspace_id).await?
        {
            return Ok(false);
        }
        let result = sqlx::query(
            "DELETE FROM workspace_memberships \
             WHERE org_id = $1 AND workspace_id = $2 \
             AND principal_kind = $3 AND principal_id = $4",
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
