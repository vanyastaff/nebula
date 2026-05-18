//! In-memory identity-zoo stores.
//!
//! One `parking_lot::Mutex`-guarded map per aggregate. Tenant-scoped
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

use std::collections::HashMap;
use std::sync::Arc;

use nebula_storage_port::dto::{
    AuditLogRow, BlobRow, MembershipRow, OrgRow, QuotaRow, ResourceRow, TriggerRow, UserRow,
    WorkspaceRow,
};
use nebula_storage_port::store::{
    AuditStore, BlobStore, MembershipStore, OrgStore, QuotaStore, ResourceStore, TriggerStore,
    UserStore, WorkspaceStore,
};
use nebula_storage_port::{Scope, StorageError};
use parking_lot::Mutex;

// ── Users ─────────────────────────────────────────────────────────────────

/// In-memory `users` store. Users are global (no tenant scope); email is
/// unique among active rows (case-insensitive).
#[derive(Debug, Default, Clone)]
pub struct InMemoryUserStore {
    inner: Arc<Mutex<HashMap<String, UserRow>>>,
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
        let mut map = self.inner.lock();
        if map.contains_key(&row.id) {
            return Err(StorageError::Duplicate {
                entity: "user",
                detail: format!("user {} already exists", row.id),
            });
        }
        let email = row.email.to_ascii_lowercase();
        if map
            .values()
            .any(|u| u.deleted_at.is_none() && u.email.to_ascii_lowercase() == email)
        {
            return Err(StorageError::Duplicate {
                entity: "user",
                detail: format!("active user with email {} already exists", row.email),
            });
        }
        map.insert(row.id.clone(), row);
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<UserRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .get(id)
            .filter(|u| u.deleted_at.is_none())
            .cloned())
    }

    async fn get_by_email(&self, email: &str) -> Result<Option<UserRow>, StorageError> {
        let needle = email.to_ascii_lowercase();
        Ok(self
            .inner
            .lock()
            .values()
            .find(|u| u.deleted_at.is_none() && u.email.to_ascii_lowercase() == needle)
            .cloned())
    }

    async fn update(&self, row: UserRow, expected_version: u64) -> Result<(), StorageError> {
        let mut map = self.inner.lock();
        let Some(cur) = map.get(&row.id).filter(|u| u.deleted_at.is_none()) else {
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
        if map.values().any(|u| {
            u.id != row.id && u.deleted_at.is_none() && u.email.to_ascii_lowercase() == email
        }) {
            return Err(StorageError::Duplicate {
                entity: "user",
                detail: format!("active user with email {} already exists", row.email),
            });
        }
        map.insert(row.id.clone(), row);
        Ok(())
    }

    async fn soft_delete(&self, id: &str) -> Result<(), StorageError> {
        let mut map = self.inner.lock();
        let Some(row) = map.get_mut(id).filter(|u| u.deleted_at.is_none()) else {
            return Err(StorageError::not_found("user", id));
        };
        row.deleted_at = Some(now_rfc3339());
        Ok(())
    }
}

// ── Orgs ──────────────────────────────────────────────────────────────────

/// In-memory `orgs` store. Slug is unique among active rows.
#[derive(Debug, Default, Clone)]
pub struct InMemoryOrgStore {
    inner: Arc<Mutex<HashMap<String, OrgRow>>>,
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
        let mut map = self.inner.lock();
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
            .get(id)
            .filter(|o| o.deleted_at.is_none())
            .cloned())
    }

    async fn get_by_slug(&self, slug: &str) -> Result<Option<OrgRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .values()
            .find(|o| o.deleted_at.is_none() && o.slug == slug)
            .cloned())
    }

    async fn update(&self, row: OrgRow, expected_version: u64) -> Result<(), StorageError> {
        let mut map = self.inner.lock();
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
        let mut map = self.inner.lock();
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
    inner: Arc<Mutex<HashMap<WsKey, WorkspaceRow>>>,
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
        let mut map = self.inner.lock();
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
            .get(&(org_id.to_string(), id.to_string()))
            .filter(|w| w.deleted_at.is_none())
            .cloned())
    }

    async fn list_for_org(&self, org_id: &str) -> Result<Vec<WorkspaceRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .values()
            .filter(|w| w.deleted_at.is_none() && w.org_id == org_id)
            .cloned()
            .collect())
    }

    async fn update(&self, row: WorkspaceRow, expected_version: u64) -> Result<(), StorageError> {
        let key = (row.org_id.clone(), row.id.clone());
        let mut map = self.inner.lock();
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
        let mut map = self.inner.lock();
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
type MemKey = (String, String, String, String);

fn mem_key(scope_kind: &str, scope_id: &str, principal_kind: &str, principal_id: &str) -> MemKey {
    (
        scope_kind.to_string(),
        scope_id.to_string(),
        principal_kind.to_string(),
        principal_id.to_string(),
    )
}

/// In-memory `org_members` + `workspace_members` store.
#[derive(Debug, Default, Clone)]
pub struct InMemoryMembershipStore {
    inner: Arc<Mutex<HashMap<MemKey, MembershipRow>>>,
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
    async fn upsert(&self, row: MembershipRow) -> Result<(), StorageError> {
        let key = mem_key(
            &row.scope_kind,
            &row.scope_id,
            &row.principal_kind,
            &row.principal_id,
        );
        self.inner.lock().insert(key, row);
        Ok(())
    }

    async fn get(
        &self,
        scope_kind: &str,
        scope_id: &str,
        principal_kind: &str,
        principal_id: &str,
    ) -> Result<Option<MembershipRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .get(&mem_key(scope_kind, scope_id, principal_kind, principal_id))
            .cloned())
    }

    async fn list_for_scope(
        &self,
        scope_kind: &str,
        scope_id: &str,
    ) -> Result<Vec<MembershipRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .values()
            .filter(|m| m.scope_kind == scope_kind && m.scope_id == scope_id)
            .cloned()
            .collect())
    }

    async fn remove(
        &self,
        scope_kind: &str,
        scope_id: &str,
        principal_kind: &str,
        principal_id: &str,
    ) -> Result<(), StorageError> {
        self.inner
            .lock()
            .remove(&mem_key(scope_kind, scope_id, principal_kind, principal_id));
        Ok(())
    }
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
