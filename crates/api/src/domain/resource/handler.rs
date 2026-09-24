//! Resource catalog endpoint handlers (workspace-scoped).
//!
//! `GET /api/v1/orgs/{org}/workspaces/{ws}/resources`,
//! `GET .../resources/{res}` (reads), and
//! `POST .../resources` (create) are config-CRUD endpoints: they manage
//! the persisted resource *definitions* for a workspace. Resource
//! lifecycle (acquire/refresh/revoke) is owned by the engine and is
//! intentionally NOT exposed over HTTP (INTEGRATION_MODEL integration seam.1).
//!
//! `create_resource` validates the submitted `config` against the target
//! `kind`'s `R::Config` schema through the engine's closed `kind →
//! registrar` allowlist (`resource_registrars`) **before** persisting —
//! schema + closed-set validation only, with no live registration into a
//! `nebula_resource::Manager` (that is an engine-activation concern, not
//! a config-create one — integration seam.1). The owning workspace is always the
//! caller's authenticated workspace, never a request-body field, so a
//! resource can never be created in another tenant's workspace.
//!
//! The resource catalog backend is optional on [`AppState`]
//! (`resource_store`); when it is not configured the endpoints report
//! `503 Service Unavailable`, matching the action/plugin catalog
//! convention rather than the retired stub-endpoint policy 501 stub.
//!
//! Every persistence call receives a [`nebula_storage_port::Scope`] derived
//! solely from the authenticated tenant context. Missing, cross-tenant,
//! soft-deleted, and malformed by-id reads remain indistinguishable 404s.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use nebula_core::{Principal, ResourceId, TenantContext};
use nebula_engine::RegistrarError;
use nebula_storage_port::{Scope, dto::ResourceRow, store::ResourceStore};

use crate::{
    domain::{
        resource::dto::{
            CreateResourceRequest, CreateResourceResponse, ListResourcesResponse, ResourcePhase,
            ResourceStatusDto, ResourceSummary, UpdateResourceRequest, UpdateResourceResponse,
        },
        shared::PaginationParams,
    },
    error::{ApiError, ApiResult, ProblemDetails},
    state::AppState,
};

/// Project a persisted [`ResourceRow`] onto the non-secret
/// [`ResourceSummary`] DTO.
///
/// Centralised so every resource read path produces an identical
/// projection: the raw `config` blob is never surfaced (no secret echo) and
/// the id is the prefixed `res_<ULID>` encoding. The `bytes_to_id` step is
/// fallible (a stored id that is not exactly 16 bytes is a storage
/// invariant violation), so the result is a `Result`.
fn row_to_summary(row: ResourceRow) -> Result<ResourceSummary, ApiError> {
    let id = ResourceId::parse(&row.id).map_err(|_| {
        tracing::error!(target: "nebula_api::resource", "stored resource id is invalid");
        ApiError::Internal("resource has an invalid stored id".to_string())
    })?;
    let version = i64::try_from(row.version).map_err(|_| {
        tracing::error!(target: "nebula_api::resource", "stored resource version exceeds API range");
        ApiError::Internal("resource has an invalid stored version".to_string())
    })?;
    Ok(ResourceSummary {
        id: id.to_string(),
        slug: row.slug,
        name: row.display_name,
        kind: row.kind,
        version,
        topology: row.topology,
        resilience_override: row.resilience_override,
        // Workflow attachment is not tracked by the resource store yet;
        // advertised honestly as empty rather than fabricated.
        attached_to_workflows: Vec::new(),
    })
}

/// Fetch a resource by its `res_<ULID>` path id, enforcing tenant isolation.
///
/// The store lookup is scope-bound. This helper also verifies the returned
/// row's workspace as defence in depth against a faulty adapter. A resource
/// in another workspace, a tombstone, an unknown id, and an unparsable id all
/// collapse to the same [`ApiError::NotFound`]. Every by-id handler goes
/// through this single audited boundary.
///
/// The order is load-bearing: parse first (an unparsable id is a 404 with
/// no backend touch), then the single fetch (a genuine backend fault
/// propagates as a `?`-mapped 500, *not* a 404), then the
/// caller-workspace + live predicate (foreign / tombstoned ⇒ 404). It is
/// applied **before any mutation** in the update/delete paths, so a caller
/// authorized for one workspace can neither mutate nor learn of another's
/// resource.
async fn fetch_owned_resource(
    store: &dyn ResourceStore,
    scope: &Scope,
    res: &str,
) -> Result<ResourceRow, ApiError> {
    let resource_id = ResourceId::parse(res)
        .map_err(|_| ApiError::NotFound(format!("Resource {res} not found")))?;
    match store.get(scope, &resource_id.to_string()).await {
        Ok(Some(row)) if row.workspace_id == scope.workspace_id && row.deleted_at.is_none() => {
            Ok(row)
        },
        Ok(_) | Err(nebula_storage_port::StorageError::ScopeViolation { .. }) => {
            Err(ApiError::NotFound(format!("Resource {res} not found")))
        },
        Err(error) => Err(error.into()),
    }
}

