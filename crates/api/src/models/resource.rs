//! Resource-listing DTOs (workspace-scoped).
//!
//! `GET /api/v1/orgs/{org}/workspaces/{ws}/resources` returns the
//! persisted resource definitions for a workspace. These DTOs are the
//! non-secret summary projection of `nebula_storage`'s `ResourceEntry`
//! — the raw `config` blob is deliberately not exposed (ADR-0028 §7).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// One resource entry in [`ListResourcesResponse`].
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ResourceSummary {
    /// `res_<ULID>` identifier.
    pub id: String,
    /// Caller-chosen display name.
    pub name: String,
    /// Resource type key (e.g. `"http_pool"`, `"redis_cache"`).
    pub kind: String,
    /// Resource-type semver version (e.g. `"1.0"`).
    pub version: String,
    /// IDs of workflows that currently reference this resource.
    ///
    /// Always empty for now: the resource store does not yet track
    /// workflow attachment, so this is reported as `[]` rather than
    /// fabricated. Populated once the attachment index lands.
    pub attached_to_workflows: Vec<String>,
}

/// `GET /api/v1/orgs/{org}/workspaces/{ws}/resources` response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListResourcesResponse {
    /// Resource summaries.
    pub resources: Vec<ResourceSummary>,
}
