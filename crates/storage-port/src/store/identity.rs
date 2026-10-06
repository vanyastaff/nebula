//! Tenant directory and workspace-object store traits.
//!
//! Every tenant-scoped query is keyed by `Scope` (or a parent id) so
//! cross-tenant reads return `None`, never another tenant's row. User accounts
//! are not here: they are Plane-A persistence (`nebula_storage::auth`).

use crate::dto::{
    OrgMemberRemoveOutcome, OrgMemberUpsert, OrgMemberUpsertOutcome, OrgMembership, OrgRow,
    PrincipalKind, PrincipalOrgMembership, ResourceRow, TenantMembershipSnapshot,
    TenantProvisioningOutcome, TenantProvisioningRequest, TriggerRow, WorkspaceMemberUpsert,
    WorkspaceMembership, WorkspaceRow,
};
use crate::error::StorageError;
use crate::scope::Scope;

/// `orgs` aggregate.
#[async_trait::async_trait]
pub trait OrgStore: Send + Sync + std::fmt::Debug {
    /// Insert a new org (duplicate active slug ⇒ `Duplicate`).
    async fn create(&self, row: OrgRow) -> Result<(), StorageError>;
    /// Read an org by id.
    async fn get(&self, id: &str) -> Result<Option<OrgRow>, StorageError>;
    /// Resolve an active org by slug.
    async fn get_by_slug(&self, slug: &str) -> Result<Option<OrgRow>, StorageError>;
    /// CAS-update an org row.
    async fn update(&self, row: OrgRow, expected_version: u64) -> Result<(), StorageError>;
    /// Soft-delete an org.
    async fn soft_delete(&self, id: &str) -> Result<(), StorageError>;
}

/// `workspaces` aggregate (scoped by parent org).
#[async_trait::async_trait]
pub trait WorkspaceStore: Send + Sync + std::fmt::Debug {
    /// Insert a new workspace. Workspace ids are unique across organizations;
    /// a taken id, a duplicate active slug per org or a second active default
    /// ⇒ `Duplicate`, a missing or deleted parent org ⇒ `NotFound`.
    async fn create(&self, row: WorkspaceRow) -> Result<(), StorageError>;
    /// Read a workspace by id; `org_id` scopes the lookup.
    async fn get(&self, org_id: &str, id: &str) -> Result<Option<WorkspaceRow>, StorageError>;
    /// Resolve an active workspace by slug within its parent organization.
    async fn get_by_slug(
        &self,
        org_id: &str,
        slug: &str,
    ) -> Result<Option<WorkspaceRow>, StorageError>;
    /// List active workspaces for an org.
    async fn list_for_org(&self, org_id: &str) -> Result<Vec<WorkspaceRow>, StorageError>;
    /// CAS-update a workspace row.
    async fn update(&self, row: WorkspaceRow, expected_version: u64) -> Result<(), StorageError>;
    /// Soft-delete a workspace.
    async fn soft_delete(&self, org_id: &str, id: &str) -> Result<(), StorageError>;
}

/// Atomic creation boundary for a tenant's initial durable authority.
///
/// This port deliberately spans the organization, default workspace, and
/// initial-owner records because exposing their bootstrap as three writes
/// permits ownerless or workspace-less tenants after a partial failure.
#[async_trait::async_trait]
pub trait TenantProvisioningStore: Send + Sync + std::fmt::Debug {
    /// Create all initial tenant records in one transaction or critical section.
    ///
    /// An exact retry returns [`TenantProvisioningOutcome::Replayed`] without
    /// rewriting any record or refreshing the owner grant. Any partial state,
    /// semantic mismatch, id collision, or active-slug collision returns a
    /// conflict outcome and leaves all existing state unchanged.
    async fn provision_tenant(
        &self,
        request: TenantProvisioningRequest,
    ) -> Result<TenantProvisioningOutcome, StorageError>;
}

