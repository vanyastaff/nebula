//! Workflow + workflow-version row DTOs (spec-16 workflow/version split).
use crate::Scope;
use serde::{Deserialize, Serialize};

/// One workflow row as the port exposes it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowRecord {
    /// Workflow id (opaque string form).
    pub id: String,
    /// Tenant scope this row belongs to.
    pub scope: Scope,
    /// Optimistic-CAS version.
    pub version: u64,
    /// Author-defined slug (unique per workspace among active rows).
    pub slug: String,
    /// Soft-delete marker.
    pub deleted: bool,
}

/// One workflow-version row.
///
/// `definition` is opaque to the port (the workflow compiler owns its
/// shape). `pinned` prevents automatic version GC.
// guard-justified: `definition` is `serde_json::Value`, which is not
// `Eq` (it can hold a float). `Eq` is therefore not derivable; the
// clippy hint is a false positive for any DTO carrying an opaque JSON
// payload.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowVersionRecord {
    /// Owning workflow id (opaque string form).
    pub workflow_id: String,
    /// Monotone version number within the workflow.
    pub number: u32,
    /// Whether this version is the published one.
    pub published: bool,
    /// Whether this version is pinned (excluded from version GC).
    pub pinned: bool,
    /// Opaque workflow definition payload.
    pub definition: serde_json::Value,
}
