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
    extract::{Path, Query, State},
    http::StatusCode,
};
use nebula_core::{Principal, ResourceId, ResourceKey, TenantContext};
use nebula_engine::RegistrarError;
use nebula_storage::repos::ResourceEntry;

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
        slug: entry.slug,
        name: entry.display_name,
        kind: entry.kind,
        // Raw CAS row version (non-secret optimistic-concurrency
        // metadata) — surfaced as the `i64` the store holds, not a
        // stringified semver.
        version: entry.version,
        // Workflow attachment is not tracked by the resource store yet;
        // advertised honestly as empty rather than fabricated.
        attached_to_workflows: Vec::new(),
    })
}

/// Fetch a resource by its `res_<ULID>` path id, enforcing tenant isolation.
///
/// `ResourceRepo::get` is keyed purely by id and is **not**
/// workspace-scoped, so this is the sole by-id tenant boundary: a resource
/// in another workspace, a soft-deleted row, an unknown id, and an
/// unparsable id ALL collapse to the same [`ApiError::NotFound`] (no
/// cross-tenant existence / content / structural oracle — probing ids can
/// never learn which are structurally valid, which exist, or which belong
/// to another tenant). Every by-id resource handler MUST go through this —
/// never call `repo.get` plus an ad-hoc filter inline, so the isolation
/// predicate has exactly one audited definition that cannot drift between
/// the read/update/delete paths.
///
/// The order is load-bearing: parse first (an unparsable id is a 404 with
/// no backend touch), then the single fetch (a genuine backend fault
/// propagates as a `?`-mapped 500, *not* a 404), then the
/// caller-workspace + live predicate (foreign / tombstoned ⇒ 404). It is
/// applied **before any mutation** in the update/delete paths, so a caller
/// authorized for one workspace can neither mutate nor learn of another's
/// resource.
async fn fetch_owned_resource(
    repo: &dyn nebula_storage::repos::ResourceRepo,
    ws_bytes: &[u8],
    res: &str,
) -> Result<ResourceEntry, ApiError> {
    let resource_id = ResourceId::parse(res)
        .map_err(|_| ApiError::NotFound(format!("Resource {res} not found")))?;
    repo.get(resource_id.as_bytes().as_slice())
        .await?
        .filter(|entry| entry.workspace_id.as_slice() == ws_bytes && entry.deleted_at.is_none())
        .ok_or_else(|| ApiError::NotFound(format!("Resource {res} not found")))
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
/// extractor as `list_workflows`). Soft-deleted rows are excluded *after*
/// the store returns the page (the store returns the raw window including
/// tombstones; the handler owns the `deleted_at` filter). The raw
/// `config` blob is never surfaced — only the non-secret summary fields
/// (ADR-0028 §7).
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
        (status = 500, description = "Resource repository error.", body = ProblemDetails),
        (status = 503, description = "Resource catalog backend is not configured on this instance.", body = ProblemDetails),
    ),
)]
pub async fn list_resources(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ListResourcesResponse>> {
    // Workspace presence is guaranteed by the tenancy middleware for this
    // route; the explicit check keeps the handler honest if the route is
    // ever remounted. RBAC parity with the other workspace-scoped list
    // endpoints (`list_workflows`, `list_executions`) — no per-handler
    // permission gate under the current shared-trust JWT model.
    let workspace_id = tenant
        .require_workspace()
        .map_err(|_| ApiError::Forbidden("workspace context required".to_string()))?;
    // Mirror the by-id handlers' isolation-audit shape: a single named
    // `ws_bytes` is the workspace-scoping value, so the listing query is
    // visibly scoped to exactly the caller's authenticated workspace.
    let ws_bytes = workspace_id.as_bytes();

    let repo = state.resource_repo.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource catalog backend not configured".into())
    })?;

    // Same pagination contract as `list_workflows`: `PaginationParams`
    // already clamps `limit` to ≤ 100 and floors `offset` at 0. The repo
    // signature mirrors `repos::WorkflowRepo::list` (`u64` offset/limit),
    // so widen the clamped `usize` window without loss.
    let offset = params.offset() as u64;
    let limit = params.limit() as u64;

    let entries = repo.list(ws_bytes.as_slice(), offset, limit).await?;

    let resources = entries
        .into_iter()
        // Soft-deleted definitions are not part of the catalog view. The
        // store returns the raw page (tombstones included); the handler
        // owns this exclusion policy.
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

    // Single audited by-id tenant boundary: an unknown / unparsable id, a
    // foreign-workspace row, and a soft-deleted tombstone all collapse to
    // an indistinguishable 404 (no cross-tenant existence/content/
    // structural oracle).
    let entry = fetch_owned_resource(&**repo, &ws_bytes, &res).await?;

    Ok(Json(entry_to_summary(entry)?))
}

