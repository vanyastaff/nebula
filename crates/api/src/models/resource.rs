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

/// `PUT /api/v1/orgs/{org}/workspaces/{ws}/resources/{res}` request body.
///
/// Like [`CreateResourceRequest`] there is **deliberately no
/// `workspace_id` / owner field**: the owning workspace is never taken
/// from the request body. On update the persisted row keeps the existing
/// row's `workspace_id` (always the caller's authenticated workspace,
/// verified before the write) — a PUT can therefore never re-home a
/// resource into another tenant's workspace (tenant isolation; the
/// confused-deputy abuse).
///
/// `expected_version` is the optimistic-concurrency token the client
/// read from a prior GET; the store applies a CAS on it and a mismatch
/// is reported as **409 Conflict**. `config`/`kind` are re-validated
/// against the kind's `R::Config` schema (and rejected if the config
/// carries an undeclared, secret-shaped field — ADR-0028 §7 /
/// PRODUCT_CANON §3.5) *before* the row is persisted, so a PUT can never
/// be a path to persist a config that a create would have rejected.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateResourceRequest {
    /// New human-readable display name.
    pub display_name: String,
    /// Resource kind key (e.g. `"http_pool"`). Re-validated against the
    /// closed registrar allowlist; an unknown kind is rejected.
    pub kind: String,
    /// Resource-specific, non-secret configuration. Re-validated against
    /// the kind's `R::Config` schema. Secrets must NOT be inlined here —
    /// they are bound through typed credential slots.
    #[schema(value_type = Object)]
    pub config: serde_json::Value,
    /// Version the caller expects the stored row to be at (read from a
    /// prior GET). The update is applied with a CAS on this counter; a
    /// mismatch is **409 Conflict**.
    pub expected_version: i64,
}

/// `PUT /api/v1/orgs/{org}/workspaces/{ws}/resources/{res}` response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateResourceResponse {
    /// `res_<ULID>` identifier of the updated resource.
    pub id: String,
    /// The row's new CAS version after the successful update.
    pub version: String,
}

/// `GET /api/v1/orgs/{org}/workspaces/{ws}/resources/{res}/status`
/// response — a **read-only** runtime-status projection.
///
/// Carries lifecycle phase / health only. It deliberately has **no**
/// `config` / credential / secret field (ADR-0028 §7): a status read is
/// not a config read. There is also no acquire/release/drain control on
/// this DTO or its endpoint — resource lifecycle is engine-owned and not
/// exposed over HTTP (INTEGRATION_MODEL §13.1).
///
/// A resource that exists as a definition but has never been activated in
/// the engine reports `phase = "inactive"` (with `healthy`/`accepting`
/// `false`) — it exists as config, it is just not running. This is a
/// `200`, not a `404` (the row exists; absence of a *live runtime* is a
/// well-defined status, not a missing resource).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ResourceStatusDto {
    /// `res_<ULID>` identifier of the resource the status is for.
    pub id: String,
    /// Lifecycle phase as a stable lowercase token: one of
    /// `initializing`, `ready`, `reloading`, `draining`,
    /// `shutting_down`, `failed`, `unknown`, or `inactive`
    /// (configured but no live runtime).
    pub phase: String,
    /// `true` iff the resource is in a healthy, request-serving phase
    /// (`ready`). A configured-but-inactive resource is not healthy.
    pub healthy: bool,
    /// `true` iff the resource can currently accept new acquire requests
    /// (`ready` / `reloading`). Surfaced read-only — it does not acquire.
    pub accepting: bool,
}
