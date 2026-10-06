//! In-memory identity-zoo stores — one file per aggregate.
//!
//! The tenant directory (orgs, workspaces, grants, provisioning) shares
//! one `parking_lot::Mutex` so parent checks, membership reads and guarded
//! mutations observe one logical snapshot; other aggregates own independent
//! maps. Tenant-scoped lookups fold the parent id (org / workspace) or
//! `Scope` into the map key, so a cross-tenant `get` returns `Ok(None)`
//! exactly as the SQL backends' `WHERE … = ?` predicate would — an id
//! outside the caller's scope is indistinguishable from one that does not
//! exist (no existence oracle, spec §6.1).
//!
//! Soft-delete is modelled by stamping `deleted_at`: a soft-deleted row
//! stays in the map but is filtered out of every read path, mirroring the
//! SQL `WHERE deleted_at IS NULL` predicate. First-writer-wins uniqueness
//! (email / slug among *active* rows) and optimistic CAS (`version`) match
//! the relational contract the conformance matrix asserts. Error variants
//! match the SQL backends; messages never carry stored values.

mod directory;
mod membership;
mod org;
mod resource;
mod trigger;
mod workspace;

pub use directory::InMemoryIdentityDirectory;
pub use membership::InMemoryMembershipStore;
pub use org::InMemoryOrgStore;
pub use resource::InMemoryResourceStore;
pub use trigger::InMemoryTriggerStore;
pub use workspace::InMemoryWorkspaceStore;

use nebula_storage_port::{Scope, StorageError};

/// Current time as an RFC 3339 string — the soft-delete stamp of the
/// aggregates that still store instants as text (resources, triggers).
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Workspace-scoped key: `(workspace_id, org_id, id)`.
type ScopedKey = (String, String, String);

fn scoped_key(scope: &Scope, id: &str) -> ScopedKey {
    (
        scope.workspace_id.clone(),
        scope.org_id.clone(),
        id.to_owned(),
    )
}

fn in_scope(key: &ScopedKey, scope: &Scope) -> bool {
    key.0 == scope.workspace_id && key.1 == scope.org_id
}

/// `Conflict` for a CAS whose expected version did not match.
fn version_conflict(entity: &'static str, id: String, expected: u64, actual: u64) -> StorageError {
    StorageError::Conflict {
        entity,
        id,
        expected,
        actual,
    }
}

/// `Duplicate` naming only the entity and the colliding field — never its
/// value.
fn duplicate(entity: &'static str, field: &str) -> StorageError {
    StorageError::Duplicate {
        entity,
        detail: format!("an active {entity} already has this {field}"),
    }
}