/// Validate `config` against `kind` through the engine's closed
/// `kind → registrar` allowlist (schema + closed-set guard, **no** live
/// `Manager` registration — that is an engine-activation concern,
/// INTEGRATION_MODEL §13.1).
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
///   logged server-side and never echoed to the client (ADR-0028 §7).
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

/// Map a [`nebula_storage::StorageError`] from a resource CAS `update`
/// onto the HTTP contract.
///
/// Only the **specific** [`StorageError::Conflict`] variant (the
/// optimistic-concurrency / CAS-version mismatch) becomes
/// [`ApiError::Conflict`] (409): a caller's `expected_version` was stale.
/// Every other `StorageError` stays the opaque
/// [`ApiError::Storage`] → 500 (no internal detail leak). This is
/// deliberately *not* a catch-all "any storage error ⇒ 409": a genuine
/// backend fault (connection, serialization, …) must not be mis-signalled
/// to the client as a version conflict, and a stale-version write must
/// not be mis-signalled as a 500.
#[must_use]
pub fn map_resource_update_storage_error(err: nebula_storage::StorageError) -> ApiError {
    match err {
        nebula_storage::StorageError::Conflict {
            expected, actual, ..
        } => {
            // The version numbers are non-secret optimistic-concurrency
            // metadata (not config/secret material), so echoing them is a
            // useful, safe CAS diagnostic per ADR-0028 §7.
            ApiError::Conflict(format!(
                "resource was modified concurrently (expected version {expected}, found {actual}); \
                 re-read and retry"
            ))
        },
        // Connection / serialization / timeout / not-found-on-update / …
        // are not a version conflict — keep the opaque 500 mapping.
        other => ApiError::Storage(other),
    }
}

/// Reserved-prefix tag (byte 0) marking a `created_by` value as a
/// non-human **audit sentinel**, never an identity.
///
/// `UserId` / `ServiceAccountId` are ULIDs whose byte 0 is the high byte
/// of a 48-bit millisecond Unix timestamp — it stays well below `0xFF`
/// for several thousand more years. So a leading `0xFF` is structurally
/// unreachable for any real id and unambiguously marks "this is a
/// reserved sentinel, not a ULID". `0x00…` (16 zero bytes) remains the
/// pre-existing "unset creator" value and is intentionally left distinct
/// from these.
const CREATED_BY_SENTINEL_TAG: u8 = 0xFF;
/// Sentinel class discriminant (byte 1) for a workflow-initiated create.
const CREATED_BY_CLASS_WORKFLOW: u8 = 0x01;
/// Sentinel class discriminant (byte 1) for a system-internal create.
const CREATED_BY_CLASS_SYSTEM: u8 = 0x02;

