//! Memberships: org and workspace grants over the shared directory, so every
//! guarded mutation reads its invariant and writes under one lock.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::{
    OrgMemberRemoveOutcome, OrgMemberUpsert, OrgMemberUpsertOutcome, OrgMembership, PrincipalKind,
    PrincipalOrgMembership, TenantMembershipSnapshot, WorkspaceMemberUpsert, WorkspaceMembership,
};
use nebula_storage_port::store::MembershipStore;

use super::directory::{OrgGrant, SharedDirectory, WorkspaceGrant, grant_key};

/// In-memory `org_memberships` + `workspace_memberships` store — standalone,
/// or a projection of an
/// [`InMemoryIdentityDirectory`](super::InMemoryIdentityDirectory).
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

/// Principal order of the SQL backends' `ORDER BY principal_kind, principal_id`.
fn principal_order(kind: PrincipalKind, id: &str) -> (&'static str, &str) {
    (kind.as_str(), id)
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
            .org_grants
            .get(&grant_key(org_id, principal_kind, principal_id))
            .filter(|_| state.live_org(org_id))
            .map(|grant| grant.role);
        let workspace_role = workspace_id
            .filter(|id| state.live_workspace(org_id, id))
            .and_then(|id| {
                state
                    .workspace_grants
                    .get(&grant_key(id, principal_kind, principal_id))
            })
            .map(|grant| grant.role);
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
        let mut result: Vec<PrincipalOrgMembership> = state
            .org_grants
            .iter()
            .filter(|((org_id, kind, id), _)| {
                *kind == principal_kind && id == principal_id && state.live_org(org_id)
            })
            .map(|((org_id, _, _), grant)| PrincipalOrgMembership {
                org_id: org_id.clone(),
                role: grant.role,
            })
            .collect();
        result.sort_by(|a, b| a.org_id.cmp(&b.org_id));
        Ok(result)
    }

    #[tracing::instrument(skip_all)]
    async fn list_org_members(&self, org_id: &str) -> Result<Vec<OrgMembership>, StorageError> {
        let state = self.inner.lock();
        if !state.live_org(org_id) {
            return Err(StorageError::not_found("org", org_id));
        }
        let mut result: Vec<OrgMembership> = state
            .org_grants
            .iter()
            .filter(|((scope, _, _), _)| scope == org_id)
            .map(|((_, kind, id), grant)| OrgMembership {
                principal_kind: *kind,
                principal_id: id.clone(),
                role: grant.role,
            })
            .collect();
        result.sort_by(|a, b| {
            principal_order(a.principal_kind, &a.principal_id)
                .cmp(&principal_order(b.principal_kind, &b.principal_id))
        });
        Ok(result)
    }

    #[tracing::instrument(skip_all)]
    async fn list_workspace_members(
        &self,
        org_id: &str,
        workspace_id: &str,
    ) -> Result<Vec<WorkspaceMembership>, StorageError> {
        let state = self.inner.lock();
        if !state.live_workspace(org_id, workspace_id) {
            return Err(StorageError::not_found("workspace", workspace_id));
        }
        let mut result: Vec<WorkspaceMembership> = state
            .workspace_grants
            .iter()
            .filter(|((scope, _, _), _)| scope == workspace_id)
            .map(|((_, kind, id), grant)| WorkspaceMembership {
                principal_kind: *kind,
                principal_id: id.clone(),
                role: grant.role,
            })
            .collect();
        result.sort_by(|a, b| {
            principal_order(a.principal_kind, &a.principal_id)
                .cmp(&principal_order(b.principal_kind, &b.principal_id))
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
        let privileged_other = state.org_grants.iter().any(|((org_id, kind, id), grant)| {
            *org_id == request.org_id
                && grant.role.is_privileged()
                && (*kind != request.principal_kind || *id != request.principal_id)
        });
        if !request.role.is_privileged() && !privileged_other {
            return Ok(OrgMemberUpsertOutcome::WouldLockOut);
        }
        state.org_grants.insert(
            grant_key(
                &request.org_id,
                request.principal_kind,
                &request.principal_id,
            ),
            OrgGrant { role: request.role },
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
        let key = grant_key(org_id, principal_kind, principal_id);
        if !state.org_grants.contains_key(&key) {
            return Ok(OrgMemberRemoveOutcome::NotFound);
        }
        let privileged_other = state
            .org_grants
            .iter()
            .any(|(other, grant)| other.0 == org_id && *other != key && grant.role.is_privileged());
        if !privileged_other {
            return Ok(OrgMemberRemoveOutcome::WouldLockOut);
        }
        state.org_grants.remove(&key);
        // The cascade of `fk_workspace_memberships__org_memberships`.
        state.workspace_grants.retain(|(_, kind, id), grant| {
            grant.org_id != org_id || *kind != principal_kind || id != principal_id
        });
        Ok(OrgMemberRemoveOutcome::Removed)
    }

    #[tracing::instrument(skip_all)]
    async fn upsert_workspace_member(
        &self,
        request: WorkspaceMemberUpsert,
    ) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        if !state.live_workspace(&request.org_id, &request.workspace_id) {
            return Err(StorageError::not_found("workspace", request.workspace_id));
        }
        if !state.org_grants.contains_key(&grant_key(
            &request.org_id,
            request.principal_kind,
            &request.principal_id,
        )) {
            return Err(StorageError::not_found(
                "org membership",
                request.principal_id,
            ));
        }
        state.workspace_grants.insert(
            grant_key(
                &request.workspace_id,
                request.principal_kind,
                &request.principal_id,
            ),
            WorkspaceGrant {
                org_id: request.org_id,
                role: request.role,
            },
        );
        Ok(())
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
        if !state.live_workspace(org_id, workspace_id) {
            return Ok(false);
        }
        Ok(state
            .workspace_grants
            .remove(&grant_key(workspace_id, principal_kind, principal_id))
            .is_some())
    }
}