/// Canonical `res_<ULID>` echo for a path id already proven to exist by
/// [`fetch_owned_resource`]. The parse here is on the success path — the
/// only reachable error is an unparsable id, which [`fetch_owned_resource`]
/// already mapped to 404, so this `?` is effectively unreachable. Single
/// definition so the update/status echo paths cannot drift.
fn canonical_res_id(res: &str) -> Result<String, ApiError> {
    Ok(ResourceId::parse(res)
        .map_err(|_| ApiError::NotFound(format!("Resource {res} not found")))?
        .to_string())
}

/// Map the engine status seam's lowercase phase token onto the closed
/// [`ResourcePhase`] vocabulary.
///
/// The seam (`nebula_engine`'s read-only status projection) emits a
/// fixed set of `&'static str` tokens. Any token outside that set is an
/// unrecognised future engine phase: it maps to
/// [`ResourcePhase::Unknown`] — the same fail-safe the seam itself uses
/// for a `#[non_exhaustive]` phase — never a panic or a guessed label.
/// `inactive` is intentionally NOT produced here; it is the handler's
/// no-live-runtime phase, set on the `None` arm only.
fn phase_from_seam(phase: &str) -> ResourcePhase {
    match phase {
        "initializing" => ResourcePhase::Initializing,
        "ready" => ResourcePhase::Ready,
        "reloading" => ResourcePhase::Reloading,
        "draining" => ResourcePhase::Draining,
        "shutting_down" => ResourcePhase::ShuttingDown,
        "failed" => ResourcePhase::Failed,
        _ => ResourcePhase::Unknown,
    }
}

/// `GET /api/v1/orgs/{org}/workspaces/{ws}/resources` — list workspace resources.
///
/// Returns resource definitions scoped to the caller's workspace,
/// paginated with the shared `page`/`page_size` query convention (same
/// extractor as `list_workflows`). The store returns the complete live
/// scoped set and the handler applies the requested page window. Raw config
/// and credential bindings are never surfaced.
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/resources",
    tag = "workspaces.resources",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        PaginationParams,
    ),
    responses(
        (status = 200, description = "Resource definitions for the workspace (no raw config).", body = ListResourcesResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 500, description = "Resource store error.", body = ProblemDetails),
        (status = 503, description = "Resource catalog backend is not configured on this instance.", body = ProblemDetails),
    ),
)]
pub async fn list_resources(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ListResourcesResponse>> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let store = state.resource_store.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource catalog backend not configured".into())
    })?;

    let rows = store.list(&scope).await?;
    let resources = rows
        .into_iter()
        .skip(params.offset())
        .take(params.limit())
        .map(row_to_summary)
        .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(Json(ListResourcesResponse { resources }))
}

/// `GET /api/v1/orgs/{org}/workspaces/{ws}/resources/{res}` — fetch one resource.
///
/// Returns a single resource definition scoped to the caller's workspace.
/// The raw `config` blob is never surfaced — only the non-secret summary
/// fields (no secret echo).
///
/// All of the following collapse to **404 Not Found**, deliberately
/// indistinguishable so no information leaks across tenants:
/// - unknown id;
/// - an unparsable id string (an id that is not a `res_<ULID>` cannot
///   name an existing resource — "not found", not a 400/500);
/// - a resource owned by a *different* workspace (the lookup is scoped by
///   the authenticated tenant, so neither content nor existence leaks);
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
        (status = 500, description = "Resource store error.", body = ProblemDetails),
        (status = 503, description = "Resource catalog backend is not configured on this instance.", body = ProblemDetails),
    ),
)]
pub async fn get_resource(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, res)): Path<(String, String, String)>,
) -> ApiResult<Json<ResourceSummary>> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let store = state.resource_store.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource catalog backend not configured".into())
    })?;

    // Single audited by-id tenant boundary: an unknown / unparsable id, a
    // foreign-workspace row, and a soft-deleted tombstone all collapse to
    // an indistinguishable 404 (no cross-tenant existence/content/
    // structural oracle).
    let row = fetch_owned_resource(&**store, &scope, &res).await?;

    Ok(Json(row_to_summary(row)?))
}

