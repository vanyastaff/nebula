//! In-memory identity-zoo stores.
//!
//! Directory views share one `parking_lot::Mutex` for membership snapshots;
//! other aggregates own independent maps. Tenant-scoped
//! lookups fold the parent id (org / workspace) or `Scope` into the map
//! key, so a cross-tenant `get` returns `Ok(None)` exactly as the SQL
//! backends' `WHERE … = ?` predicate would — an id outside the caller's
//! scope is indistinguishable from one that does not exist (no existence
//! oracle, spec §6.1).
//!
//! Soft-delete is modelled by stamping `deleted_at`: a soft-deleted row
//! stays in the map but is filtered out of every read path, mirroring the
//! SQL `WHERE deleted_at IS NULL` predicate. First-writer-wins uniqueness
//! (email / slug among *active* rows) and optimistic CAS (`version`) match
//! the relational contract the conformance matrix asserts.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nebula_storage_port::dto::{
    AuditLogRow, BlobRow, MembershipRow, OrgRow, PrincipalKind, QuotaRow, ResourceRow, ScopeKind,
    TriggerRow, UserRow, WorkspaceRow,
};
use nebula_storage_port::dto::{
    OrgMemberRemoveOutcome, OrgMemberUpsert, OrgMemberUpsertOutcome, OrgMembershipRole,
    PrincipalOrgMembership, TenantMembershipSnapshot, TenantProvisioningConflict,
    TenantProvisioningOutcome, TenantProvisioningRequest, WorkspaceMemberUpsert,
    WorkspaceMembershipRole,
};
use nebula_storage_port::store::{
    AuditStore, BlobStore, MembershipStore, OrgStore, QuotaStore, ResourceStore,
    TenantProvisioningStore, TriggerStore, UserStore, WorkspaceStore,
};
use nebula_storage_port::{Scope, StorageError};
use parking_lot::Mutex;

// ── Users ─────────────────────────────────────────────────────────────────

/// In-memory `users` store. Users are global (no tenant scope); email is
/// unique among active rows (case-insensitive).
#[derive(Debug, Default, Clone)]
pub struct InMemoryUserStore {
    inner: Arc<Mutex<InMemoryUserState>>,
}

#[derive(Debug, Default)]
struct InMemoryUserState {
    rows: HashMap<String, Arc<UserRow>>,
    deleted_ids: HashSet<String>,
}

impl InMemoryUserStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl UserStore for InMemoryUserStore {
    async fn create(&self, row: UserRow) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        if state.rows.contains_key(&row.id) {
            return Err(StorageError::Duplicate {
                entity: "user",
                detail: format!("user {} already exists", row.id),
            });
        }
        let email = row.email.to_ascii_lowercase();
        if state.rows.iter().any(|(id, user)| {
            !state.deleted_ids.contains(id)
                && user.deleted_at.is_none()
                && user.email.to_ascii_lowercase() == email
        }) {
            return Err(StorageError::Duplicate {
                entity: "user",
                detail: format!("active user with email {} already exists", row.email),
            });
        }
        state.rows.insert(row.id.clone(), Arc::new(row));
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<Arc<UserRow>>, StorageError> {
        let state = self.inner.lock();
        if state.deleted_ids.contains(id) {
            return Ok(None);
        }
        Ok(state
            .rows
            .get(id)
            .filter(|user| user.deleted_at.is_none())
            .cloned())
    }

    async fn get_by_email(&self, email: &str) -> Result<Option<Arc<UserRow>>, StorageError> {
        let needle = email.to_ascii_lowercase();
        let state = self.inner.lock();
        Ok(state
            .rows
            .iter()
            .find(|(id, user)| {
                !state.deleted_ids.contains(*id)
                    && user.deleted_at.is_none()
                    && user.email.to_ascii_lowercase() == needle
            })
            .map(|(_, user)| Arc::clone(user)))
    }

