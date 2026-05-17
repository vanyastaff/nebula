//! Resource-listing DTOs (workspace-scoped).
//!
//! `GET /api/v1/orgs/{org}/workspaces/{ws}/resources` is currently a 501
//! stub (audit class (c)). The DTOs below describe the **planned** payload
//! shape per ADR-0047 Stub Endpoint Policy.

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
    pub attached_to_workflows: Vec<String>,
}

/// `GET /api/v1/orgs/{org}/workspaces/{ws}/resources` response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListResourcesResponse {
    /// Resource summaries.
    pub resources: Vec<ResourceSummary>,
}