/// Validate `config` against `kind` through the engine's closed
/// `kind → registrar` allowlist (schema + closed-set guard, **no** live
/// `Manager` registration — that is an engine-activation concern,
/// INTEGRATION_MODEL integration seam.1).
///
/// Shared by `create_resource` and `update_resource` so a PUT can never
/// be a path to persist a config that a POST would have rejected. The
/// outcomes are identical and fail **closed**:
/// - validation backend not configured ⇒ **422** (an unvalidated config
///   — which could carry an inlined secret — is never persisted);
/// - unknown `kind` ⇒ **409 Conflict** (closed-allowlist miss; a
///   non-retryable caller fault, classified exactly as
///   `RegistrarError::UnknownKind`);
/// - schema / closed-set failure ⇒ **422** with a generic detail. The
///   validator's raw report can restate submitted field values, so it is
///   logged server-side and never echoed to the client (no secret echo).
fn validate_resource_config(
    state: &AppState,
    kind: &str,
    config: serde_json::Value,
) -> Result<(), ApiError> {
    let registrars = state.resource_registrars.as_ref().ok_or_else(|| {
        ApiError::Unprocessable(
            "resource config validation is unavailable on this instance".to_string(),
        )
    })?;

    registrars.validate(kind, config).map_err(|err| match err {
        RegistrarError::UnknownKind(kind) => {
            ApiError::Conflict(format!("unknown resource kind `{kind}`"))
        },
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
        // means validation did not conclusively succeed, so fail closed
        // with a generic 422 rather than persist an unvalidated config or
        // leak an unmapped error shape.
        other => {
            tracing::warn!(
                target: "nebula_api::resource",
                error = %other,
                "resource config validation failed with an unmapped registrar error"
            );
            ApiError::Unprocessable("resource configuration is invalid".to_string())
        },
    })
}

/// Validate the operator settings — `topology` and `resilience_override` —
/// against `kind` before persistence, with the same fail-closed rules as
/// [`validate_resource_config`].
///
/// Unlike a config report, these messages are authored by the resource
/// runtime to name the field and the rule without restating submitted
/// values (the parsers' own reports stay server-side as the error source),
/// so the 422 carries them: an operator learns *which* setting the kind
/// refused and why.
fn validate_operator_settings(
    state: &AppState,
    kind: &str,
    topology: Option<&serde_json::Value>,
    resilience_override: Option<&serde_json::Value>,
) -> Result<(), ApiError> {
    let registrars = state.resource_registrars.as_ref().ok_or_else(|| {
        ApiError::Unprocessable(
            "resource settings validation is unavailable on this instance".to_string(),
        )
    })?;
    registrars
        .validate_topology(kind, topology)
        .and_then(|()| registrars.validate_resilience_override(kind, resilience_override))
        .map_err(|err| match err {
            RegistrarError::UnknownKind(kind) => {
                ApiError::Conflict(format!("unknown resource kind `{kind}`"))
            },
            RegistrarError::Register { kind, source } => {
                tracing::debug!(
                    target: "nebula_api::resource",
                    kind = %kind,
                    error = ?source,
                    "resource operator settings rejected"
                );
                ApiError::Unprocessable(source.to_string())
            },
            other => {
                tracing::warn!(
                    target: "nebula_api::resource",
                    error = %other,
                    "resource settings validation failed with an unmapped registrar error"
                );
                ApiError::Unprocessable("resource settings are invalid".to_string())
            },
        })
}