    async fn update(&self, row: UserRow, expected_version: u64) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        let Some(cur) = state
            .rows
            .get(&row.id)
            .filter(|user| !state.deleted_ids.contains(&row.id) && user.deleted_at.is_none())
        else {
            return Err(StorageError::not_found("user", row.id));
        };
        if cur.version != expected_version {
            return Err(StorageError::Conflict {
                entity: "user",
                id: row.id,
                expected: expected_version,
                actual: cur.version,
            });
        }
        // Re-enforce the create-path active-email-uniqueness invariant on
        // update: an email change must not collide with another active
        // user. Without this an `update` could silently introduce a
        // duplicate the `create` path forbids (first-writer-wins).
        let email = row.email.to_ascii_lowercase();
        if state.rows.iter().any(|(id, user)| {
            *id != row.id
                && !state.deleted_ids.contains(id)
                && user.deleted_at.is_none()
                && user.email.to_ascii_lowercase() == email
        }) {
            return Err(StorageError::Duplicate {
                entity: "user",
                detail: format!("active user with email {} already exists", row.email),
            });
        }
        state.rows.insert(row.id.clone(), Arc::new(row));
        Ok(())
    }

    async fn soft_delete(&self, id: &str) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        let Some(row) = state
            .rows
            .get(id)
            .filter(|user| !state.deleted_ids.contains(id) && user.deleted_at.is_none())
        else {
            return Err(StorageError::not_found("user", id));
        };
        let _ = row;
        state.deleted_ids.insert(id.to_owned());
        Ok(())
    }
}

// ── Orgs ──────────────────────────────────────────────────────────────────

/// Directory rows and grants share a snapshot and mutation critical section.
#[derive(Debug, Default)]
struct InMemoryDirectoryState {
    orgs: HashMap<String, OrgRow>,
    workspaces: HashMap<WsKey, WorkspaceRow>,
    memberships: HashMap<MemKey, MembershipRow>,
}

/// Composition root for in-memory tenant directory stores.
///
/// The three projections share one lock so parent checks, membership reads,
/// and guarded mutations observe one logical snapshot.
#[derive(Debug, Default, Clone)]
pub struct InMemoryIdentityDirectory {
    inner: Arc<Mutex<InMemoryDirectoryState>>,
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
        InMemoryOrgStore {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Workspace projection over the shared directory state.
    #[must_use]
    pub fn workspace_store(&self) -> InMemoryWorkspaceStore {
        InMemoryWorkspaceStore {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Membership projection over the shared directory state.
    #[must_use]
    pub fn membership_store(&self) -> InMemoryMembershipStore {
        InMemoryMembershipStore {
            inner: Arc::clone(&self.inner),
        }
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
        let owner_key = mem_key(
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

/// In-memory `orgs` store. Slug is unique among active rows.
#[derive(Debug, Default, Clone)]
pub struct InMemoryOrgStore {
    inner: Arc<Mutex<InMemoryDirectoryState>>,
}

impl InMemoryOrgStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl OrgStore for InMemoryOrgStore {
    async fn create(&self, row: OrgRow) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        let map = &mut state.orgs;
        if map.contains_key(&row.id) {
            return Err(StorageError::Duplicate {
                entity: "org",
                detail: format!("org {} already exists", row.id),
            });
        }
        if map
            .values()
            .any(|o| o.deleted_at.is_none() && o.slug == row.slug)
        {
            return Err(StorageError::Duplicate {
                entity: "org",
                detail: format!("active org with slug {} already exists", row.slug),
            });
        }
        map.insert(row.id.clone(), row);
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<OrgRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .orgs
            .get(id)
            .filter(|o| o.deleted_at.is_none())
            .cloned())
    }

    async fn get_by_slug(&self, slug: &str) -> Result<Option<OrgRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .orgs
            .values()
            .find(|o| o.deleted_at.is_none() && o.slug == slug)
            .cloned())
    }

    async fn update(&self, row: OrgRow, expected_version: u64) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        let map = &mut state.orgs;
        let Some(cur) = map.get(&row.id).filter(|o| o.deleted_at.is_none()) else {
            return Err(StorageError::not_found("org", row.id));
        };
        if cur.version != expected_version {
            return Err(StorageError::Conflict {
                entity: "org",
                id: row.id,
                expected: expected_version,
                actual: cur.version,
            });
        }
        // Re-enforce the create-path active-slug-uniqueness invariant on
        // update: a slug change must not collide with another active org.
        if map
            .values()
            .any(|o| o.id != row.id && o.deleted_at.is_none() && o.slug == row.slug)
        {
            return Err(StorageError::Duplicate {
                entity: "org",
                detail: format!("active org with slug {} already exists", row.slug),
            });
        }
        map.insert(row.id.clone(), row);
        Ok(())
    }

    async fn soft_delete(&self, id: &str) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        let map = &mut state.orgs;
        let Some(row) = map.get_mut(id).filter(|o| o.deleted_at.is_none()) else {
            return Err(StorageError::not_found("org", id));
        };
        row.deleted_at = Some(now_rfc3339());
        Ok(())
    }
}

