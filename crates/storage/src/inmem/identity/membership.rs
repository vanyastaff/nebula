//! Memberships: org and workspace grants over the shared directory, so every
//! guarded mutation reads its invariant and writes under one lock.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::{
    MembershipRow, OrgMemberRemoveOutcome, OrgMemberUpsert, OrgMemberUpsertOutcome,
    OrgMembershipRole, PrincipalKind, PrincipalOrgMembership, ScopeKind, TenantMembershipSnapshot,
    WorkspaceMemberUpsert, WorkspaceMembership, WorkspaceMembershipRole,
};
use nebula_storage_port::store::MembershipStore;

use super::directory::{DirectoryState, SharedDirectory, membership_key};
use super::now_rfc3339;

/// In-memory `org_members` + `workspace_members` store — standalone, or a
/// projection of an [`InMemoryIdentityDirectory`](super::InMemoryIdentityDirectory).
#[derive(Debug, Default, Clone)]
pub struct InMemoryMembershipStore {
    inner: SharedDirectory,
}

impl InMemoryMembershipStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn over(inner: SharedDirectory) -> Self {
        Self { inner }
    }
}

fn invalid_role() -> StorageError {
    StorageError::Corrupt("column `role` holds an unknown value".into())
}

fn org_role(row: &MembershipRow) -> Result<OrgMembershipRole, StorageError> {
    OrgMembershipRole::parse(&row.role).map_err(|_| invalid_role())
}

fn workspace_role(row: &MembershipRow) -> Result<WorkspaceMembershipRole, StorageError> {
    WorkspaceMembershipRole::parse(&row.role).map_err(|_| invalid_role())
}

fn is_org_grant(row: &MembershipRow, org_id: &str) -> bool {
    row.scope_kind == ScopeKind::Org && row.scope_id == org_id
}

fn insert_grant(state: &mut DirectoryState, row: MembershipRow) {
    let key = membership_key(
        row.scope_kind,
        &row.scope_id,
        row.principal_kind,
        &row.principal_id,
    );
    state.memberships.insert(key, row);
}