/// Map a [`nebula_storage_port::StorageError`] from a resource CAS `update`
/// onto the HTTP contract.
///
/// Only the **specific** `StorageError::Conflict` variant (the
/// optimistic-concurrency / CAS-version mismatch) becomes
/// [`ApiError::Conflict`] (409): a caller's `expected_version` was stale.
/// Every other `StorageError` stays the opaque
/// [`ApiError::Storage`] → 500 (no internal detail leak). This is
/// deliberately *not* a catch-all "any storage error ⇒ 409": a genuine
/// backend fault (connection, serialization, …) must not be mis-signalled
/// to the client as a version conflict, and a stale-version write must
/// not be mis-signalled as a 500.
#[must_use]
pub fn map_resource_update_storage_error(err: nebula_storage_port::StorageError) -> ApiError {
    match err {
        nebula_storage_port::StorageError::Conflict {
            expected, actual, ..
        } => {
            // The version numbers are non-secret optimistic-concurrency
            // metadata (not config/secret material), so echoing them is a
            // useful, safe CAS diagnostic per no secret echo.
            ApiError::Conflict(format!(
                "resource was modified concurrently (expected version {expected}, found {actual}); \
                 re-read and retry"
            ))
        },
        // Connection / serialization / timeout / not-found-on-update / …
        // are not a version conflict — keep the opaque 500 mapping.
        other => other.into(),
    }
}

/// Map a [`nebula_storage_port::StorageError`] from a resource `create` onto
/// the HTTP contract.
///
/// A workspace `slug` is unique per workspace, so a colliding create is a
/// **caller** conflict, not a server fault: the store surfaces it as
/// `StorageError::Duplicate` (the unique-constraint variant — slug /
/// idempotency / dedup). That, and the CAS `StorageError::Conflict`
/// variant for symmetry, become [`ApiError::Conflict`] (409). Every
/// other `StorageError` (connection / serialization / timeout / …) stays
/// the opaque [`ApiError::Storage`] → 500 — a genuine backend fault must
/// not be mis-signalled to the client as a duplicate. Mirrors
/// [`map_resource_update_storage_error`]'s deliberately-narrow shape (no
/// catch-all "any storage error ⇒ 409").
#[must_use]
pub fn map_resource_create_storage_error(err: nebula_storage_port::StorageError) -> ApiError {
    match err {
        // Unique-constraint violation — a duplicate workspace slug is the
        // expected case here. The `detail` is store-authored (constraint
        // name / generic text), not the submitted config, so it carries
        // no secret material (no secret echo).
        nebula_storage_port::StorageError::Duplicate { detail, .. } => {
            ApiError::Conflict(format!("resource already exists: {detail}"))
        },
        // CAS-mismatch cannot arise on an initial create, but map it to a
        // 409 for symmetry rather than letting it fall through to a 500.
        nebula_storage_port::StorageError::Conflict { .. } => ApiError::Conflict(
            "resource conflicts with an existing row; re-read and retry".to_string(),
        ),
        // Connection / serialization / timeout / … are not a caller
        // conflict — keep the opaque 500 mapping.
        other => other.into(),
    }
}

/// Stable audit identity for the authenticated creator.
///
/// Human and service-account principals retain their typed identifier.
/// Workflow and system actors use distinct class sentinels because this
/// field records the author class rather than workflow execution identity.
fn created_by(principal: &Principal) -> String {
    match principal {
        Principal::User(user_id) => user_id.to_string(),
        Principal::ServiceAccount(service_account_id) => service_account_id.to_string(),
        Principal::Workflow { .. } => "workflow".to_owned(),
        Principal::System => "system".to_owned(),
        // Non-exhaustive: future principal kinds default to the system sentinel.
        _ => "system".to_owned(),
    }
}

fn new_resource_row(
    resource_id: ResourceId,
    scope: &Scope,
    principal: &Principal,
    body: CreateResourceRequest,
) -> ResourceRow {
    ResourceRow {
        id: resource_id.to_string(),
        workspace_id: scope.workspace_id.clone(),
        slug: body.slug,
        display_name: body.display_name,
        kind: body.kind,
        config: body.config,
        credential_bindings: body.credential_bindings,
        topology: non_null(body.topology),
        resilience_override: non_null(body.resilience_override),
        created_at: chrono::Utc::now().to_rfc3339(),
        created_by: created_by(principal),
        version: 0,
        deleted_at: None,
    }
}

/// An explicit JSON `null` setting is stored as absent, so a row has one
/// representation of "the kind's defaults".
fn non_null(value: Option<serde_json::Value>) -> Option<serde_json::Value> {
    value.filter(|value| !value.is_null())
}

