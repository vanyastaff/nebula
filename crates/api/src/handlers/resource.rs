//! Resource catalog endpoint handlers (workspace-scoped).
//!
//! `GET /api/v1/orgs/{org}/workspaces/{ws}/resources`,
//! `GET .../resources/{res}` (reads), and
//! `POST .../resources` (create) are config-CRUD endpoints: they manage
//! the persisted resource *definitions* for a workspace. Resource
//! lifecycle (acquire/refresh/revoke) is owned by the engine and is
//! intentionally NOT exposed over HTTP (INTEGRATION_MODEL §13.1).
//!
//! `create_resource` validates the submitted `config` against the target
//! `kind`'s `R::Config` schema through the engine's closed `kind →
//! registrar` allowlist (`resource_registrars`) **before** persisting —
//! schema + closed-set validation only, with no live registration into a
//! `nebula_resource::Manager` (that is an engine-activation concern, not
//! a config-create one — §13.1). The owning workspace is always the
//! caller's authenticated workspace, never a request-body field, so a
//! resource can never be created in another tenant's workspace.
//!
//! The resource catalog backend is optional on [`AppState`]
//! (`resource_repo`); when it is not configured the endpoints report
//! `503 Service Unavailable`, matching the action/plugin catalog
//! convention rather than the retired ADR-0047 501 stub.
//!
//! Tenant isolation for the single-resource read: `ResourceRepo::get` is
//! looked up purely by id and is **not** workspace-scoped. The handler is
//! the isolation boundary — a resource whose `workspace_id` differs from
//! the caller's authorized workspace is reported as `404 Not Found`,
//! indistinguishable from a missing row (no cross-tenant existence or
//! content leak). A soft-deleted row and an unparsable id are likewise
//! 404.

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use nebula_core::{Principal, ResourceId, TenantContext};
use nebula_engine::RegistrarError;
use nebula_storage::repos::ResourceEntry;

use crate::{
    errors::{ApiError, ApiResult, ProblemDetails},
    models::{
        CreateResourceRequest, CreateResourceResponse, ListResourcesResponse, ResourceSummary,
    },
    state::AppState,
};

/// Project a persisted [`ResourceEntry`] onto the non-secret
/// [`ResourceSummary`] DTO.
///
/// Centralised so every resource read path produces an identical
/// projection: the raw `config` blob is never surfaced (ADR-0028 §7) and
/// the id is the prefixed `res_<ULID>` encoding. The `bytes_to_id` step is
/// fallible (a stored id that is not exactly 16 bytes is a storage
/// invariant violation), so the result is a `Result`.
fn entry_to_summary(entry: ResourceEntry) -> Result<ResourceSummary, ApiError> {
    let id = nebula_storage::mapping::bytes_to_id(&entry.id)
        .map(|bytes| ResourceId::from_bytes(bytes).to_string())
        .map_err(ApiError::from)?;
    Ok(ResourceSummary {
        id,
        name: entry.display_name,
        kind: entry.kind,
        version: entry.version.to_string(),
        // Workflow attachment is not tracked by the resource store yet;
        // advertised honestly as empty rather than fabricated.
        attached_to_workflows: Vec::new(),
    })
}

/// `GET /api/v1/orgs/{org}/workspaces/{ws}/resources` — list workspace resources.
///
/// Returns resource definitions scoped to the caller's workspace. Soft-deleted
/// rows are excluded. The raw `config` blob is never surfaced — only the
/// non-secret summary fields (ADR-0028 §7).
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/resources",
    tag = "workspaces.resources",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
    ),
    responses(
        (status = 200, description = "Resource definitions for the workspace (no raw config).", body = ListResourcesResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 500, description = "Resource repository error.", body = ProblemDetails),
        (status = 503, description = "Resource catalog backend is not configured on this instance.", body = ProblemDetails),
    ),
)]
pub async fn list_resources(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
) -> ApiResult<Json<ListResourcesResponse>> {
    // Workspace presence is guaranteed by the tenancy middleware for this
    // route; the explicit check keeps the handler honest if the route is
    // ever remounted. RBAC parity with the other workspace-scoped list
    // endpoints (`list_workflows`, `list_executions`) — no per-handler
    // permission gate under the current shared-trust JWT model.
    let workspace_id = tenant
        .require_workspace()
        .map_err(|_| ApiError::Forbidden("workspace context required".to_string()))?;

    let repo = state.resource_repo.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource catalog backend not configured".into())
    })?;

    let entries = repo.list(workspace_id.as_bytes().as_slice()).await?;

    let resources = entries
        .into_iter()
        // Soft-deleted definitions are not part of the catalog view.
        .filter(|entry| entry.deleted_at.is_none())
        .map(entry_to_summary)
        .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(Json(ListResourcesResponse { resources }))
}

