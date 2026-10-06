//! The shared tenant directory: orgs, workspaces and memberships behind one
//! lock, plus atomic tenant provisioning over it.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::{
    MembershipRow, OrgMembershipRole, OrgRow, PrincipalKind, ScopeKind, TenantProvisioningConflict,
    TenantProvisioningOutcome, TenantProvisioningRequest, WorkspaceRow,
};
use nebula_storage_port::store::TenantProvisioningStore;
use parking_lot::Mutex;

use super::membership::InMemoryMembershipStore;
use super::now_rfc3339;
use super::org::InMemoryOrgStore;
use super::workspace::InMemoryWorkspaceStore;

/// Workspace key: `(org_id, workspace_id)` so a cross-org `get` misses.
pub(super) type WorkspaceKey = (String, String);

/// Membership key: `(scope_kind, scope_id, principal_kind, principal_id)`,
/// with each kind keyed by its stable text form.
pub(super) type MembershipKey = (String, String, String, String);

pub(super) fn membership_key(
    scope_kind: ScopeKind,
    scope_id: &str,
    principal_kind: PrincipalKind,
    principal_id: &str,
) -> MembershipKey {
    (
        scope_kind.as_str().to_owned(),
        scope_id.to_owned(),
        principal_kind.as_str().to_owned(),
        principal_id.to_owned(),
    )
}

/// Directory rows and grants share a snapshot and mutation critical section.
#[derive(Debug, Default)]
pub(super) struct DirectoryState {
    pub(super) orgs: HashMap<String, OrgRow>,
    pub(super) workspaces: HashMap<WorkspaceKey, WorkspaceRow>,
    pub(super) memberships: HashMap<MembershipKey, MembershipRow>,
}

impl DirectoryState {
    pub(super) fn live_org(&self, org_id: &str) -> bool {
        self.orgs
            .get(org_id)
            .is_some_and(|row| row.deleted_at.is_none())
    }

    /// A live workspace whose id belongs to no other org. Grants are keyed by
    /// workspace id alone, so a second parent — even a deleted one — makes
    /// the identity ambiguous and its grants are never reused.
    pub(super) fn live_unambiguous_workspace(&self, org_id: &str, workspace_id: &str) -> bool {
        self.live_org(org_id)
            && self
                .workspaces
                .get(&(org_id.to_owned(), workspace_id.to_owned()))
                .is_some_and(|row| row.deleted_at.is_none())
            && !self
                .workspaces
                .values()
                .any(|row| row.id == workspace_id && row.org_id != org_id)
    }

    /// Every workspace id of `org_id`, refusing an org that shares a
    /// workspace id with another org.
    pub(super) fn unambiguous_workspace_ids(
        &self,
        org_id: &str,
    ) -> Result<HashSet<String>, StorageError> {
        let ids = self
            .workspaces
            .values()
            .filter(|row| row.org_id == org_id)
            .map(|row| row.id.clone())
            .collect::<HashSet<_>>();
        if self
            .workspaces
            .values()
            .any(|row| row.org_id != org_id && ids.contains(&row.id))
        {
            return Err(StorageError::Corrupt(
                "a workspace id belongs to more than one org".into(),
            ));
        }
        Ok(ids)
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
        let owner_key = membership_key(
            ScopeKind::Org,
            org_request.id(),
            request.owner_principal_kind(),
            request.owner_principal_id(),
        );
        let org = state.orgs.get(org_request.id());
        let workspace = state.workspaces.get(&(
            org_request.id().to_owned(),
            workspace_request.id().to_owned(),
        ));
        let owner = state.memberships.get(&owner_key);

        let active_org_collision = state.orgs.values().any(|row| {
            row.id != org_request.id() && row.deleted_at.is_none() && row.slug == org_request.slug()
        });
        let active_workspace_collision = state.workspaces.values().any(|row| {
            let exact_identity = row.id == workspace_request.id() && row.org_id == org_request.id();
            if exact_identity {
                return false;
            }
            let id_collision = row.id == workspace_request.id();
            let live_sibling = row.org_id == org_request.id() && row.deleted_at.is_none();
            id_collision
                || (live_sibling && (row.slug == workspace_request.slug() || row.is_default))
        });
        if org.is_some_and(|row| org_request.matches_persisted(row))
            && workspace
                .is_some_and(|row| workspace_request.matches_persisted(org_request.id(), row))
            && owner.is_some_and(|row| {
                row.role == OrgMembershipRole::Owner.as_str()
                    && row.added_by.as_deref() == request.owner_added_by()
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

        let created_at = now_rfc3339();
        state.orgs.insert(
            org_request.id().to_owned(),
            org_request.materialize(created_at.clone()),
        );
        state.workspaces.insert(
            (
                org_request.id().to_owned(),
                workspace_request.id().to_owned(),
            ),
            workspace_request.materialize(org_request.id().to_owned(), created_at),
        );
        state.memberships.insert(
            owner_key,
            MembershipRow {
                scope_kind: ScopeKind::Org,
                scope_id: org_request.id().to_owned(),
                principal_kind: request.owner_principal_kind(),
                principal_id: request.owner_principal_id().to_owned(),
                role: OrgMembershipRole::Owner.as_str().to_owned(),
                added_at: now_rfc3339(),
                added_by: request.owner_added_by().map(ToOwned::to_owned),
            },
        );
        Ok(TenantProvisioningOutcome::Created)
    }
}