fn replacement_resource_row(
    existing: ResourceRow,
    body: UpdateResourceRequest,
    next_version: u64,
) -> ResourceRow {
    ResourceRow {
        id: existing.id,
        workspace_id: existing.workspace_id,
        slug: existing.slug,
        display_name: body.display_name,
        kind: body.kind,
        config: body.config,
        credential_bindings: body.credential_bindings,
        topology: non_null(body.topology),
        resilience_override: non_null(body.resilience_override),
        created_at: existing.created_at,
        created_by: existing.created_by,
        version: next_version,
        deleted_at: None,
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
/// INTEGRATION_MODEL integration seam.1). Outcomes:
/// - unknown `kind` ⇒ **409 Conflict** (the kind is not in the closed
///   allowlist — a non-retryable caller fault, classified exactly as the
///   engine's `RegistrarError::UnknownKind`);
/// - the workspace `slug` collides with an existing resource ⇒ **409
///   Conflict** (a workspace-unique slug — a caller conflict mapped from
///   the store's `StorageError::Duplicate`, not a 500);
/// - `config` fails the kind's schema or carries an undeclared,
///   secret-shaped field ⇒ **422 Unprocessable** with a generic detail
///   (the validator's raw report is logged server-side, never echoed —
///   it could restate submitted values; no secret echo);
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
        (status = 409, description = "Unknown resource `kind` (not in the closed registrar allowlist), or the workspace `slug` collides with an existing resource.", body = ProblemDetails),
        (status = 422, description = "Resource config failed schema/closed-set validation, or the validation backend is not configured.", body = ProblemDetails),
        (status = 500, description = "Resource store error.", body = ProblemDetails),
        (status = 503, description = "Resource catalog backend is not configured on this instance.", body = ProblemDetails),
    ),
)]
pub async fn create_resource(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws)): Path<(String, String)>,
    Json(body): Json<CreateResourceRequest>,
) -> ApiResult<(StatusCode, Json<CreateResourceResponse>)> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let store = state.resource_store.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource catalog backend not configured".into())
    })?;

    // Validate the config against the target kind BEFORE persistence. If
    // the validation surface is not wired, fail closed — persisting an
    // unvalidated config (which could carry an inlined secret) would
    // violate no secret echo / product credential boundary.
    validate_resource_config(&state, &body.kind, body.config.clone())?;
    validate_operator_settings(
        &state,
        &body.kind,
        body.topology.as_ref(),
        body.resilience_override.as_ref(),
    )?;

    let resource_id = ResourceId::new();
    let row = new_resource_row(resource_id, &scope, &tenant.principal, body);

    // A colliding workspace slug is a caller conflict (409), not a
    // server fault (500): the store surfaces it as
    // `StorageError::Duplicate`. Map it specifically — the narrow shape
    // mirrors the update path's CAS mapper.
    store
        .create(&scope, row)
        .await
        .map_err(map_resource_create_storage_error)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateResourceResponse {
            id: resource_id.to_string(),
        }),
    ))
}