#[async_trait::async_trait]
impl MembershipStore for InMemoryMembershipStore {
    #[tracing::instrument(skip_all)]
    async fn get_tenant_membership(
        &self,
        org_id: &str,
        workspace_id: Option<&str>,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<TenantMembershipSnapshot, StorageError> {
        let state = self.inner.lock();
        let org_role = state
            .memberships
            .get(&membership_key(
                ScopeKind::Org,
                org_id,
                principal_kind,
                principal_id,
            ))
            .map(org_role)
            .transpose()?;
        let workspace_role = workspace_id
            .filter(|id| state.live_unambiguous_workspace(org_id, id))
            .and_then(|id| {
                state.memberships.get(&membership_key(
                    ScopeKind::Workspace,
                    id,
                    principal_kind,
                    principal_id,
                ))
            })
            .map(workspace_role)
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
        let state = self.inner.lock();
        let mut result = state
            .memberships
            .values()
            .filter(|row| {
                row.scope_kind == ScopeKind::Org
                    && row.principal_kind == principal_kind
                    && row.principal_id == principal_id
            })
            .map(|row| {
                Ok(PrincipalOrgMembership {
                    org_id: row.scope_id.clone(),
                    role: org_role(row)?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        result.sort_by(|a, b| a.org_id.cmp(&b.org_id));
        Ok(result)
    }

    #[tracing::instrument(skip_all)]
    async fn list_workspace_members(
        &self,
        org_id: &str,
        workspace_id: &str,
    ) -> Result<Vec<WorkspaceMembership>, StorageError> {
        let state = self.inner.lock();
        if !state.live_unambiguous_workspace(org_id, workspace_id) {
            return Err(StorageError::not_found("workspace", workspace_id));
        }
        let mut result = state
            .memberships
            .values()
            .filter(|row| row.scope_kind == ScopeKind::Workspace && row.scope_id == workspace_id)
            .map(|row| {
                Ok(WorkspaceMembership {
                    principal_kind: row.principal_kind,
                    principal_id: row.principal_id.clone(),
                    role: workspace_role(row)?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        result.sort_by(|a, b| {
            (a.principal_kind.as_str(), a.principal_id.as_str())
                .cmp(&(b.principal_kind.as_str(), b.principal_id.as_str()))
        });
        Ok(result)
    }

    #[tracing::instrument(skip_all)]
    async fn upsert_org_member_guarded(
        &self,
        request: OrgMemberUpsert,
    ) -> Result<OrgMemberUpsertOutcome, StorageError> {
        let mut state = self.inner.lock();
        if !state.live_org(&request.org_id) {
            return Err(StorageError::not_found("org", request.org_id));
        }
        let mut privileged_other = false;
        for row in state
            .memberships
            .values()
            .filter(|row| is_org_grant(row, &request.org_id))
        {
            privileged_other |= org_role(row)?.is_privileged()
                && (row.principal_kind != request.principal_kind
                    || row.principal_id != request.principal_id);
        }
        if !request.role.is_privileged() && !privileged_other {
            return Ok(OrgMemberUpsertOutcome::WouldLockOut);
        }
        insert_grant(
            &mut state,
            MembershipRow {
                scope_kind: ScopeKind::Org,
                scope_id: request.org_id,
                principal_kind: request.principal_kind,
                principal_id: request.principal_id,
                role: request.role.as_str().into(),
                added_at: now_rfc3339(),
                added_by: request.added_by,
            },
        );
        Ok(OrgMemberUpsertOutcome::Applied)
    }

    #[tracing::instrument(skip_all)]
    async fn remove_org_member_guarded(
        &self,
        org_id: &str,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<OrgMemberRemoveOutcome, StorageError> {
        let mut state = self.inner.lock();
        if !state.live_org(org_id) {
            return Err(StorageError::not_found("org", org_id));
        }
        let key = membership_key(ScopeKind::Org, org_id, principal_kind, principal_id);
        let mut privileged_other = false;
        for row in state
            .memberships
            .values()
            .filter(|row| is_org_grant(row, org_id))
        {
            privileged_other |= org_role(row)?.is_privileged()
                && (row.principal_kind != principal_kind || row.principal_id != principal_id);
        }
        if !state.memberships.contains_key(&key) {
            return Ok(OrgMemberRemoveOutcome::NotFound);
        }
        if !privileged_other {
            return Ok(OrgMemberRemoveOutcome::WouldLockOut);
        }
        let workspace_ids = state.unambiguous_workspace_ids(org_id)?;
        state.memberships.remove(&key);
        state.memberships.retain(|_, row| {
            row.scope_kind != ScopeKind::Workspace
                || row.principal_kind != principal_kind
                || row.principal_id != principal_id
                || !workspace_ids.contains(&row.scope_id)
        });
        Ok(OrgMemberRemoveOutcome::Removed)
    }

    #[tracing::instrument(skip_all)]
    async fn upsert_workspace_member(
        &self,
        request: WorkspaceMemberUpsert,
    ) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        if !state.live_unambiguous_workspace(&request.org_id, &request.workspace_id) {
            return Err(StorageError::not_found("workspace", request.workspace_id));
        }
        let Some(org_grant) = state.memberships.get(&membership_key(
            ScopeKind::Org,
            &request.org_id,
            request.principal_kind,
            &request.principal_id,
        )) else {
            return Err(StorageError::not_found(
                "org membership",
                request.principal_id,
            ));
        };
        org_role(org_grant)?;
        insert_grant(
            &mut state,
            MembershipRow {
                scope_kind: ScopeKind::Workspace,
                scope_id: request.workspace_id,
                principal_kind: request.principal_kind,
                principal_id: request.principal_id,
                role: request.role.as_str().into(),
                added_at: now_rfc3339(),
                added_by: request.added_by,
            },
        );
        Ok(())
    }

    async fn get(
        &self,
        scope_kind: ScopeKind,
        scope_id: &str,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<Option<MembershipRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .memberships
            .get(&membership_key(
                scope_kind,
                scope_id,
                principal_kind,
                principal_id,
            ))
            .cloned())
    }

    async fn list_for_scope(
        &self,
        scope_kind: ScopeKind,
        scope_id: &str,
    ) -> Result<Vec<MembershipRow>, StorageError> {
        let mut rows: Vec<MembershipRow> = self
            .inner
            .lock()
            .memberships
            .values()
            .filter(|row| row.scope_kind == scope_kind && row.scope_id == scope_id)
            .cloned()
            .collect();
        // Same order as the SQL backends' `ORDER BY principal_id`.
        rows.sort_by(|a, b| a.principal_id.cmp(&b.principal_id));
        Ok(rows)
    }

    #[tracing::instrument(skip_all)]
    async fn remove_workspace_member(
        &self,
        org_id: &str,
        workspace_id: &str,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<bool, StorageError> {
        let mut state = self.inner.lock();
        if !state.live_unambiguous_workspace(org_id, workspace_id) {
            return Ok(false);
        }
        Ok(state
            .memberships
            .remove(&membership_key(
                ScopeKind::Workspace,
                workspace_id,
                principal_kind,
                principal_id,
            ))
            .is_some())
    }
}
