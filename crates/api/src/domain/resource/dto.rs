//! Resource-listing DTOs (workspace-scoped).
//!
//! `GET /api/v1/orgs/{org}/workspaces/{ws}/resources` returns the
//! persisted resource definitions for a workspace. These DTOs are the
//! non-secret summary projection of [`nebula_storage_port::dto::ResourceRow`].
//! Raw config and credential bindings are deliberately not exposed.

use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// One resource entry in [`ListResourcesResponse`].
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ResourceSummary {
    /// `res_<ULID>` identifier.
    pub id: String,
    /// Stable, workspace-unique, non-secret slug. Assigned at create and
    /// immutable thereafter (a PUT preserves it), so it is a durable
    /// client-side handle for the resource within its workspace.
    #[schema(example = "primary-http-pool")]
    pub slug: String,
    /// Caller-chosen display name.
    pub name: String,
    /// Resource type key (e.g. `"http_pool"`, `"redis_cache"`).
    pub kind: String,
    /// Monotonic CAS row version (the optimistic-concurrency token; a
    /// caller echoes it back as `expected_version` on a PUT). Non-secret
    /// concurrency metadata, not a resource-type semver.
    pub version: i64,
    /// Operator topology settings (see [`CreateResourceRequest::topology`]);
    /// `null` uses the kind's defaults. Non-secret: validated numbers and
    /// modes only.
    #[schema(value_type = Option<Object>)]
    pub topology: Option<serde_json::Value>,
    /// Operator resilience override (see
    /// [`CreateResourceRequest::resilience_override`]); `null` enforces the
    /// kind's declared policy.
    #[schema(value_type = Option<Object>)]
    pub resilience_override: Option<serde_json::Value>,
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
/// secret-shaped field — no secret echo / product credential boundary) *before* the
/// row is persisted.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
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
    /// Credential selectors keyed by the resource kind's declared slot name.
    /// Values are persisted for credential-runtime resolution and are never
    /// included in resource read responses or debug output.
    #[serde(default)]
    pub credential_bindings: BTreeMap<String, String>,
    /// Operator topology settings — pool size, timeouts, concurrency mode —
    /// in the shape the kind's topology accepts. Validated before the row is
    /// stored; omitted or `null` uses the kind's defaults.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub topology: Option<serde_json::Value>,
    /// Override of the resilience the kind declares:
    /// `{"rate": {"requests": 30, "period_ms": 1000, "burst": 5}}`. Only
    /// what the kind's policy allows is accepted (by default a slower rate
    /// than declared). Omitted or `null` enforces the declared policy.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub resilience_override: Option<serde_json::Value>,
}

impl fmt::Debug for CreateResourceRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateResourceRequest")
            .field("slug", &self.slug)
            .field("display_name", &self.display_name)
            .field("kind", &self.kind)
            .finish()
    }
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
/// carries an undeclared, secret-shaped field — no secret echo /
/// product credential boundary) *before* the row is persisted, so a PUT can never
/// be a path to persist a config that a create would have rejected.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
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
    /// Replacement credential selectors keyed by declared slot name.
    /// Omission clears all bindings, matching full-replacement `PUT`
    /// semantics. Values never appear in resource responses or debug output.
    #[serde(default)]
    pub credential_bindings: BTreeMap<String, String>,
    /// Replacement topology settings, validated as on create. Omission
    /// resets to the kind's defaults, matching full-replacement `PUT`
    /// semantics.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub topology: Option<serde_json::Value>,
    /// Replacement resilience override, validated as on create. Omission
    /// restores the kind's declared policy, matching full-replacement `PUT`
    /// semantics.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub resilience_override: Option<serde_json::Value>,
    /// Version the caller expects the stored row to be at (read from a
    /// prior GET). The update is applied with a CAS on this counter; a
    /// mismatch is **409 Conflict**.
    pub expected_version: i64,
}

impl fmt::Debug for UpdateResourceRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdateResourceRequest")
            .field("display_name", &self.display_name)
            .field("kind", &self.kind)
            .field("expected_version", &self.expected_version)
            .finish()
    }
}