/// 16 raw id bytes for the resource's `created_by`, derived from the
/// authenticated principal.
///
/// `User` / `ServiceAccount` carry a single ULID identity → its bytes.
/// `Workflow` / `System` are not a config-author identity for a
/// human/SA-initiated create, but they are two *distinct* non-human
/// actor classes and must not collapse into the same audit bytes (nor
/// into the pre-existing all-zero "unset" value). Each gets a fixed
/// 16-byte reserved sentinel — `[0xFF, class, 0, …]` — that is
/// **audit-only**: structurally not a valid ULID/user-id, never to be
/// resolved back to an identity. `Workflow`'s `workflow_id` is
/// deliberately NOT embedded: `created_by` is a *who authored this
/// config* field, and a workflow is an actor class here, not an
/// individual creator — recording its id would fabricate a per-row
/// identity the audit model does not assign.
fn created_by_bytes(principal: &Principal) -> Vec<u8> {
    let sentinel = |class: u8| {
        let mut bytes = vec![0_u8; 16];
        bytes[0] = CREATED_BY_SENTINEL_TAG;
        bytes[1] = class;
        bytes
    };
    match principal {
        Principal::User(uid) => uid.as_bytes().to_vec(),
        Principal::ServiceAccount(sid) => sid.as_bytes().to_vec(),
        Principal::Workflow { .. } => sentinel(CREATED_BY_CLASS_WORKFLOW),
        Principal::System => sentinel(CREATED_BY_CLASS_SYSTEM),
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
    validate_resource_config(&state, &body.kind, body.config.clone())?;

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
/// Tenant isolation (the severe surface — `ResourceRepo::get`/`update`
/// are keyed purely by id and are **not** workspace-scoped, so the
/// handler is the isolation boundary). All of the following collapse to
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
///   report is logged server-side, never echoed — ADR-0028 §7).
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
    let workspace_id = tenant
        .require_workspace()
        .map_err(|_| ApiError::Forbidden("workspace context required".to_string()))?;
    let ws_bytes = workspace_id.as_bytes();

    let repo = state.resource_repo.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource catalog backend not configured".into())
    })?;

    // Tenant-isolation boundary *before any mutation*: the single audited
    // by-id fetch (the store is NOT workspace-scoped). A foreign / missing
    // / soft-deleted / unparsable target collapses to the same 404 — a
    // caller authorized for one workspace can neither mutate nor learn of
    // another workspace's resource (no 200/403/409, no existence/content
    // leak).
    let existing = fetch_owned_resource(&**repo, &ws_bytes, &res).await?;

    // Re-validate kind + config BEFORE persistence (same fail-closed
    // rules as create). A PUT must not bypass create-time validation.
    validate_resource_config(&state, &body.kind, body.config.clone())?;

    // The CAS contract increments the stored counter on a successful
    // compare-and-swap against `expected_version`. A *saturating* add
    // would silently pin at `i64::MAX` and defeat CAS (every subsequent
    // update would compare-and-swap against the same ceiling), so a
    // would-overflow `expected_version` is rejected as a conflict
    // instead — re-read and retry. (`i64::MAX` row versions are not
    // reachable in practice; this is a correctness guard, not a hot
    // path.)
    let next_version = body.expected_version.checked_add(1).ok_or_else(|| {
        ApiError::Conflict("resource version is at its maximum; re-read and retry".to_string())
    })?;

    // Build the row to persist from the *fetched* row: `id`,
    // `workspace_id` (== caller ws, just verified — NEVER from the body;
    // the DTO has no such field), `created_at`/`created_by` are
    // immutable. Only the caller-mutable fields are taken from the
    // request. `slug` is workspace-unique and not part of the update
    // contract, so the existing slug is preserved.
    let updated = ResourceEntry {
        id: existing.id,
        workspace_id: existing.workspace_id,
        slug: existing.slug,
        display_name: body.display_name,
        kind: body.kind,
        config: body.config,
        created_at: existing.created_at,
        created_by: existing.created_by,
        version: next_version,
        deleted_at: None,
    };

    // The store owns the post-CAS increment and returns the
    // authoritative new version; `next_version` above was only the row
    // we asked it to persist. Surface the store's value, never the
    // handler-side prediction.
    let new_version = repo
        .update(&updated, body.expected_version)
        .await
        .map_err(map_resource_update_storage_error)?;

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
/// Tenant isolation is identical to the update path: `ResourceRepo::get`
/// / `soft_delete` are keyed purely by id and are **not**
/// workspace-scoped, so the handler is the isolation boundary. An unknown
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
        (status = 500, description = "Resource repository error.", body = ProblemDetails),
        (status = 503, description = "Resource catalog backend is not configured on this instance.", body = ProblemDetails),
    ),
)]
pub async fn delete_resource(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, res)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let workspace_id = tenant
        .require_workspace()
        .map_err(|_| ApiError::Forbidden("workspace context required".to_string()))?;
    let ws_bytes = workspace_id.as_bytes();

    let repo = state.resource_repo.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource catalog backend not configured".into())
    })?;

    // Tenant-isolation boundary (identical single audited fetch as the
    // read/update paths): a foreign / missing / already-tombstoned /
    // unparsable target collapses to the same 404 *before* `soft_delete`
    // is ever issued — a caller authorized for one workspace can neither
    // delete nor learn of another workspace's resource.
    let existing = fetch_owned_resource(&**repo, &ws_bytes, &res).await?;

    repo.soft_delete(existing.id.as_slice()).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/v1/orgs/{org}/workspaces/{ws}/resources/{res}/status` —
/// **read-only** runtime-status projection of one resource.
///
/// This endpoint **observes** a resource's lifecycle phase; it never
/// mutates one. Resource lifecycle (acquire / release / drain / reload)
/// is engine-owned and is intentionally NOT exposed over HTTP — there is
/// deliberately no acquire/release/drain route (INTEGRATION_MODEL
/// §13.1). The response body carries phase/health only, never `config`
/// or credential material (ADR-0028 §7).
///
/// Tenant isolation composes with the read path. The runtime
/// [`nebula_resource::Manager`] is keyed by `(ResourceKey, ScopeLevel)`,
/// **not** by workspace — workspace ownership lives in the config row. So
/// the handler first establishes ownership through the *same* audited
/// [`fetch_owned_resource`] boundary as the read/update/delete paths
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
///   the existence of a foreign resource.
///
/// A stored `kind` that is not a valid resource key is a storage
/// invariant violation (the resource genuinely exists and is owned), so
/// it is an opaque **500** — never a 404 (which would falsely deny an
/// owned resource) and never a fabricated status.
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
        (status = 500, description = "Resource repository error, or a stored resource `kind` that is not a valid resource key.", body = ProblemDetails),
        (status = 503, description = "Resource catalog backend or the runtime-status backend is not configured on this instance.", body = ProblemDetails),
    ),
)]
pub async fn get_resource_status(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, res)): Path<(String, String, String)>,
) -> ApiResult<Json<ResourceStatusDto>> {
    let workspace_id = tenant
        .require_workspace()
        .map_err(|_| ApiError::Forbidden("workspace context required".to_string()))?;
    let ws_bytes = workspace_id.as_bytes();

    let repo = state.resource_repo.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource catalog backend not configured".into())
    })?;

    // Tenant-isolation boundary FIRST (the single audited by-id fetch;
    // the store is NOT workspace-scoped). A foreign / missing /
    // soft-deleted / unparsable target collapses to the same 404 —
    // resolved *before* the status seam is ever consulted, so a status
    // request can never be an existence/content/status oracle for
    // another workspace's resource.
    let entry = fetch_owned_resource(&**repo, &ws_bytes, &res).await?;

    // Ownership confirmed. The runtime-status backend is checked only
    // now (after isolation) so an absent backend reports 503 without
    // ever revealing whether a *foreign* resource exists.
    let status_port = state.resource_status.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource runtime-status backend not configured".into())
    })?;

    // The confirmed row's own `kind` keys the engine status seam — never
    // an attacker-influenced value. A stored kind that is not a valid
    // resource key is a storage-invariant violation on an
    // already-confirmed-owned resource: an opaque 500, never a 404 (the
    // resource exists and is owned) and never a fabricated status.
    let key = ResourceKey::new(&entry.kind).map_err(|_| {
        tracing::error!(
            target: "nebula_api::resource",
            "stored resource kind is not a valid resource key"
        );
        ApiError::Internal("resource has an invalid stored kind".to_string())
    })?;

    // Canonical `res_<ULID>` echo of the (already isolation-verified)
    // path id — single shared definition; see `canonical_res_id`.
    let id = canonical_res_id(&res)?;

    // Project the read-only engine seam. `None` = the resource exists as
    // a definition but has no live runtime (never activated, or a
    // fail-closed ambiguous `(key, scope)`): a well-defined `inactive`
    // status in a 200 body — NOT a 404 (the config row exists; absence
    // of a *live runtime* is a status, not a missing resource).
    let dto = match status_port.runtime_status(&key) {
        Some(s) => ResourceStatusDto {
            id,
            phase: phase_from_seam(s.phase),
            healthy: s.healthy,
            accepting: s.accepting,
        },
        None => ResourceStatusDto {
            id,
            phase: ResourcePhase::Inactive,
            healthy: false,
            accepting: false,
        },
    };

    Ok(Json(dto))
}

#[cfg(test)]
mod tests {
    use nebula_core::{Principal, WorkflowId};

    use super::{
        CREATED_BY_CLASS_SYSTEM, CREATED_BY_CLASS_WORKFLOW, CREATED_BY_SENTINEL_TAG,
        created_by_bytes,
    };

    /// SECURITY/audit: the two non-human actor classes must NOT collapse
    /// into identical `created_by` bytes, and neither may collide with
    /// the pre-existing all-zero "unset creator" value. A `Workflow`
    /// principal must also never embed its `workflow_id` (a workflow is
    /// an actor *class* in this audit field, not an individual creator).
    #[test]
    fn created_by_distinguishes_workflow_system_and_unset() {
        let wf = created_by_bytes(&Principal::Workflow {
            workflow_id: WorkflowId::new(),
            trigger_id: None,
        });
        let sys = created_by_bytes(&Principal::System);

        // All exactly 16 bytes (the `resources.created_by` BYTEA width).
        assert_eq!(wf.len(), 16);
        assert_eq!(sys.len(), 16);

        // Distinct from each other and from the unset sentinel.
        assert_ne!(
            wf, sys,
            "Workflow and System must produce different created_by bytes"
        );
        assert_ne!(wf, vec![0_u8; 16], "Workflow must not be the unset value");
        assert_ne!(sys, vec![0_u8; 16], "System must not be the unset value");

        // Reserved tag + class discriminant, audit-only (structurally
        // not a valid ULID — byte 0 == 0xFF is unreachable for a real
        // id for millennia).
        assert_eq!(wf[0], CREATED_BY_SENTINEL_TAG);
        assert_eq!(wf[1], CREATED_BY_CLASS_WORKFLOW);
        assert_eq!(sys[0], CREATED_BY_SENTINEL_TAG);
        assert_eq!(sys[1], CREATED_BY_CLASS_SYSTEM);

        // The workflow id is NOT embedded: two different workflows yield
        // the same class sentinel (no fabricated per-row identity).
        let wf2 = created_by_bytes(&Principal::Workflow {
            workflow_id: WorkflowId::new(),
            trigger_id: None,
        });
        assert_eq!(
            wf, wf2,
            "the Workflow sentinel must not vary with workflow_id"
        );
    }
}
