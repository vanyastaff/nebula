//! Identity-zoo store traits.
//!
//! These declare the contract; adapter implementations for InMemory/SQLite/
//! Postgres land later. Every tenant-scoped query is keyed by `Scope` (or a
//! parent id) so cross-tenant reads return `None`, never another tenant's
//! row.
use std::sync::Arc;

use crate::dto::{
    AuditLogRow, BlobRow, MembershipRow, OrgMemberRemoveOutcome, OrgMemberUpsert,
    OrgMemberUpsertOutcome, OrgRow, PrincipalKind, PrincipalOrgMembership, QuotaRow, ResourceRow,
    ScopeKind, TenantMembershipSnapshot, TenantProvisioningOutcome, TenantProvisioningRequest,
    TriggerRow, UserRow, WorkspaceMemberUpsert, WorkspaceRow,
};
use crate::error::StorageError;
use crate::scope::Scope;

/// `users` aggregate. Users are global (not workspace-scoped) but lookups
/// stay first-writer-wins on email among active rows.
#[async_trait::async_trait]
pub trait UserStore: Send + Sync + std::fmt::Debug {
    /// Insert a new user (duplicate active email ⇒ `Duplicate`).
    async fn create(&self, row: UserRow) -> Result<(), StorageError>;
    /// Read a user by id.
    async fn get(&self, id: &str) -> Result<Option<Arc<UserRow>>, StorageError>;
    /// Resolve an active user by (case-insensitive) email.
    async fn get_by_email(&self, email: &str) -> Result<Option<Arc<UserRow>>, StorageError>;
    /// CAS-update a user row; `expected_version` must match.
    async fn update(&self, row: UserRow, expected_version: u64) -> Result<(), StorageError>;
    /// Soft-delete a user.
    async fn soft_delete(&self, id: &str) -> Result<(), StorageError>;
}

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
    /// Insert a new workspace (duplicate active slug per org ⇒ `Duplicate`).
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

/// `org_members` + `workspace_members` aggregate.
#[async_trait::async_trait]
pub trait MembershipStore: Send + Sync + std::fmt::Debug {
    /// Read explicit organization and optional workspace roles for one principal
    /// from one logical snapshot. Two independent reads are not sufficient.
    /// The adapter must reject unknown persisted roles, never treat them as absent.
    /// A workspace role may be returned only for a live workspace belonging to
    /// `org_id`, verified in the same snapshot. A missing/deleted/wrong-parent
    /// workspace yields no workspace role. Because the legacy membership key
    /// omits the parent organization, an id present under any second organization
    /// is ambiguous (including a deleted alias) and must also yield no role.
    /// This operation reads membership evidence and does not grant authority.
    async fn get_tenant_membership(
        &self,
        org_id: &str,
        workspace_id: Option<&str>,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<TenantMembershipSnapshot, StorageError>;

    /// Enumerate explicit organization memberships for exactly this principal.
    /// Unknown persisted organization roles fail the whole read closed.
    async fn list_orgs_for_principal(
        &self,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<Vec<PrincipalOrgMembership>, StorageError>;

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
    async fn remove_org_member_guarded(
        &self,
        org_id: &str,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<OrgMemberRemoveOutcome, StorageError>;

    /// Replace explicit workspace membership after verifying a live workspace
    /// under the requested organization. Missing, wrong-parent, or ambiguous
    /// workspace ids return `StorageError::NotFound`; this cannot mutate
    /// organization roles.
    /// The adapter records `added_at` using its own clock inside the atomic write.
    async fn upsert_workspace_member(
        &self,
        request: WorkspaceMemberUpsert,
    ) -> Result<(), StorageError>;
    /// Read one membership by (scope_kind, scope_id, principal).
    async fn get(
        &self,
        scope_kind: ScopeKind,
        scope_id: &str,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<Option<MembershipRow>, StorageError>;
    /// List all members of a scope (org or workspace).
    async fn list_for_scope(
        &self,
        scope_kind: ScopeKind,
        scope_id: &str,
    ) -> Result<Vec<MembershipRow>, StorageError>;
    /// Remove explicit workspace membership, scoped through a live workspace's
    /// organization. Returns false for absent membership, wrong/missing parent,
    /// or a workspace id that is ambiguous across organizations.
    /// Organization memberships are writable only through the guarded methods.
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

/// `org_quotas` + `org_quota_usage` aggregate (org-scoped, CAS counters).
#[async_trait::async_trait]
pub trait QuotaStore: Send + Sync + std::fmt::Debug {
    /// Read the quota row for an org.
    async fn get(&self, org_id: &str) -> Result<Option<QuotaRow>, StorageError>;
    /// Upsert the quota limits + usage row.
    async fn upsert(&self, row: QuotaRow) -> Result<(), StorageError>;
    /// Atomically adjust the concurrent-execution counter by `delta`,
    /// returning the new value. Rejects going below zero.
    async fn adjust_concurrent(&self, org_id: &str, delta: i32) -> Result<i32, StorageError>;
}

/// `audit_log` aggregate (append-only, org/workspace-scoped).
#[async_trait::async_trait]
pub trait AuditStore: Send + Sync + std::fmt::Debug {
    /// Append one audit-log row.
    async fn append(&self, row: AuditLogRow) -> Result<(), StorageError>;
    /// List recent audit rows for an org, newest first, capped by `limit`.
    async fn list_for_org(
        &self,
        org_id: &str,
        limit: u32,
    ) -> Result<Vec<AuditLogRow>, StorageError>;
}

/// `blobs` aggregate (workspace-scoped).
#[async_trait::async_trait]
pub trait BlobStore: Send + Sync + std::fmt::Debug {
    /// Persist a blob row.
    async fn put(&self, row: BlobRow) -> Result<(), StorageError>;
    /// Read a blob row by id within a workspace.
    async fn get(&self, workspace_id: &str, id: &str) -> Result<Option<BlobRow>, StorageError>;
    /// Delete a blob row.
    async fn delete(&self, workspace_id: &str, id: &str) -> Result<(), StorageError>;
    /// Delete expired temp blobs; returns the count deleted.
    async fn evict_expired(&self) -> Result<u64, StorageError>;
}