/// `PUT /api/v1/orgs/{org}/workspaces/{ws}/resources/{res}` response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateResourceResponse {
    /// `res_<ULID>` identifier of the updated resource.
    pub id: String,
    /// The row's new CAS version after the successful update.
    ///
    /// The successful store CAS advances this counter by one.
    pub version: i64,
}

/// Closed runtime-phase vocabulary for [`ResourceStatusDto::phase`].
///
/// Serialised as a stable lowercase token. The first seven mirror the
/// engine's read-only status seam projection; `Inactive` is the
/// API-side phase for a resource that exists as a definition but has no
/// live runtime (configured, never activated). `Unknown` is the
/// fail-safe for an unrecognised future engine phase — never a panic or
/// a guessed label (the engine seam is `#[non_exhaustive]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResourcePhase {
    /// Runtime is being constructed.
    Initializing,
    /// Healthy and serving requests.
    Ready,
    /// Reloading (e.g. blue-green); may still accept.
    Reloading,
    /// Draining in-flight work; not accepting new acquires.
    Draining,
    /// Shutting down.
    ShuttingDown,
    /// Entered a failed state.
    Failed,
    /// Unrecognised future engine phase (fail-safe, not fabricated).
    Unknown,
    /// Configured as a definition but no live runtime exists yet.
    Inactive,
}

/// `GET /api/v1/orgs/{org}/workspaces/{ws}/resources/{res}/status`
/// response — a **read-only** runtime-status projection.
///
/// Carries lifecycle phase / health only. It deliberately has **no**
/// `config` / credential / secret field (no secret echo): a status read is
/// not a config read. There is also no acquire/release/drain control on
/// this DTO or its endpoint — resource lifecycle is engine-owned and not
/// exposed over HTTP (INTEGRATION_MODEL integration seam.1).
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
    /// Lifecycle phase, drawn from the closed [`ResourcePhase`]
    /// vocabulary (serialised as a stable lowercase token).
    pub phase: ResourcePhase,
    /// `true` iff the resource is in a healthy, request-serving phase
    /// (`ready`). A configured-but-inactive resource is not healthy.
    pub healthy: bool,
    /// `true` iff the resource can currently accept new acquire requests
    /// (`ready` / `reloading`). Surfaced read-only — it does not acquire.
    pub accepting: bool,
    /// Number of live worker processes currently serving the resource.
    /// With several, `phase` is the least healthy one reported, `healthy`
    /// requires all of them healthy and `accepting` any of them accepting.
    /// `0` for an inactive resource.
    pub instances: u32,
}

#[cfg(test)]
mod tests {
    use super::{CreateResourceRequest, UpdateResourceRequest};

    #[test]
    fn create_request_defaults_credential_bindings_and_redacts_debug() {
        let request: CreateResourceRequest = serde_json::from_value(serde_json::json!({
            "slug": "primary-http",
            "display_name": "Primary HTTP",
            "kind": "http_pool",
            "config": { "api_token": "config-secret" }
        }))
        .expect("create request without bindings must deserialize");

        assert!(request.credential_bindings.is_empty());
        let debug = format!("{request:?}");
        assert!(!debug.contains("config-secret"));
        assert!(!debug.contains("config"));
        assert!(!debug.contains("credential_bindings"));
    }

    #[test]
    fn update_request_debug_does_not_expose_binding_selectors() {
        let request: UpdateResourceRequest = serde_json::from_value(serde_json::json!({
            "display_name": "Primary HTTP",
            "kind": "http_pool",
            "config": { "api_token": "config-secret" },
            "credential_bindings": { "api_token": "credential-selector" },
            "expected_version": 4
        }))
        .expect("update request with bindings must deserialize");

        assert_eq!(
            request
                .credential_bindings
                .get("api_token")
                .map(String::as_str),
            Some("credential-selector")
        );
        let debug = format!("{request:?}");
        assert!(!debug.contains("config-secret"));
        assert!(!debug.contains("credential-selector"));
        assert!(!debug.contains("config"));
        assert!(!debug.contains("credential_bindings"));
    }
}