// ── Workspaces ────────────────────────────────────────────────────────────

/// Workspace key: `(org_id, workspace_id)` so a cross-org `get` misses.
type WsKey = (String, String);

/// In-memory `workspaces` store (scoped by parent org). Slug is unique
/// among active rows *per org*.
#[derive(Debug, Default, Clone)]
pub struct InMemoryWorkspaceStore {
    inner: Arc<Mutex<InMemoryDirectoryState>>,
}

impl InMemoryWorkspaceStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl WorkspaceStore for InMemoryWorkspaceStore {
    async fn create(&self, row: WorkspaceRow) -> Result<(), StorageError> {
        let key = (row.org_id.clone(), row.id.clone());
        let mut state = self.inner.lock();
        let map = &mut state.workspaces;
        if map.contains_key(&key) {
            return Err(StorageError::Duplicate {
                entity: "workspace",
                detail: format!("workspace {} already exists", row.id),
            });
        }
        if map
            .values()
            .any(|w| w.deleted_at.is_none() && w.org_id == row.org_id && w.slug == row.slug)
        {
            return Err(StorageError::Duplicate {
                entity: "workspace",
                detail: format!(
                    "active workspace with slug {} already exists in org {}",
                    row.slug, row.org_id
                ),
            });
        }
        map.insert(key, row);
        Ok(())
    }

    async fn get(&self, org_id: &str, id: &str) -> Result<Option<WorkspaceRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .workspaces
            .get(&(org_id.to_string(), id.to_string()))
            .filter(|w| w.deleted_at.is_none())
            .cloned())
    }

    async fn get_by_slug(
        &self,
        org_id: &str,
        slug: &str,
    ) -> Result<Option<WorkspaceRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .workspaces
            .values()
            .find(|row| row.deleted_at.is_none() && row.org_id == org_id && row.slug == slug)
            .cloned())
    }

    async fn list_for_org(&self, org_id: &str) -> Result<Vec<WorkspaceRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .workspaces
            .values()
            .filter(|w| w.deleted_at.is_none() && w.org_id == org_id)
            .cloned()
            .collect())
    }

    async fn update(&self, row: WorkspaceRow, expected_version: u64) -> Result<(), StorageError> {
        let key = (row.org_id.clone(), row.id.clone());
        let mut state = self.inner.lock();
        let map = &mut state.workspaces;
        let Some(cur) = map.get(&key).filter(|w| w.deleted_at.is_none()) else {
            return Err(StorageError::not_found("workspace", row.id));
        };
        if cur.version != expected_version {
            return Err(StorageError::Conflict {
                entity: "workspace",
                id: row.id,
                expected: expected_version,
                actual: cur.version,
            });
        }
        // Re-enforce the create-path active-slug-uniqueness invariant
        // (slug is unique among active rows per org) on update.
        if map.values().any(|w| {
            w.id != row.id && w.deleted_at.is_none() && w.org_id == row.org_id && w.slug == row.slug
        }) {
            return Err(StorageError::Duplicate {
                entity: "workspace",
                detail: format!(
                    "active workspace with slug {} already exists in org {}",
                    row.slug, row.org_id
                ),
            });
        }
        map.insert(key, row);
        Ok(())
    }

    async fn soft_delete(&self, org_id: &str, id: &str) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        let map = &mut state.workspaces;
        let Some(row) = map
            .get_mut(&(org_id.to_string(), id.to_string()))
            .filter(|w| w.deleted_at.is_none())
        else {
            return Err(StorageError::not_found("workspace", id));
        };
        row.deleted_at = Some(now_rfc3339());
        Ok(())
    }
}

