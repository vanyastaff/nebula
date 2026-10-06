//! `port_memberships`: org and workspace grants. Every guarded mutation runs
//! under `BEGIN IMMEDIATE`, which serializes its invariant read with every
//! writer.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::{
    MembershipRow, OrgMemberRemoveOutcome, OrgMemberUpsert, OrgMemberUpsertOutcome,
    OrgMembershipRole, PrincipalKind, PrincipalOrgMembership, ScopeKind, TenantMembershipSnapshot,
    WorkspaceMemberUpsert, WorkspaceMembership, WorkspaceMembershipRole,
};
use nebula_storage_port::store::MembershipStore;
use sqlx::sqlite::SqliteRow;
use sqlx::{SqliteConnection, SqlitePool};

use super::{now_rfc3339, optional, required};
use crate::sql_error::storage_error;

/// A live workspace whose id belongs to no other org (historical grants lack
/// an org column, so an id with a second parent — even a deleted one — is
/// ambiguous and its grants are never reused).
const LIVE_UNAMBIGUOUS_WORKSPACE: &str = "SELECT id FROM port_workspaces \
     WHERE org_id = ?1 AND id = ?2 AND deleted_at IS NULL \
     AND EXISTS (SELECT 1 FROM port_orgs o WHERE o.id = ?1 AND o.deleted_at IS NULL) \
     AND NOT EXISTS (SELECT 1 FROM port_workspaces other WHERE other.id = ?2 AND other.org_id <> ?1)";

const UPSERT_MEMBERSHIP: &str = "INSERT INTO port_memberships \
     (scope_kind, scope_id, principal_kind, principal_id, role, added_at, added_by) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
     ON CONFLICT (scope_kind, scope_id, principal_kind, principal_id) \
     DO UPDATE SET role = excluded.role, added_at = excluded.added_at, added_by = excluded.added_by";

/// SQLite-backed `org_members` + `workspace_members` store.
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

pub(super) fn decode_membership(row: &SqliteRow) -> Result<MembershipRow, StorageError> {
    Ok(MembershipRow {
        // Fail closed: an unrecognized authz-domain value is corrupt, never
        // coerced to a default.
        scope_kind: ScopeKind::parse(&required::<String>(row, "scope_kind")?)
            .map_err(|_| unknown_value("scope_kind"))?,
        scope_id: required(row, "scope_id")?,
        principal_kind: PrincipalKind::parse(&required::<String>(row, "principal_kind")?)
            .map_err(|_| unknown_value("principal_kind"))?,
        principal_id: required(row, "principal_id")?,
        role: required(row, "role")?,
        added_at: required(row, "added_at")?,
        added_by: optional(row, "added_by")?,
    })
}

fn unknown_value(column: &'static str) -> StorageError {
    StorageError::Corrupt(format!("column `{column}` holds an unknown value"))
}

fn org_role(row: &MembershipRow) -> Result<OrgMembershipRole, StorageError> {
    OrgMembershipRole::parse(&row.role).map_err(|_| unknown_value("role"))
}

fn workspace_role(row: &MembershipRow) -> Result<WorkspaceMembershipRole, StorageError> {
    WorkspaceMembershipRole::parse(&row.role).map_err(|_| unknown_value("role"))
}

async fn org_grants(
    connection: &mut SqliteConnection,
    org_id: &str,
) -> Result<Vec<MembershipRow>, StorageError> {
    sqlx::query("SELECT * FROM port_memberships WHERE scope_kind = 'org' AND scope_id = ?1")
        .bind(org_id)
        .fetch_all(connection)
        .await
        .map_err(storage_error)?
        .iter()
        .map(decode_membership)
        .collect()
}

async fn live_org_exists(
    connection: &mut SqliteConnection,
    org_id: &str,
) -> Result<bool, StorageError> {
    Ok(
        sqlx::query("SELECT id FROM port_orgs WHERE id = ?1 AND deleted_at IS NULL")
            .bind(org_id)
            .fetch_optional(connection)
            .await
            .map_err(storage_error)?
            .is_some(),
    )
}

