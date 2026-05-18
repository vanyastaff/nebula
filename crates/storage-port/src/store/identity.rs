//! Identity-zoo store traits.
//!
//! These declare the contract; adapter implementations for InMemory/SQLite/
//! Postgres land later. Every tenant-scoped query is keyed by `Scope` (or a
//! parent id) so cross-tenant reads return `None`, never another tenant's
//! row.
use crate::dto::{
    AuditLogRow, BlobRow, MembershipRow, OrgRow, PrincipalKind, QuotaRow, ResourceRow, ScopeKind,
    TriggerRow, UserRow, WorkspaceRow,
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
    async fn get(&self, id: &str) -> Result<Option<UserRow>, StorageError>;
    /// Resolve an active user by (case-insensitive) email.
    async fn get_by_email(&self, email: &str) -> Result<Option<UserRow>, StorageError>;
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
    /// List active workspaces for an org.
    async fn list_for_org(&self, org_id: &str) -> Result<Vec<WorkspaceRow>, StorageError>;
    /// CAS-update a workspace row.
    async fn update(&self, row: WorkspaceRow, expected_version: u64) -> Result<(), StorageError>;
    /// Soft-delete a workspace.
    async fn soft_delete(&self, org_id: &str, id: &str) -> Result<(), StorageError>;
}

/// `org_members` + `workspace_members` aggregate.
#[async_trait::async_trait]
pub trait MembershipStore: Send + Sync + std::fmt::Debug {
    /// Add (or replace) a membership row.
    async fn upsert(&self, row: MembershipRow) -> Result<(), StorageError>;
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
    /// Remove a membership.
    async fn remove(
        &self,
        scope_kind: ScopeKind,
        scope_id: &str,
        principal_kind: PrincipalKind,
        principal_id: &str,
    ) -> Result<(), StorageError>;
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
