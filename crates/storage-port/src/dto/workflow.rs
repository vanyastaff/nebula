//! Workflow + workflow-version row DTOs (spec-16 workflow/version split).
use crate::Scope;
use serde::{Deserialize, Serialize};

/// Exact identities installed when a workflow version is activated.
///
/// This immutable metadata proves no tenant authority or catalog admission.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowActivation {
    #[serde(rename = "workflow_version_id")]
    workflow_revision: nebula_core::WorkflowVersionId,
    #[serde(rename = "executable_plan_id")]
    executable_plan: nebula_core::ExecutablePlanRevisionId,
    #[serde(rename = "worker_flavor_id")]
    worker_flavor: nebula_core::WorkerFlavorRevisionId,
}

impl WorkflowActivation {
    /// Pair the activation's workflow revision with its installed exact plan and flavor.
    #[must_use]
    pub const fn new(
        workflow_version_id: nebula_core::WorkflowVersionId,
        revisions: super::PlanFlavorRevisionIds,
    ) -> Self {
        Self {
            workflow_revision: workflow_version_id,
            executable_plan: revisions.plan(),
            worker_flavor: revisions.worker_flavor(),
        }
    }

    /// Immutable workflow revision supplied to the compiler.
    #[must_use]
    pub const fn workflow_version_id(self) -> nebula_core::WorkflowVersionId {
        self.workflow_revision
    }

    /// Exact installed executable plan and frozen flavor.
    #[must_use]
    pub const fn revisions(self) -> super::PlanFlavorRevisionIds {
        super::PlanFlavorRevisionIds::new(self.executable_plan, self.worker_flavor)
    }
}

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
#[expect(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowVersionRecord {
    /// Complete activation identity, absent for drafts and legacy versions.
    #[serde(default)]
    pub activation: Option<WorkflowActivation>,
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