async fn live_unambiguous_workspace(
    connection: &mut SqliteConnection,
    org_id: &str,
    workspace_id: &str,
) -> Result<bool, StorageError> {
    Ok(sqlx::query(LIVE_UNAMBIGUOUS_WORKSPACE)
        .bind(org_id)
        .bind(workspace_id)
        .fetch_optional(connection)
        .await
        .map_err(storage_error)?
        .is_some())
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
        let rows = sqlx::query(
            "SELECT m.* FROM port_memberships m \
             WHERE m.principal_kind = ?1 AND m.principal_id = ?2 \
             AND ((m.scope_kind = 'org' AND m.scope_id = ?3) \
             OR (m.scope_kind = 'workspace' AND m.scope_id = ?4 \
             AND EXISTS (SELECT 1 FROM port_orgs o WHERE o.id = ?3 AND o.deleted_at IS NULL) \
             AND EXISTS (SELECT 1 FROM port_workspaces w \
             WHERE w.org_id = ?3 AND w.id = ?4 AND w.deleted_at IS NULL \
             AND NOT EXISTS (SELECT 1 FROM port_workspaces other \
             WHERE other.id = w.id AND other.org_id <> w.org_id))))",
        )
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .bind(org_id)
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?;
        let mut snapshot = TenantMembershipSnapshot::default();
        for raw in &rows {
            let row = decode_membership(raw)?;
            match row.scope_kind {
                ScopeKind::Org => snapshot.org_role = Some(org_role(&row)?),
                ScopeKind::Workspace => snapshot.workspace_role = Some(workspace_role(&row)?),
            }
        }
        Ok(snapshot)
    }

    #[tracing::instrument(skip_all)]
    async fn list_orgs_for_principal(
        &self,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<Vec<PrincipalOrgMembership>, StorageError> {
        sqlx::query(
            "SELECT * FROM port_memberships \
             WHERE scope_kind = 'org' AND principal_kind = ?1 AND principal_id = ?2 \
             ORDER BY scope_id",
        )
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?
        .iter()
        .map(|raw| {
            let row = decode_membership(raw)?;
            Ok(PrincipalOrgMembership {
                role: org_role(&row)?,
                org_id: row.scope_id,
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
        if !live_unambiguous_workspace(&mut tx, org_id, workspace_id).await? {
            return Err(StorageError::not_found("workspace", workspace_id));
        }
        let rows = sqlx::query(
            "SELECT * FROM port_memberships WHERE scope_kind = 'workspace' AND scope_id = ?1 \
             ORDER BY principal_kind, principal_id",
        )
        .bind(workspace_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;
        rows.iter()
            .map(|raw| {
                let row = decode_membership(raw)?;
                Ok(WorkspaceMembership {
                    role: workspace_role(&row)?,
                    principal_kind: row.principal_kind,
                    principal_id: row.principal_id,
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
        for row in org_grants(&mut tx, &request.org_id).await? {
            privileged_other |= org_role(&row)?.is_privileged()
                && (row.principal_kind != request.principal_kind
                    || row.principal_id != request.principal_id);
        }
        if !request.role.is_privileged() && !privileged_other {
            return Ok(OrgMemberUpsertOutcome::WouldLockOut);
        }
        sqlx::query(UPSERT_MEMBERSHIP)
            .bind(ScopeKind::Org.as_str())
            .bind(&request.org_id)
            .bind(request.principal_kind.as_str())
            .bind(&request.principal_id)
            .bind(request.role.as_str())
            .bind(now_rfc3339())
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
        for row in org_grants(&mut tx, org_id).await? {
            let role = org_role(&row)?;
            let target = row.principal_kind == principal_kind && row.principal_id == principal_id;
            found |= target;
            privileged_other |= role.is_privileged() && !target;
        }
        if !found {
            return Ok(OrgMemberRemoveOutcome::NotFound);
        }
        if !privileged_other {
            return Ok(OrgMemberRemoveOutcome::WouldLockOut);
        }
        let ambiguous_workspace = sqlx::query(
            "SELECT 1 FROM port_workspaces own JOIN port_workspaces other \
             ON own.id = other.id AND own.org_id <> other.org_id \
             WHERE own.org_id = ?1 LIMIT 1",
        )
        .bind(org_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
        if ambiguous_workspace.is_some() {
            return Err(StorageError::Corrupt(
                "a workspace id belongs to more than one org".into(),
            ));
        }
        sqlx::query(
            "DELETE FROM port_memberships \
             WHERE scope_kind = 'org' AND scope_id = ?1 AND principal_kind = ?2 AND principal_id = ?3",
        )
        .bind(org_id)
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        sqlx::query(
            "DELETE FROM port_memberships \
             WHERE scope_kind = 'workspace' AND principal_kind = ?1 AND principal_id = ?2 \
             AND scope_id IN (SELECT id FROM port_workspaces WHERE org_id = ?3)",
        )
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .bind(org_id)
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
        if !live_unambiguous_workspace(&mut tx, &request.org_id, &request.workspace_id).await? {
            return Err(StorageError::not_found("workspace", request.workspace_id));
        }
        let org_grant = sqlx::query(
            "SELECT * FROM port_memberships \
             WHERE scope_kind = 'org' AND scope_id = ?1 AND principal_kind = ?2 AND principal_id = ?3",
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
        org_role(&decode_membership(&org_grant)?)?;
        sqlx::query(UPSERT_MEMBERSHIP)
            .bind(ScopeKind::Workspace.as_str())
            .bind(&request.workspace_id)
            .bind(request.principal_kind.as_str())
            .bind(&request.principal_id)
            .bind(request.role.as_str())
            .bind(now_rfc3339())
            .bind(&request.added_by)
            .execute(&mut *tx)
            .await
            .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)
    }

    async fn get(
        &self,
        scope_kind: ScopeKind,
        scope_id: &str,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<Option<MembershipRow>, StorageError> {
        sqlx::query(
            "SELECT * FROM port_memberships WHERE scope_kind = ? AND scope_id = ? \
             AND principal_kind = ? AND principal_id = ?",
        )
        .bind(scope_kind.as_str())
        .bind(scope_id)
        .bind(principal_kind.as_str())
        .bind(principal_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .as_ref()
        .map(decode_membership)
        .transpose()
    }

    async fn list_for_scope(
        &self,
        scope_kind: ScopeKind,
        scope_id: &str,
    ) -> Result<Vec<MembershipRow>, StorageError> {
        sqlx::query(
            "SELECT * FROM port_memberships \
             WHERE scope_kind = ? AND scope_id = ? ORDER BY principal_id",
        )
        .bind(scope_kind.as_str())
        .bind(scope_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?
        .iter()
        .map(decode_membership)
        .collect()
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
        if !live_unambiguous_workspace(&mut tx, org_id, workspace_id).await? {
            return Ok(false);
        }
        let result = sqlx::query(
            "DELETE FROM port_memberships \
             WHERE scope_kind = 'workspace' AND scope_id = ?1 \
             AND principal_kind = ?2 AND principal_id = ?3",
        )
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