/// `PUT /api/v1/orgs/{org}/workspaces/{ws}/resources/{res}` — replace a
/// resource definition (optimistic-concurrency / CAS).
///
/// Updates the persisted `display_name`/`kind`/`config` of an existing
/// resource. The `config`/`kind` are **re-validated** through the
/// engine's closed `kind → registrar` allowlist *before* the row is
/// persisted (identical schema + closed-set rules as create) — a PUT can
/// never be a path to persist a schema-invalid or unknown-kind config
/// that create would reject.
///
/// Tenant isolation is enforced by passing the authenticated scope to both
/// the read and update. All of the following collapse to
/// an indistinguishable **404 Not Found** *before any mutation*, so a
/// caller authorized for one workspace can neither mutate nor learn of a
/// resource owned by another:
/// - unknown id, or an unparsable id string;
/// - a resource owned by a *different* workspace;
/// - a soft-deleted row (a tombstone is not a resource).
///
/// The persisted row keeps the fetched row's `workspace_id` / `id` /
/// `created_*` (the request body deliberately has no workspace/owner
/// field), so an update can never re-home a resource into another
/// tenant's workspace. Other outcomes:
/// - the caller's `expected_version` is stale ⇒ **409 Conflict** (the
///   storage CAS-mismatch error mapped specifically — not a catch-all);
/// - unknown `kind` ⇒ **409**; schema/closed-set failure or no
///   validation backend ⇒ **422** (fail closed; the validator's raw
///   report is logged server-side, never echoed — no secret echo).
#[utoipa::path(
    put,
    path = "/orgs/{org}/workspaces/{ws}/resources/{res}",
    tag = "workspaces.resources",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("res" = String, Path, description = "Resource identifier (`res_<ULID>`)."),
    ),
    request_body = UpdateResourceRequest,
    responses(
        (status = 200, description = "Resource updated. `version` is the authoritative store-assigned post-CAS counter returned by the storage backend.", body = UpdateResourceResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Resource does not exist (also returned for a resource in another workspace, a soft-deleted resource, or an unparsable id — no cross-tenant leak).", body = ProblemDetails),
        (status = 409, description = "Unknown resource `kind`, or the supplied `expected_version` is stale (optimistic-concurrency conflict).", body = ProblemDetails),
        (status = 422, description = "Resource config failed schema/closed-set validation, or the validation backend is not configured.", body = ProblemDetails),
        (status = 500, description = "Resource repository error.", body = ProblemDetails),
        (status = 503, description = "Resource catalog backend is not configured on this instance.", body = ProblemDetails),
    ),
)]
pub async fn update_resource(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, res)): Path<(String, String, String)>,
    Json(body): Json<UpdateResourceRequest>,
) -> ApiResult<Json<UpdateResourceResponse>> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let store = state.resource_store.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource catalog backend not configured".into())
    })?;

    // Tenant-isolation boundary *before any mutation*: the single audited
    // by-id fetch. A foreign / missing
    // / soft-deleted / unparsable target collapses to the same 404 — a
    // caller authorized for one workspace can neither mutate nor learn of
    // another workspace's resource (no 200/403/409, no existence/content
    // leak).
    let existing = fetch_owned_resource(&**store, &scope, &res).await?;

    // Re-validate kind + config BEFORE persistence (same fail-closed
    // rules as create). A PUT must not bypass create-time validation.
    validate_resource_config(&state, &body.kind, body.config.clone())?;
    validate_operator_settings(
        &state,
        &body.kind,
        body.topology.as_ref(),
        body.resilience_override.as_ref(),
    )?;

    // The CAS contract increments the stored counter on a successful
    // compare-and-swap against `expected_version`. A *saturating* add
    // would silently pin at `i64::MAX` and defeat CAS (every subsequent
    // update would compare-and-swap against the same ceiling), so a
    // would-overflow `expected_version` is rejected as a conflict
    // instead — re-read and retry. (`i64::MAX` row versions are not
    // reachable in practice; this is a correctness guard, not a hot
    // path.)
    let expected_version = u64::try_from(body.expected_version).map_err(|_| {
        ApiError::Conflict("resource version must be non-negative; re-read and retry".to_string())
    })?;
    let next_version = expected_version.checked_add(1).ok_or_else(|| {
        ApiError::Conflict("resource version is at its maximum; re-read and retry".to_string())
    })?;

    // Build the row to persist from the *fetched* row: `id`,
    // `workspace_id` (== caller ws, just verified — NEVER from the body;
    // the DTO has no such field), `created_at`/`created_by` are
    // immutable. Only the caller-mutable fields are taken from the
    // request. `slug` is workspace-unique and not part of the update
    // contract, so the existing slug is preserved.
    let updated = replacement_resource_row(existing, body, next_version);

    store
        .update(&scope, updated, expected_version)
        .await
        .map_err(map_resource_update_storage_error)?;
    let new_version = i64::try_from(next_version)
        .map_err(|_| ApiError::Internal("resource version exceeds the API range".to_string()))?;

    // Canonical `res_<ULID>` echo of the (already isolation-verified) path
    // id — single shared definition; see `canonical_res_id`.
    let id = canonical_res_id(&res)?;

    Ok(Json(UpdateResourceResponse {
        id,
        version: new_version,
    }))
}