/// Organization and workspace memberships.
#[async_trait::async_trait]
pub trait MembershipStore: Send + Sync + std::fmt::Debug {
    /// Read explicit organization and optional workspace roles for one principal
    /// from one logical snapshot. Two independent reads are not sufficient.
    /// The adapter must reject unknown persisted roles, never treat them as absent.
    /// Roles are returned only beneath a live organization: a deleted
    /// organization yields neither role. A workspace role may be returned only
    /// for a live workspace belonging to `org_id`, verified in the same
    /// snapshot; a missing, deleted or wrong-parent workspace yields no
    /// workspace role.
    /// This operation reads membership evidence and does not grant authority.
    async fn get_tenant_membership(
        &self,
        org_id: &str,
        workspace_id: Option<&str>,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<TenantMembershipSnapshot, StorageError>;

    /// Enumerate explicit memberships of live organizations for exactly this
    /// principal, ordered by organization id. Unknown persisted organization
    /// roles fail the whole read closed.
    async fn list_orgs_for_principal(
        &self,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<Vec<PrincipalOrgMembership>, StorageError>;

    /// List explicit grants of one live organization, ordered by principal
    /// kind then id (bytewise). A missing or deleted organization fails
    /// closed as `StorageError::NotFound`; unknown persisted roles fail the
    /// whole read closed.
    async fn list_org_members(&self, org_id: &str) -> Result<Vec<OrgMembership>, StorageError>;

    /// List explicit grants for one live, parent-qualified workspace.
    ///
    /// Results use the closed workspace-role vocabulary and deterministic
    /// principal ordering. A missing, deleted or wrong-parent workspace fails
    /// closed as `StorageError::NotFound`.
    async fn list_workspace_members(
        &self,
        org_id: &str,
        workspace_id: &str,
    ) -> Result<Vec<WorkspaceMembership>, StorageError>;

    /// Atomically replace an organization membership only when the resulting
    /// organization retains at least one owner or administrator. This also
    /// applies to the first insert: bootstrap must insert a privileged role.
    /// The role read,
    /// privileged-member count, and write must share an exclusive organization
    /// critical section across replicas, including concurrent removals.
    /// Unknown roles encountered by the invariant check fail closed with no write.
    /// The adapter records `added_at` using its own clock inside the atomic write;
    /// callers cannot supply the persisted write timestamp.
    async fn upsert_org_member_guarded(
        &self,
        request: OrgMemberUpsert,
    ) -> Result<OrgMemberUpsertOutcome, StorageError>;

    /// Atomically remove membership unless it is the last owner/administrator.
    /// Shares the organization critical section used by guarded upsert; neither
    /// a count-then-delete sequence nor a process-local lock suffices for SQL.
    /// Unknown roles encountered by the invariant check fail closed with no write.
    /// A successful removal also deletes every explicit workspace grant for
    /// the principal beneath this organization in the same transaction.
    async fn remove_org_member_guarded(
        &self,
        org_id: &str,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<OrgMemberRemoveOutcome, StorageError>;

    /// Replace explicit workspace membership after verifying a live workspace
    /// under the requested organization and a current organization membership
    /// for the same principal. Missing membership or a wrong/missing parent
    /// returns `StorageError::NotFound`; this cannot mutate organization roles.
    /// The organization-membership check and write serialize with guarded
    /// organization-member removal and with a workspace soft delete, so a
    /// concurrent removal or delete either rejects this write or atomically
    /// deletes its result.
    /// The adapter records `added_at` using its own clock inside the atomic write.
    async fn upsert_workspace_member(
        &self,
        request: WorkspaceMemberUpsert,
    ) -> Result<(), StorageError>;
    /// Remove explicit workspace membership, scoped through a live workspace's
    /// organization. Returns false for absent membership or a wrong/missing
    /// parent. Organization memberships are writable only through the guarded
    /// methods.
    async fn remove_workspace_member(
        &self,
        org_id: &str,
        workspace_id: &str,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<bool, StorageError>;
}

/// `resources` aggregate (workspace-scoped).
#[async_trait::async_trait]
pub trait ResourceStore: Send + Sync + std::fmt::Debug {
    /// Insert a new resource (duplicate active slug per workspace ⇒
    /// `Duplicate`).
    async fn create(&self, scope: &Scope, row: ResourceRow) -> Result<(), StorageError>;
    /// Read a resource by id within `scope`.
    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ResourceRow>, StorageError>;
    /// List active resources in `scope`.
    async fn list(&self, scope: &Scope) -> Result<Vec<ResourceRow>, StorageError>;
    /// CAS-update a resource row.
    async fn update(
        &self,
        scope: &Scope,
        row: ResourceRow,
        expected_version: u64,
    ) -> Result<(), StorageError>;
    /// Soft-delete a resource.
    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError>;
}

/// `triggers` aggregate (workspace-scoped).
#[async_trait::async_trait]
pub trait TriggerStore: Send + Sync + std::fmt::Debug {
    /// Insert a new trigger.
    async fn create(&self, scope: &Scope, row: TriggerRow) -> Result<(), StorageError>;
    /// Read a trigger by id within `scope`.
    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<TriggerRow>, StorageError>;
    /// List active triggers in `scope`.
    async fn list(&self, scope: &Scope) -> Result<Vec<TriggerRow>, StorageError>;
    /// CAS-update a trigger row.
    async fn update(
        &self,
        scope: &Scope,
        row: TriggerRow,
        expected_version: u64,
    ) -> Result<(), StorageError>;
    /// Soft-delete a trigger.
    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError>;
}
