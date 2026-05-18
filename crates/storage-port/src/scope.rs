//! Plain-data tenant scope.
//!
//! [`Scope`] is a value type only. Resolving a scope from a principal and
//! enforcing cross-tenant denial is policy and lives in `nebula-tenancy`.
//! Keeping the type here (Core tier) lets tenant-scoped port signatures
//! require it without an upward dependency on the policy crate.
use serde::{Deserialize, Serialize};

/// Workspace + org isolation key. Required by every tenant-scoped operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Scope {
    /// Workspace identifier.
    pub workspace_id: String,
    /// Organization identifier.
    pub org_id: String,
}

impl Scope {
    /// Build a scope from workspace + org ids.
    pub fn new(workspace_id: impl Into<String>, org_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            org_id: org_id.into(),
        }
    }
}