// ── Memberships ───────────────────────────────────────────────────────────

/// Membership key: `(scope_kind, scope_id, principal_kind, principal_id)`.
/// `*_kind` is keyed by the enum's stable text form so lookups match the
/// stored row exactly.
type MemKey = (String, String, String, String);

fn mem_key(
    scope_kind: ScopeKind,
    scope_id: &str,
    principal_kind: PrincipalKind,
    principal_id: &str,
) -> MemKey {
    (
        scope_kind.as_str().to_string(),
        scope_id.to_string(),
        principal_kind.as_str().to_string(),
        principal_id.to_string(),
    )
}

/// In-memory `org_members` + `workspace_members` store.
#[derive(Debug, Default, Clone)]
pub struct InMemoryMembershipStore {
    inner: Arc<Mutex<InMemoryDirectoryState>>,
}

impl InMemoryMembershipStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
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
            .get(&mem_key(
                ScopeKind::Org,
                org_id,
                principal_kind,
                principal_id,
            ))
            .map(|row| parse_org_role(&row.role))
            .transpose()?;
        let workspace_role = workspace_id
            .filter(|id| live_membership_workspace(&state, org_id, id))
            .and_then(|id| {
                state.memberships.get(&mem_key(
                    ScopeKind::Workspace,
                    id,
                    principal_kind,
                    principal_id,
                ))
            })
            .map(|row| {
                WorkspaceMembershipRole::parse(&row.role)
                    .map_err(|_| StorageError::Serialization("membership role is invalid".into()))
            })
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
                    role: parse_org_role(&row.role)?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        result.sort_by(|a, b| a.org_id.cmp(&b.org_id));
        Ok(result)
    }

    #[tracing::instrument(skip_all)]
    async fn upsert_org_member_guarded(
        &self,
        request: OrgMemberUpsert,
    ) -> Result<OrgMemberUpsertOutcome, StorageError> {
        let mut state = self.inner.lock();
        if state
            .orgs
            .get(&request.org_id)
            .is_none_or(|org| org.deleted_at.is_some())
        {
            return Err(StorageError::not_found("org", request.org_id));
        }
        let mut privileged_other = false;
        for row in state
            .memberships
            .values()
            .filter(|row| row.scope_kind == ScopeKind::Org && row.scope_id == request.org_id)
        {
            let role = parse_org_role(&row.role)?;
            privileged_other |= role.is_privileged()
                && (row.principal_kind != request.principal_kind
                    || row.principal_id != request.principal_id);
        }
        if !request.role.is_privileged() && !privileged_other {
            return Ok(OrgMemberUpsertOutcome::WouldLockOut);
        }
        let row = MembershipRow {
            scope_kind: ScopeKind::Org,
            scope_id: request.org_id,
            principal_kind: request.principal_kind,
            principal_id: request.principal_id,
            role: request.role.as_str().into(),
            added_at: now_rfc3339(),
            added_by: request.added_by,
        };
        state.memberships.insert(
            mem_key(
                row.scope_kind,
                &row.scope_id,
                row.principal_kind,
                &row.principal_id,
            ),
            row,
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
        if state
            .orgs
            .get(org_id)
            .is_none_or(|org| org.deleted_at.is_some())
        {
            return Err(StorageError::not_found("org", org_id));
        }
        let key = mem_key(ScopeKind::Org, org_id, principal_kind, principal_id);
        let mut privileged_other = false;
        for row in state
            .memberships
            .values()
            .filter(|row| row.scope_kind == ScopeKind::Org && row.scope_id == org_id)
        {
            let role = parse_org_role(&row.role)?;
            privileged_other |= role.is_privileged()
                && (row.principal_kind != principal_kind || row.principal_id != principal_id);
        }
        if !state.memberships.contains_key(&key) {
            return Ok(OrgMemberRemoveOutcome::NotFound);
        }
        if !privileged_other {
            return Ok(OrgMemberRemoveOutcome::WouldLockOut);
        }
        state.memberships.remove(&key);
        Ok(OrgMemberRemoveOutcome::Removed)
    }

    #[tracing::instrument(skip_all)]
    async fn upsert_workspace_member(
        &self,
        request: WorkspaceMemberUpsert,
    ) -> Result<(), StorageError> {
        let mut state = self.inner.lock();
        if !live_membership_workspace(&state, &request.org_id, &request.workspace_id) {
            return Err(StorageError::not_found("workspace", request.workspace_id));
        }
        let row = MembershipRow {
            scope_kind: ScopeKind::Workspace,
            scope_id: request.workspace_id,
            principal_kind: request.principal_kind,
            principal_id: request.principal_id,
            role: request.role.as_str().into(),
            added_at: now_rfc3339(),
            added_by: request.added_by,
        };
        state.memberships.insert(
            mem_key(
                row.scope_kind,
                &row.scope_id,
                row.principal_kind,
                &row.principal_id,
            ),
            row,
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
            .get(&mem_key(scope_kind, scope_id, principal_kind, principal_id))
            .cloned())
    }

    async fn list_for_scope(
        &self,
        scope_kind: ScopeKind,
        scope_id: &str,
    ) -> Result<Vec<MembershipRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .memberships
            .values()
            .filter(|row| row.scope_kind == scope_kind && row.scope_id == scope_id)
            .cloned()
            .collect())
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
        if !live_membership_workspace(&state, org_id, workspace_id) {
            return Ok(false);
        }
        Ok(state
            .memberships
            .remove(&mem_key(
                ScopeKind::Workspace,
                workspace_id,
                principal_kind,
                principal_id,
            ))
            .is_some())
    }
}

