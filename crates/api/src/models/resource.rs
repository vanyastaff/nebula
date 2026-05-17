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

/// `POST /api/v1/orgs/{org}/workspaces/{ws}/resources` request body.
///
/// There is **deliberately no `workspace_id` / owner field**: the owning
/// workspace is taken solely from the authenticated path context, never
/// from the request body, so a caller can never create a resource in
/// another tenant's workspace (tenant isolation; the confused-deputy
/// abuse). `config` is validated against the target `kind`'s
/// `R::Config` schema (and rejected if it carries an undeclared,
/// secret-shaped field — ADR-0028 §7 / PRODUCT_CANON §3.5) *before* the
/// row is persisted.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateResourceRequest {
    /// Workspace-unique slug for the new resource.
    pub slug: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Resource kind key (e.g. `"http_pool"`). Must be a kind the
    /// instance has registered; an unknown kind is rejected.
    pub kind: String,
    /// Resource-specific, non-secret configuration. Validated against
    /// the kind's `R::Config` schema. Secrets must NOT be inlined here —
    /// they are bound through typed credential slots.
    #[schema(value_type = Object)]
    pub config: serde_json::Value,
}

/// `POST /api/v1/orgs/{org}/workspaces/{ws}/resources` response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateResourceResponse {
    /// `res_<ULID>` identifier of the newly created resource.
    pub id: String,
}