/// `GET /api/v1/orgs/{org}/workspaces/{ws}/resources/{res}` — fetch one resource.
///
/// Returns a single resource definition scoped to the caller's workspace.
/// The raw `config` blob is never surfaced — only the non-secret summary
/// fields (ADR-0028 §7).
///
/// All of the following collapse to **404 Not Found**, deliberately
/// indistinguishable so no information leaks across tenants:
/// - unknown id;
/// - an unparsable id string (an id that is not a `res_<ULID>` cannot
///   name an existing resource — "not found", not a 400/500);
/// - a resource owned by a *different* workspace (`ResourceRepo::get` is
///   keyed purely by id and is not workspace-scoped, so the handler
///   enforces tenant isolation here — neither the content nor the
///   existence of another tenant's resource is revealed);
/// - a soft-deleted row (a tombstone is not a resource).
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/resources/{res}",
    tag = "workspaces.resources",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("res" = String, Path, description = "Resource identifier (`res_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Resource definition (no raw config).", body = ResourceSummary),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Resource does not exist (also returned for a resource in another workspace, a soft-deleted resource, or an unparsable id — no cross-tenant leak).", body = ProblemDetails),
        (status = 500, description = "Resource repository error.", body = ProblemDetails),
        (status = 503, description = "Resource catalog backend is not configured on this instance.", body = ProblemDetails),
    ),
)]
pub async fn get_resource(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, res)): Path<(String, String, String)>,
) -> ApiResult<Json<ResourceSummary>> {
    let workspace_id = tenant
        .require_workspace()
        .map_err(|_| ApiError::Forbidden("workspace context required".to_string()))?;
    let ws_bytes = workspace_id.as_bytes();

    let repo = state.resource_repo.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource catalog backend not configured".into())
    })?;

    // An unparsable id cannot name an existing resource. Surfacing it as
    // 404 (not 400) keeps malformed-id and cross-tenant lookups
    // indistinguishable, so probing ids never leaks which ones are
    // structurally valid.
    let resource_id = ResourceId::parse(&res)
        .map_err(|_| ApiError::NotFound(format!("Resource {res} not found")))?;

    let entry = repo
        .get(resource_id.as_bytes().as_slice())
        .await?
        .filter(|entry| {
            // Tenant isolation + soft-delete tombstone collapse to the
            // same "not found" outcome as a genuinely absent row: a
            // resource in another workspace, or a deleted one, must be
            // indistinguishable from non-existence.
            entry.workspace_id.as_slice() == ws_bytes && entry.deleted_at.is_none()
        })
        .ok_or_else(|| ApiError::NotFound(format!("Resource {res} not found")))?;

    Ok(Json(entry_to_summary(entry)?))
}

/// 16 raw id bytes for the resource's `created_by`, derived from the
/// authenticated principal.
///
/// `User` / `ServiceAccount` carry a single ULID identity → its bytes.
/// `Workflow` / `System` are not a config-author identity for a
/// human/SA-initiated create; recording 16 zero bytes is the honest
/// "no individual creator" sentinel (consistent with the unset-creator
/// rows elsewhere) rather than fabricating one or panicking.
fn created_by_bytes(principal: &Principal) -> Vec<u8> {
    match principal {
        Principal::User(uid) => uid.as_bytes().to_vec(),
        Principal::ServiceAccount(sid) => sid.as_bytes().to_vec(),
        Principal::Workflow { .. } | Principal::System => vec![0_u8; 16],
    }
}