// Grants are keyed by workspace id in the existing schema. A second parent,
// even a deleted one, makes that identity ambiguous; never reuse its grants.
fn live_membership_workspace(
    state: &InMemoryDirectoryState,
    org_id: &str,
    workspace_id: &str,
) -> bool {
    state
        .workspaces
        .get(&(org_id.to_owned(), workspace_id.to_owned()))
        .is_some_and(|row| row.deleted_at.is_none())
        && !state
            .workspaces
            .values()
            .any(|row| row.id == workspace_id && row.org_id != org_id)
}

fn parse_org_role(value: &str) -> Result<OrgMembershipRole, StorageError> {
    OrgMembershipRole::parse(value)
        .map_err(|_| StorageError::Serialization("membership role is invalid".into()))
}

// ── Resources (workspace-scoped) ──────────────────────────────────────────

/// Scoped key: `(workspace_id, org_id, id)`.
type ScopedKey = (String, String, String);

fn scoped_key(scope: &Scope, id: &str) -> ScopedKey {
    (
        scope.workspace_id.clone(),
        scope.org_id.clone(),
        id.to_string(),
    )
}

/// In-memory `resources` store. Slug is unique among active rows per
/// workspace scope.
#[derive(Debug, Default, Clone)]
pub struct InMemoryResourceStore {
    inner: Arc<Mutex<HashMap<ScopedKey, ResourceRow>>>,
}

