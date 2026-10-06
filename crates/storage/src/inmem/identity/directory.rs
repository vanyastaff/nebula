//! The shared tenant directory: orgs, workspaces and grants behind one lock,
//! plus atomic tenant provisioning over it.
//!
//! The maps mirror the relational schema: workspace ids are unique across
//! organizations, an organization grant is keyed by `(org, principal)`, and
//! a workspace grant by `(workspace, principal)` beneath its organization
//! grant — removing the organization grant removes the workspace grants.

use std::collections::HashMap;
use std::sync::Arc;

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::{
    OrgMembershipRole, OrgRow, PrincipalKind, TenantProvisioningConflict,
    TenantProvisioningOutcome, TenantProvisioningRequest, WorkspaceMembershipRole, WorkspaceRow,
};
use nebula_storage_port::store::TenantProvisioningStore;
use parking_lot::Mutex;

use super::membership::InMemoryMembershipStore;
use super::org::InMemoryOrgStore;
use super::workspace::InMemoryWorkspaceStore;

/// Grant key: `(scope_id, principal_kind, principal_id)`, where the scope is
/// the organization or the workspace.
pub(super) type GrantKey = (String, PrincipalKind, String);

pub(super) fn grant_key(
    scope_id: &str,
    principal_kind: PrincipalKind,
    principal_id: &str,
) -> GrantKey {
    (scope_id.to_owned(), principal_kind, principal_id.to_owned())
}

/// An organization grant.
#[derive(Debug, Clone)]
pub(super) struct OrgGrant {
    pub(super) role: OrgMembershipRole,
    pub(super) added_by: Option<String>,
}

/// A workspace grant; `org_id` is the workspace's organization.
#[derive(Debug, Clone)]
pub(super) struct WorkspaceGrant {
    pub(super) org_id: String,
    pub(super) role: WorkspaceMembershipRole,
}

/// Directory rows and grants share a snapshot and mutation critical section.
#[derive(Debug, Default)]
pub(super) struct DirectoryState {
    pub(super) orgs: HashMap<String, OrgRow>,
    /// Keyed by workspace id, unique across organizations.
    pub(super) workspaces: HashMap<String, WorkspaceRow>,
    pub(super) org_grants: HashMap<GrantKey, OrgGrant>,
    pub(super) workspace_grants: HashMap<GrantKey, WorkspaceGrant>,
}

impl DirectoryState {
    pub(super) fn live_org(&self, org_id: &str) -> bool {
        self.orgs
            .get(org_id)
            .is_some_and(|row| row.deleted_at.is_none())
    }

    /// A live workspace under `org_id` in a live organization.
    pub(super) fn live_workspace(&self, org_id: &str, workspace_id: &str) -> bool {
        self.live_org(org_id)
            && self
                .workspaces
                .get(workspace_id)
                .is_some_and(|row| row.org_id == org_id && row.deleted_at.is_none())
    }
}

pub(super) type SharedDirectory = Arc<Mutex<DirectoryState>>;

/// Composition root for in-memory tenant directory stores.
///
/// The projections share one lock so parent checks, membership reads, and
/// guarded mutations observe one logical snapshot.
#[derive(Debug, Default, Clone)]
pub struct InMemoryIdentityDirectory {
    inner: SharedDirectory,
}

impl InMemoryIdentityDirectory {
    /// Create an empty tenant directory.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Organization projection over the shared directory state.
    #[must_use]
    pub fn org_store(&self) -> InMemoryOrgStore {
        InMemoryOrgStore::over(Arc::clone(&self.inner))
    }

    /// Workspace projection over the shared directory state.
    #[must_use]
    pub fn workspace_store(&self) -> InMemoryWorkspaceStore {
        InMemoryWorkspaceStore::over(Arc::clone(&self.inner))
    }

    /// Membership projection over the shared directory state.
    #[must_use]
    pub fn membership_store(&self) -> InMemoryMembershipStore {
        InMemoryMembershipStore::over(Arc::clone(&self.inner))
    }

    /// Atomic tenant-provisioning view over the shared directory state.
    #[must_use]
    pub fn provisioning_store(&self) -> Self {
        self.clone()
    }
}

#[async_trait::async_trait]
impl TenantProvisioningStore for InMemoryIdentityDirectory {
    #[tracing::instrument(skip_all)]
    async fn provision_tenant(
        &self,
        request: TenantProvisioningRequest,
    ) -> Result<TenantProvisioningOutcome, StorageError> {
        let mut state = self.inner.lock();
        let org_request = request.org();
        let workspace_request = request.default_workspace();
        let owner_key = grant_key(
            org_request.id(),
            request.owner_principal_kind(),
            request.owner_principal_id(),
        );
        let org = state.orgs.get(org_request.id());
        let workspace = state.workspaces.get(workspace_request.id());
        let owner = state.org_grants.get(&owner_key);

        let active_org_collision = state.orgs.values().any(|row| {
            row.id != org_request.id() && row.deleted_at.is_none() && row.slug == org_request.slug()
        });
        let active_workspace_collision = state.workspaces.values().any(|row| {
            row.id != workspace_request.id()
                && row.org_id == org_request.id()
                && row.deleted_at.is_none()
                && (row.slug == workspace_request.slug() || row.is_default)
        });
        if org.is_some_and(|row| org_request.matches_persisted(row))
            && workspace
                .is_some_and(|row| workspace_request.matches_persisted(org_request.id(), row))
            && owner.is_some_and(|grant| {
                grant.role == OrgMembershipRole::Owner
                    && grant.added_by.as_deref() == request.owner_added_by()
            })
            && !active_org_collision
            && !active_workspace_collision
        {
            return Ok(TenantProvisioningOutcome::Replayed);
        }

        if org.is_some()
            || workspace.is_some()
            || owner.is_some()
            || active_org_collision
            || active_workspace_collision
        {
            return Ok(TenantProvisioningOutcome::Conflict(
                TenantProvisioningConflict::ExistingState,
            ));
        }

        let created_at = chrono::Utc::now();
        state.orgs.insert(
            org_request.id().to_owned(),
            org_request.materialize(created_at),
        );
        state.workspaces.insert(
            workspace_request.id().to_owned(),
            workspace_request.materialize(org_request.id().to_owned(), created_at),
        );
        state.org_grants.insert(
            owner_key,
            OrgGrant {
                role: OrgMembershipRole::Owner,
                added_by: request.owner_added_by().map(ToOwned::to_owned),
            },
        );
        Ok(TenantProvisioningOutcome::Created)
    }
}