/// `DELETE /api/v1/orgs/{org}/workspaces/{ws}/resources/{res}` —
/// soft-delete a resource definition.
///
/// Marks the resource row as deleted (a tombstone — the row is retained
/// for audit, not physically removed). A subsequent `GET` of the same id
/// is **404**: the read path excludes `deleted_at.is_some()`, so a
/// deleted resource is indistinguishable from a missing one.
///
/// Tenant isolation is identical to the update path: both operations receive
/// the authenticated scope. An unknown
/// id, an unparsable id, a resource owned by a *different* workspace, or
/// an already soft-deleted row all collapse to an indistinguishable
/// **404 Not Found** *before any mutation* — a caller authorized for one
/// workspace can neither soft-delete nor learn of another workspace's
/// resource. A successful soft-delete is **204 No Content**.
#[utoipa::path(
    delete,
    path = "/orgs/{org}/workspaces/{ws}/resources/{res}",
    tag = "workspaces.resources",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("res" = String, Path, description = "Resource identifier (`res_<ULID>`)."),
    ),
    responses(
        (status = 204, description = "Resource soft-deleted."),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Resource does not exist (also returned for a resource in another workspace, an already soft-deleted resource, or an unparsable id — no cross-tenant leak).", body = ProblemDetails),
        (status = 500, description = "Resource store error.", body = ProblemDetails),
        (status = 503, description = "Resource catalog backend is not configured on this instance.", body = ProblemDetails),
    ),
)]
pub async fn delete_resource(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, res)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let store = state.resource_store.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource catalog backend not configured".into())
    })?;

    // Tenant-isolation boundary (identical single audited fetch as the
    // read/update paths): a foreign / missing / already-tombstoned /
    // unparsable target collapses to the same 404 *before* `soft_delete`
    // is ever issued — a caller authorized for one workspace can neither
    // delete nor learn of another workspace's resource.
    let existing = fetch_owned_resource(&**store, &scope, &res).await?;

    store.soft_delete(&scope, &existing.id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/v1/orgs/{org}/workspaces/{ws}/resources/{res}/status` —
/// **read-only** runtime-status projection of one resource.
///
/// This endpoint **observes** a resource's lifecycle phase; it never
/// mutates one. Resource lifecycle (acquire / release / drain / reload)
/// is engine-owned and is intentionally NOT exposed over HTTP — there is
/// deliberately no acquire/release/drain route (INTEGRATION_MODEL
/// integration seam.1). The response body carries phase/health only, never `config`
/// or credential material (no secret echo).
///
/// The runtime lives in the worker processes that activated the resource;
/// they publish per-row status snapshots, and this handler reads them back
/// keyed by the confirmed row's own `(workspace, org)` and id, aggregated
/// over every live worker (`instances`).
///
/// Tenant isolation composes with the read path: the handler first
/// establishes ownership through the *same* audited
/// `fetch_owned_resource` boundary as the read/update/delete paths
/// (an unknown / unparsable id, a resource owned by a *different*
/// workspace, and a soft-deleted row ALL collapse to an indistinguishable
/// **404** — no cross-tenant existence/content/status oracle), and only
/// then projects the engine status seam for that **confirmed-owned**
/// resource. A status request for a resource not owned by the caller's
/// workspace never reaches the seam.
///
/// Outcomes for an owned, live resource:
/// - it has a live runtime in the engine ⇒ **200** with its projected
///   phase/health;
/// - it exists as a definition but was never activated ⇒ **200** with a
///   well-defined `inactive` status (it exists as config; it is just not
///   running — NOT a 404);
/// - no status backend is configured on this instance ⇒ **503** (the
///   catalog None-convention — an honest "unavailable", never a
///   fabricated status), checked *after* ownership so a 503 cannot leak
///   the existence of a foreign resource; an unreadable status backend is
///   likewise a **503**.
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/resources/{res}/status",
    tag = "workspaces.resources",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("res" = String, Path, description = "Resource identifier (`res_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Read-only runtime status (phase/health only; no raw config). A configured-but-never-activated resource reports an `inactive` phase here, not a 404.", body = ResourceStatusDto),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Resource does not exist (also returned for a resource in another workspace, a soft-deleted resource, or an unparsable id — no cross-tenant leak).", body = ProblemDetails),
        (status = 500, description = "Resource store error.", body = ProblemDetails),
        (status = 503, description = "Resource catalog backend or the runtime-status backend is not configured or unavailable.", body = ProblemDetails),
    ),
)]
pub async fn get_resource_status(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, res)): Path<(String, String, String)>,
) -> ApiResult<Json<ResourceStatusDto>> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let store = state.resource_store.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource catalog backend not configured".into())
    })?;

    // Tenant-isolation boundary FIRST (the single audited by-id fetch;
    // scoped store). A foreign / missing /
    // soft-deleted / unparsable target collapses to the same 404 —
    // resolved *before* the status seam is ever consulted, so a status
    // request can never be an existence/content/status oracle for
    // another workspace's resource.
    let row = fetch_owned_resource(&**store, &scope, &res).await?;

    // Ownership confirmed. The runtime-status backend is checked only
    // now (after isolation) so an absent backend reports 503 without
    // ever revealing whether a *foreign* resource exists.
    let status_port = state.resource_status.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource runtime-status backend not configured".into())
    })?;

    // Canonical `res_<ULID>` echo of the (already isolation-verified)
    // path id — single shared definition; see `canonical_res_id`.
    let id = canonical_res_id(&res)?;

    // The confirmed row's own id and scope key the status read — never an
    // attacker-influenced value. `None` = the resource exists as a
    // definition but no live worker serves it (never activated, retired,
    // or its workers are gone): a well-defined `inactive` status in a 200
    // body — NOT a 404 (the config row exists; absence of a *live runtime*
    // is a status, not a missing resource). An unreadable status backend
    // is a 503, never a fabricated status. Only workers running the row's
    // current version count.
    let status = status_port
        .runtime_status(&scope, &row.id, row.version)
        .await
        .map_err(|error| {
            tracing::warn!(
                target: "nebula_api::resource",
                %error,
                "resource runtime status could not be read"
            );
            ApiError::ServiceUnavailable("Resource runtime status is unavailable".into())
        })?;
    let dto = match status {
        Some(s) => ResourceStatusDto {
            id,
            phase: phase_from_seam(s.phase),
            healthy: s.healthy,
            accepting: s.accepting,
            instances: s.instances,
        },
        None => ResourceStatusDto {
            id,
            phase: ResourcePhase::Inactive,
            healthy: false,
            accepting: false,
            instances: 0,
        },
    };

    Ok(Json(dto))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use nebula_core::{Principal, WorkflowId};
    use nebula_storage_port::Scope;

    use super::{created_by, new_resource_row, replacement_resource_row};
    use crate::domain::resource::dto::{CreateResourceRequest, UpdateResourceRequest};

    /// SECURITY/audit: the two non-human actor classes must NOT collapse
    /// into identical `created_by` values. A `Workflow`
    /// principal must also never embed its `workflow_id` (a workflow is
    /// an actor *class* in this audit field, not an individual creator).
    #[test]
    fn created_by_distinguishes_workflow_system_and_unset() {
        let workflow = created_by(&Principal::Workflow {
            workflow_id: WorkflowId::new(),
            trigger_id: None,
        });
        let system = created_by(&Principal::System);

        // Distinct from each other and from the unset sentinel.
        assert_ne!(
            workflow, system,
            "Workflow and System must produce different created_by values"
        );
        assert_eq!(workflow, "workflow");
        assert_eq!(system, "system");

        // The workflow id is NOT embedded: two different workflows yield
        // the same class sentinel (no fabricated per-row identity).
        let other_workflow = created_by(&Principal::Workflow {
            workflow_id: WorkflowId::new(),
            trigger_id: None,
        });
        assert_eq!(
            workflow, other_workflow,
            "the Workflow sentinel must not vary with workflow_id"
        );
    }

    #[test]
    fn resource_rows_preserve_create_and_replacement_bindings_and_settings() {
        let scope = Scope::new("ws_test", "org_test");
        let create_bindings = BTreeMap::from([("token".to_owned(), "cred_create".to_owned())]);
        let override_doc = serde_json::json!({ "rate": { "requests": 5, "period_ms": 1000 } });
        let row = new_resource_row(
            nebula_core::ResourceId::new(),
            &scope,
            &Principal::System,
            CreateResourceRequest {
                slug: "primary".to_owned(),
                display_name: "Primary".to_owned(),
                kind: "http_pool".to_owned(),
                config: serde_json::json!({}),
                credential_bindings: create_bindings.clone(),
                topology: Some(serde_json::json!({ "max_size": 4 })),
                resilience_override: Some(override_doc.clone()),
            },
        );
        assert_eq!(row.credential_bindings, create_bindings);
        assert_eq!(row.resilience_override, Some(override_doc));

        let update_bindings = BTreeMap::from([("token".to_owned(), "cred_update".to_owned())]);
        let updated = replacement_resource_row(
            row,
            UpdateResourceRequest {
                display_name: "Updated".to_owned(),
                kind: "http_pool".to_owned(),
                config: serde_json::json!({}),
                credential_bindings: update_bindings.clone(),
                topology: Some(serde_json::Value::Null),
                resilience_override: None,
                expected_version: 0,
            },
            1,
        );
        assert_eq!(updated.credential_bindings, update_bindings);
        // Full replacement: omitted or `null` settings reset to the kind's
        // defaults, stored as absent either way.
        assert_eq!(updated.topology, None);
        assert_eq!(updated.resilience_override, None);
    }
}