/// `POST /api/v1/orgs/{org}/workspaces/{ws}/resources` — create a resource.
///
/// Persists a new resource *definition* after validating its `config`
/// against the target `kind`'s `R::Config` schema. The owning workspace
/// is **always** the caller's authenticated workspace — there is no
/// workspace/owner field in the request body, so a resource can never be
/// created in another tenant's workspace (the confused-deputy abuse).
///
/// Validation runs through the engine's closed `kind → registrar`
/// allowlist (schema + closed-set guard, **no** live `Manager`
/// registration — live registration is an engine-activation concern,
/// INTEGRATION_MODEL §13.1). Outcomes:
/// - unknown `kind` ⇒ **409 Conflict** (the kind is not in the closed
///   allowlist — a non-retryable caller fault, classified exactly as the
///   engine's `RegistrarError::UnknownKind`);
/// - `config` fails the kind's schema or carries an undeclared,
///   secret-shaped field ⇒ **422 Unprocessable** with a generic detail
///   (the validator's raw report is logged server-side, never echoed —
///   it could restate submitted values; ADR-0028 §7);
/// - validation backend not configured ⇒ **422** (fail closed: an
///   unvalidated config is never persisted).
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/resources",
    tag = "workspaces.resources",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
    ),
    request_body = CreateResourceRequest,
    responses(
        (status = 201, description = "Resource created; returns the new `res_<ULID>` id.", body = CreateResourceResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 409, description = "Unknown resource `kind` (not in the closed registrar allowlist).", body = ProblemDetails),
        (status = 422, description = "Resource config failed schema/closed-set validation, or the validation backend is not configured.", body = ProblemDetails),
        (status = 500, description = "Resource repository error.", body = ProblemDetails),
        (status = 503, description = "Resource catalog backend is not configured on this instance.", body = ProblemDetails),
    ),
)]
pub async fn create_resource(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws)): Path<(String, String)>,
    Json(body): Json<CreateResourceRequest>,
) -> ApiResult<(StatusCode, Json<CreateResourceResponse>)> {
    let workspace_id = tenant
        .require_workspace()
        .map_err(|_| ApiError::Forbidden("workspace context required".to_string()))?;

    let repo = state.resource_repo.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource catalog backend not configured".into())
    })?;

    // Validate the config against the target kind BEFORE persistence. If
    // the validation surface is not wired, fail closed — persisting an
    // unvalidated config (which could carry an inlined secret) would
    // violate ADR-0028 §7 / PRODUCT_CANON §3.5.
    let registrars = state.resource_registrars.as_ref().ok_or_else(|| {
        ApiError::Unprocessable(
            "resource config validation is unavailable on this instance".to_string(),
        )
    })?;

    registrars
        .validate(&body.kind, body.config.clone())
        .map_err(|err| match err {
            // Unknown kind: closed-allowlist miss caught before any typed
            // call — a non-retryable caller conflict, never a 5xx
            // (mirrors RegistrarError::UnknownKind → ErrorCategory::Conflict).
            RegistrarError::UnknownKind(kind) => {
                ApiError::Conflict(format!("unknown resource kind `{kind}`"))
            },
            // Schema / closed-set failure. The inner report can restate
            // submitted field values, so it is logged server-side and the
            // client gets a generic 422 — never the raw message (ADR-0028
            // §7). 422 (not the inner Error::permanent → Internal/500):
            // invalid client input is a client error, and the handler owns
            // the HTTP contract here rather than delegating classification.
            RegistrarError::Register { kind, source } => {
                tracing::warn!(
                    target: "nebula_api::resource",
                    kind = %kind,
                    error = %source,
                    "resource config rejected by schema/closed-set validation"
                );
                ApiError::Unprocessable(format!(
                    "resource configuration is invalid for kind `{kind}`"
                ))
            },
            // `RegistrarError` is `#[non_exhaustive]`. Any future variant
            // means validation did not conclusively succeed, so fail
            // closed with a generic 422 rather than persist an
            // unvalidated config or leak an unmapped error shape.
            other => {
                tracing::warn!(
                    target: "nebula_api::resource",
                    error = %other,
                    "resource config validation failed with an unmapped registrar error"
                );
                ApiError::Unprocessable("resource configuration is invalid".to_string())
            },
        })?;

    let resource_id = ResourceId::new();
    let entry = ResourceEntry {
        id: resource_id.as_bytes().to_vec(),
        // Tenant isolation: the owning workspace is the caller's
        // authenticated workspace, taken from the tenancy context — NEVER
        // a request-body field (the DTO deliberately has none).
        workspace_id: workspace_id.as_bytes().to_vec(),
        slug: body.slug,
        display_name: body.display_name,
        kind: body.kind,
        config: body.config,
        created_at: chrono::Utc::now(),
        created_by: created_by_bytes(&tenant.principal),
        // Initial version for a freshly created row; subsequent updates
        // use CAS on this counter.
        version: 0,
        deleted_at: None,
    };

    repo.create(&entry).await?;

    Ok((
        StatusCode::CREATED,
        Json(CreateResourceResponse {
            id: resource_id.to_string(),
        }),
    ))
}