impl InMemoryResourceStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl ResourceStore for InMemoryResourceStore {
    async fn create(&self, scope: &Scope, row: ResourceRow) -> Result<(), StorageError> {
        let key = scoped_key(scope, &row.id);
        let mut map = self.inner.lock();
        if map.contains_key(&key) {
            return Err(StorageError::Duplicate {
                entity: "resource",
                detail: format!("resource {} already exists", row.id),
            });
        }
        if map.iter().any(|((ws, org, _), r)| {
            *ws == scope.workspace_id
                && *org == scope.org_id
                && r.deleted_at.is_none()
                && r.slug == row.slug
        }) {
            return Err(StorageError::Duplicate {
                entity: "resource",
                detail: format!("active resource with slug {} already exists", row.slug),
            });
        }
        map.insert(key, row);
        Ok(())
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ResourceRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .get(&scoped_key(scope, id))
            .filter(|r| r.deleted_at.is_none())
            .cloned())
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<ResourceRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .iter()
            .filter(|((ws, org, _), r)| {
                *ws == scope.workspace_id && *org == scope.org_id && r.deleted_at.is_none()
            })
            .map(|(_, r)| r.clone())
            .collect())
    }

    async fn update(
        &self,
        scope: &Scope,
        row: ResourceRow,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        let key = scoped_key(scope, &row.id);
        let mut map = self.inner.lock();
        let Some(cur) = map.get(&key).filter(|r| r.deleted_at.is_none()) else {
            return Err(StorageError::not_found("resource", row.id));
        };
        if cur.version != expected_version {
            return Err(StorageError::Conflict {
                entity: "resource",
                id: row.id,
                expected: expected_version,
                actual: cur.version,
            });
        }
        // Re-enforce the create-path active-slug-uniqueness invariant
        // (slug unique among active rows in this scope) on update.
        if map.iter().any(|((ws, org, rid), r)| {
            *ws == scope.workspace_id
                && *org == scope.org_id
                && *rid != row.id
                && r.deleted_at.is_none()
                && r.slug == row.slug
        }) {
            return Err(StorageError::Duplicate {
                entity: "resource",
                detail: format!("active resource with slug {} already exists", row.slug),
            });
        }
        map.insert(key, row);
        Ok(())
    }

    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError> {
        let mut map = self.inner.lock();
        let Some(row) = map
            .get_mut(&scoped_key(scope, id))
            .filter(|r| r.deleted_at.is_none())
        else {
            return Err(StorageError::not_found("resource", id));
        };
        row.deleted_at = Some(now_rfc3339());
        Ok(())
    }
}

// ── Triggers (workspace-scoped) ───────────────────────────────────────────

/// In-memory `triggers` store.
#[derive(Debug, Default, Clone)]
pub struct InMemoryTriggerStore {
    inner: Arc<Mutex<HashMap<ScopedKey, TriggerRow>>>,
}

impl InMemoryTriggerStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl TriggerStore for InMemoryTriggerStore {
    async fn create(&self, scope: &Scope, row: TriggerRow) -> Result<(), StorageError> {
        let key = scoped_key(scope, &row.id);
        let mut map = self.inner.lock();
        if map.contains_key(&key) {
            return Err(StorageError::Duplicate {
                entity: "trigger",
                detail: format!("trigger {} already exists", row.id),
            });
        }
        map.insert(key, row);
        Ok(())
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<TriggerRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .get(&scoped_key(scope, id))
            .filter(|t| t.deleted_at.is_none())
            .cloned())
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<TriggerRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .iter()
            .filter(|((ws, org, _), t)| {
                *ws == scope.workspace_id && *org == scope.org_id && t.deleted_at.is_none()
            })
            .map(|(_, t)| t.clone())
            .collect())
    }

    async fn update(
        &self,
        scope: &Scope,
        row: TriggerRow,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        let key = scoped_key(scope, &row.id);
        let mut map = self.inner.lock();
        let Some(cur) = map.get(&key).filter(|t| t.deleted_at.is_none()) else {
            return Err(StorageError::not_found("trigger", row.id));
        };
        if cur.version != expected_version {
            return Err(StorageError::Conflict {
                entity: "trigger",
                id: row.id,
                expected: expected_version,
                actual: cur.version,
            });
        }
        map.insert(key, row);
        Ok(())
    }

    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError> {
        let mut map = self.inner.lock();
        let Some(row) = map
            .get_mut(&scoped_key(scope, id))
            .filter(|t| t.deleted_at.is_none())
        else {
            return Err(StorageError::not_found("trigger", id));
        };
        row.deleted_at = Some(now_rfc3339());
        Ok(())
    }
}

// ── Quotas (org-scoped, CAS counters) ─────────────────────────────────────

/// In-memory `org_quotas` + `org_quota_usage` store.
#[derive(Debug, Default, Clone)]
pub struct InMemoryQuotaStore {
    inner: Arc<Mutex<HashMap<String, QuotaRow>>>,
}

impl InMemoryQuotaStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl QuotaStore for InMemoryQuotaStore {
    async fn get(&self, org_id: &str) -> Result<Option<QuotaRow>, StorageError> {
        Ok(self.inner.lock().get(org_id).cloned())
    }

    async fn upsert(&self, row: QuotaRow) -> Result<(), StorageError> {
        self.inner.lock().insert(row.org_id.clone(), row);
        Ok(())
    }

    async fn adjust_concurrent(&self, org_id: &str, delta: i32) -> Result<i32, StorageError> {
        let mut map = self.inner.lock();
        let Some(row) = map.get_mut(org_id) else {
            return Err(StorageError::not_found("quota", org_id));
        };
        let next = row.concurrent_executions + delta;
        if next < 0 {
            return Err(StorageError::Conflict {
                entity: "quota",
                id: org_id.to_string(),
                expected: 0,
                actual: row.concurrent_executions as u64,
            });
        }
        row.concurrent_executions = next;
        Ok(next)
    }
}

// ── Audit log (append-only) ───────────────────────────────────────────────

/// In-memory `audit_log` store. Append-only; reads are newest-first.
#[derive(Debug, Default, Clone)]
pub struct InMemoryAuditStore {
    inner: Arc<Mutex<Vec<AuditLogRow>>>,
}

impl InMemoryAuditStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl AuditStore for InMemoryAuditStore {
    async fn append(&self, row: AuditLogRow) -> Result<(), StorageError> {
        self.inner.lock().push(row);
        Ok(())
    }

    async fn list_for_org(
        &self,
        org_id: &str,
        limit: u32,
    ) -> Result<Vec<AuditLogRow>, StorageError> {
        let log = self.inner.lock();
        let mut rows: Vec<AuditLogRow> =
            log.iter().filter(|r| r.org_id == org_id).cloned().collect();
        // Newest first: emitted_at descending, ULID id as the tiebreaker
        // (monotone — same total order the SQL `ORDER BY emitted_at DESC,
        // id DESC` produces).
        rows.sort_by(|a, b| {
            b.emitted_at
                .cmp(&a.emitted_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        rows.truncate(limit as usize);
        Ok(rows)
    }
}

// ── Blobs (workspace-scoped) ──────────────────────────────────────────────

/// Blob key: `(workspace_id, id)` so a cross-workspace `get` misses.
type BlobKey = (String, String);

/// In-memory `blobs` store.
#[derive(Debug, Default, Clone)]
pub struct InMemoryBlobStore {
    inner: Arc<Mutex<HashMap<BlobKey, BlobRow>>>,
}

impl InMemoryBlobStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl BlobStore for InMemoryBlobStore {
    async fn put(&self, row: BlobRow) -> Result<(), StorageError> {
        let key = (row.workspace_id.clone(), row.id.clone());
        self.inner.lock().insert(key, row);
        Ok(())
    }

    async fn get(&self, workspace_id: &str, id: &str) -> Result<Option<BlobRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .get(&(workspace_id.to_string(), id.to_string()))
            .cloned())
    }

    async fn delete(&self, workspace_id: &str, id: &str) -> Result<(), StorageError> {
        self.inner
            .lock()
            .remove(&(workspace_id.to_string(), id.to_string()));
        Ok(())
    }

    async fn evict_expired(&self) -> Result<u64, StorageError> {
        // Compare parsed instants, not RFC3339 strings: a lexical compare
        // is wrong across differing offsets / fractional-second precision
        // / timezones (`…T00:00:00Z` vs `…T01:00:00+01:00` denote the same
        // instant but don't order lexically). A blob with an unparseable
        // `expires_at` is treated as already expired (fail-closed — never
        // retain a row we cannot prove is still fresh), mirroring the
        // idempotency cache's `expires_at_ms`.
        let now = chrono::Utc::now().timestamp_millis();
        let mut map = self.inner.lock();
        let before = map.len();
        map.retain(|_, b| match &b.expires_at {
            Some(exp) => {
                chrono::DateTime::parse_from_rfc3339(exp)
                    .map(|dt| dt.timestamp_millis())
                    .unwrap_or(i64::MIN)
                    > now
            },
            None => true,
        });
        Ok((before - map.len()) as u64)
    }
}

// ── shared ────────────────────────────────────────────────────────────────

/// Current time as an RFC 3339 string (the soft-delete / eviction stamp
/// format the port DTOs use; consistent with the SQL backends' `NOW()`
/// rendered through the same encoding).
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}
