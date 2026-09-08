//! Workflow handlers
//!
//! `unused_qualifications` is silenced for the module: the
//! `IntoParams`-derived `PaginationParams` triggers it from inside the
//! `#[utoipa::path(... params(PaginationParams))]` expansion (utoipa 5.5
//! macro-generated code paths qualify the type).
#![allow(unused_qualifications)]

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use chrono::Utc;
use nebula_core::{TenantContext, WorkflowId};
use serde_json::Value;

use crate::{
    domain::{
        execution::{
            dto::{ExecutionResponse, StartExecutionRequest},
            handler::start_workflow,
        },
        shared::PaginationParams,
        workflow::dto::{
            CreateWorkflowRequest, ListWorkflowsResponse, UpdateWorkflowRequest, WorkflowResponse,
            WorkflowValidateResponse,
        },
    },
    error::{ApiError, ApiResult, ProblemDetails},
    state::AppState,
};

/// Identity and control fields inside a stored workflow definition that the
/// API must never let a client overwrite via `update_workflow`.
///
/// Mutating these drifts the stored identity away from the repository key and
/// corrupts downstream consumers that rely on canonical `WorkflowDefinition`
/// invariants (version, ownership, schema version). See issue #344.
const IMMUTABLE_DEFINITION_FIELDS: &[&str] = &[
    "id",
    "version",
    "owner_id",
    "schema_version",
    "created_at",
    "updated_at",
    // `name` / `description` have dedicated top-level payload fields already,
    // so they must not be smuggled through a nested `definition` update either.
    "name",
    "description",
];

/// Extract a Unix-epoch timestamp from a workflow definition field.
///
/// Canonical `WorkflowDefinition` serializes timestamps as RFC3339 strings
/// (because `chrono::DateTime<Utc>` uses string representation), while the
/// current API write path still stores them as raw i64 unix seconds. This
/// helper accepts **both** shapes so responses remain correct regardless of
/// which path produced the stored blob.
///
/// Returns `None` when the field is absent or has an unsupported shape — the
/// caller decides whether to fall back to `0`, surface an internal error, or
/// omit the field. Fixes issue #343.
pub(crate) fn extract_timestamp(definition: &Value, key: &str) -> Option<i64> {
    let field = definition.get(key)?;
    if let Some(n) = field.as_i64() {
        return Some(n);
    }
    if let Some(s) = field.as_str()
        && let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s)
    {
        return Some(dt.timestamp());
    }
    None
}

/// List workflows
/// GET /api/v1/orgs/{org}/workspaces/{ws}/workflows
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/workflows",
    tag = "workspaces.workflows",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        PaginationParams,
    ),
    responses(
        (status = 200, description = "Paginated workflow summaries.", body = ListWorkflowsResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 500, description = "Workflow repository unavailable.", body = ProblemDetails),
    ),
)]
pub async fn list_workflows(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ListWorkflowsResponse>> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let offset = params.offset();
    let limit = params.limit();

    // Fetch workflows scoped to the caller's tenant (a freshly bound
    // tenancy decorator keyed by the request scope).
    let workflows = state.workflow_list_scoped(&scope, offset, limit).await?;

    let total = state.workflow_count_scoped(&scope).await?;

    // Map to response DTOs
    let workflow_responses: Vec<WorkflowResponse> = workflows
        .into_iter()
        .map(|(id, definition)| {
            // Extract fields from workflow definition JSON
            let name = definition
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Unnamed Workflow")
                .to_string();

            let description = definition
                .get("description")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);

            let created_at = extract_timestamp(&definition, "created_at").unwrap_or(0);
            let updated_at = extract_timestamp(&definition, "updated_at").unwrap_or(0);

            WorkflowResponse {
                id: id.to_string(),
                name,
                description,
                created_at,
                updated_at,
            }
        })
        .collect();

    Ok(Json(ListWorkflowsResponse {
        workflows: workflow_responses,
        total,
        page: params.page,
        page_size: params.page_size,
    }))
}

/// Get workflow by ID
/// GET /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/workflows/{wf}",
    tag = "workspaces.workflows",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("wf" = String, Path, description = "Workflow identifier (`wf_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Workflow detail.", body = WorkflowResponse),
        (status = 400, description = "Invalid workflow identifier.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Workflow does not exist.", body = ProblemDetails),
        (status = 500, description = "Workflow repository unavailable.", body = ProblemDetails),
    ),
)]
pub async fn get_workflow(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, id)): Path<(String, String, String)>,
) -> ApiResult<Json<WorkflowResponse>> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    // Parse workflow ID
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {e}")))?;

    // Fetch the workflow scoped to the caller's tenant.
    let definition = state
        .workflow_definition_scoped(&scope, workflow_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Workflow {id} not found")))?;

    // Extract fields from workflow definition JSON
    let name = definition
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unnamed Workflow")
        .to_string();

    let description = definition
        .get("description")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);

    let created_at = extract_timestamp(&definition, "created_at").unwrap_or(0);
    let updated_at = extract_timestamp(&definition, "updated_at").unwrap_or(0);

    Ok(Json(WorkflowResponse {
        id,
        name,
        description,
        created_at,
        updated_at,
    }))
}

/// Create workflow
/// POST /api/v1/orgs/{org}/workspaces/{ws}/workflows
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/workflows",
    tag = "workspaces.workflows",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
    ),
    request_body = CreateWorkflowRequest,
    responses(
        (status = 201, description = "Workflow created.", body = WorkflowResponse),
        (status = 400, description = "Validation error (e.g. blank name).", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 500, description = "Workflow repository unavailable.", body = ProblemDetails),
    ),
)]
pub async fn create_workflow(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Json(payload): Json<CreateWorkflowRequest>,
) -> ApiResult<(StatusCode, Json<WorkflowResponse>)> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    // Validate workflow name
    if payload.name.trim().is_empty() {
        return Err(ApiError::validation_message(
            "Workflow name cannot be empty",
        ));
    }

    // Generate new workflow ID
    let workflow_id = WorkflowId::new();

    // Current timestamp — `chrono::Utc::now()` is monotonic through time
    // shifts and does not panic on clocks set before 1970, unlike
    // `SystemTime::duration_since(UNIX_EPOCH).unwrap()`.
    //
    // The stored definition must round-trip as a `WorkflowDefinition`
    // (its `created_at`/`updated_at` are `DateTime<Utc>`, which serde
    // encodes as RFC 3339 strings). Writing raw Unix-seconds integers
    // here produces a JSON object that *looks* like a workflow but fails
    // `serde_json::from_str::<WorkflowDefinition>` — the parse the
    // activate path performs before flipping the active flag. Persist the
    // RFC 3339 form; the `WorkflowResponse` API field stays Unix seconds
    // and is derived from the same instant.
    let now = Utc::now();
    let now_secs = now.timestamp();
    let now_rfc3339 = now.to_rfc3339();

    // Build workflow definition by merging request definition with metadata.
    // A workflow definition must be an object: wrapping arrays / strings /
    // null into an outer metadata object creates JSON that cannot
    // deserialize as `WorkflowDefinition` on activation.
    let mut definition = payload.definition.clone();
    let Some(obj) = definition.as_object_mut() else {
        return Err(ApiError::validation_message(
            "Workflow definition must be a JSON object",
        ));
    };
    // The server owns every immutable identity/control field — a
    // client must not smuggle its own `id` / `version` / `owner_id`
    // / `schema_version` through the create `definition` (the update
    // path *rejects* these per issue #344; create previously
    // persisted them verbatim, so the stored definition's identity
    // could diverge from the server-generated `workflow_id` used as
    // the repository key). These are *overwritten* with
    // server-authoritative values, not stripped: `id` / `version` /
    // `schema_version` are required by the canonical
    // `WorkflowDefinition` schema, so removing them would yield a
    // blob that fails the `from_str::<WorkflowDefinition>` parse the
    // activate path performs. A client-supplied `owner_id` is
    // dropped (the field is optional; create assigns no owner here).
    obj.insert("id".to_string(), serde_json::json!(workflow_id.to_string()));
    obj.insert(
        "version".to_string(),
        serde_json::json!({ "major": 0, "minor": 1, "patch": 0 }),
    );
    obj.insert(
        "schema_version".to_string(),
        serde_json::json!(nebula_workflow::CURRENT_SCHEMA_VERSION),
    );
    obj.remove("owner_id");
    obj.insert("name".to_string(), serde_json::json!(payload.name));
    if let Some(desc) = &payload.description {
        obj.insert("description".to_string(), serde_json::json!(desc));
    }
    obj.insert("created_at".to_string(), serde_json::json!(now_rfc3339));
    obj.insert("updated_at".to_string(), serde_json::json!(now_rfc3339));

    // Save workflow with version 0 (new workflow) scoped to the tenant.
    state
        .workflow_save_scoped(&scope, workflow_id, 0, definition.clone())
        .await?;

    // Build response
    let response = WorkflowResponse {
        id: workflow_id.to_string(),
        name: payload.name,
        description: payload.description,
        created_at: now_secs,
        updated_at: now_secs,
    };

    Ok((StatusCode::CREATED, Json(response)))
}

/// Update workflow
/// PUT /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}
#[utoipa::path(
    put,
    path = "/orgs/{org}/workspaces/{ws}/workflows/{wf}",
    tag = "workspaces.workflows",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("wf" = String, Path, description = "Workflow identifier (`wf_<ULID>`)."),
    ),
    request_body = UpdateWorkflowRequest,
    responses(
        (status = 200, description = "Workflow updated.", body = WorkflowResponse),
        (status = 400, description = "Validation error or attempt to mutate immutable identity field.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Workflow does not exist.", body = ProblemDetails),
        (status = 409, description = "Concurrent modification detected (optimistic concurrency).", body = ProblemDetails),
        (status = 500, description = "Workflow repository unavailable.", body = ProblemDetails),
    ),
)]
pub async fn update_workflow(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, id)): Path<(String, String, String)>,
    Json(payload): Json<UpdateWorkflowRequest>,
) -> ApiResult<Json<WorkflowResponse>> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    // Parse workflow ID
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {e}")))?;

    // Get current workflow with version, scoped to the tenant.
    let (version, mut definition) = state
        .workflow_with_version_scoped(&scope, workflow_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Workflow {id} not found")))?;

    // Current timestamp — `chrono::Utc::now()` is monotonic through time
    // shifts and does not panic on clocks set before 1970, unlike
    // `SystemTime::duration_since(UNIX_EPOCH).unwrap()`. Persist the
    // RFC 3339 form so the stored definition stays a parseable
    // `WorkflowDefinition` (see `create_workflow` for the rationale);
    // the response timestamp is derived via `extract_timestamp`, which
    // accepts both encodings.
    let now_rfc3339 = Utc::now().to_rfc3339();

    // Update definition with new values
    if let Some(obj) = definition.as_object_mut() {
        // Update name if provided
        if let Some(name) = &payload.name {
            if name.trim().is_empty() {
                return Err(ApiError::validation_message(
                    "Workflow name cannot be empty",
                ));
            }
            obj.insert("name".to_string(), serde_json::json!(name));
        }

        // Update description if provided
        if let Some(desc) = &payload.description {
            obj.insert("description".to_string(), serde_json::json!(desc));
        }

        // Merge definition if provided.
        //
        // Reject any attempt to mutate immutable identity/control fields
        // inside the nested `definition` payload (issue #344). A client that
        // wants a different identity must create a new workflow — otherwise
        // the stored id/version/owner would silently diverge from the
        // repository key used to route the request.
        if let Some(new_def) = &payload.definition {
            let Some(new_obj) = new_def.as_object() else {
                return Err(ApiError::validation_message(
                    "Workflow definition must be a JSON object",
                ));
            };
            for key in new_obj.keys() {
                if IMMUTABLE_DEFINITION_FIELDS.contains(&key.as_str()) {
                    return Err(ApiError::validation_message(format!(
                        "Cannot modify immutable workflow field '{key}'",
                    )));
                }
            }
            for (key, value) in new_obj {
                obj.insert(key.clone(), value.clone());
            }
        }

        // Update the updated_at timestamp
        obj.insert("updated_at".to_string(), serde_json::json!(now_rfc3339));
    } else {
        return Err(ApiError::Internal(
            "Invalid workflow definition format".to_string(),
        ));
    }

    // Save with optimistic concurrency control via the accessor (a CAS
    // miss is mapped to the same 409 message the legacy path produced).
    state
        .workflow_save_scoped(&scope, workflow_id, version, definition.clone())
        .await?;

    // Extract fields for response
    let name = definition
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unnamed Workflow")
        .to_string();

    let description = definition
        .get("description")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);

    let created_at = extract_timestamp(&definition, "created_at").unwrap_or(0);
    let updated_at = extract_timestamp(&definition, "updated_at").unwrap_or(0);

    Ok(Json(WorkflowResponse {
        id,
        name,
        description,
        created_at,
        updated_at,
    }))
}

/// Delete workflow
/// DELETE /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}
#[utoipa::path(
    delete,
    path = "/orgs/{org}/workspaces/{ws}/workflows/{wf}",
    tag = "workspaces.workflows",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("wf" = String, Path, description = "Workflow identifier (`wf_<ULID>`)."),
    ),
    responses(
        (status = 204, description = "Workflow deleted."),
        (status = 400, description = "Invalid workflow identifier.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Workflow does not exist.", body = ProblemDetails),
        (status = 500, description = "Workflow repository unavailable.", body = ProblemDetails),
    ),
)]
pub async fn delete_workflow(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, id)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    // Parse workflow ID
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {e}")))?;

    // Delete the workflow scoped to the tenant (missing ⇒ false).
    let existed = state.workflow_delete_scoped(&scope, workflow_id).await?;

    // Return 404 if workflow didn't exist, 204 No Content if it was deleted
    if existed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound(format!("Workflow {id} not found")))
    }
}

/// Activate workflow
/// POST /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/activate
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/workflows/{wf}/activate",
    tag = "workspaces.workflows",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("wf" = String, Path, description = "Workflow identifier (`wf_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Workflow activated.", body = WorkflowResponse),
        (status = 400, description = "Invalid workflow identifier.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Workflow does not exist.", body = ProblemDetails),
        (status = 409, description = "Concurrent modification detected.", body = ProblemDetails),
        (status = 422, description = "Workflow definition cannot be compiled for the selected worker release.", body = ProblemDetails),
        (status = 500, description = "Workflow repository unavailable.", body = ProblemDetails),
        (status = 503, description = "Activation is unavailable or publication outcome is indeterminate. An indeterminate outcome identifies the original attempt for reconciliation.", body = ProblemDetails),
    ),
)]
pub async fn activate_workflow(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, id)): Path<(String, String, String)>,
) -> ApiResult<Json<WorkflowResponse>> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    // Parse workflow ID
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {e}")))?;

    // Get current workflow with version for optimistic concurrency,
    // scoped to the tenant.
    let (version, definition) = state
        .workflow_with_version_scoped(&scope, workflow_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Workflow {id} not found")))?;

    // NOTE: `serde_json::from_value` cannot zero-copy borrow `&str` from a
    // `Value::String`, which causes failures for types like `domain_key::Key<T>`
    // that use `<&str>::deserialize` on human-readable formats.  Round-tripping
    // through a JSON string (`to_string` → `from_str`) gives a proper streaming
    // deserializer that does support `visit_borrowed_str`, so all key types
    // parse correctly.
    let raw_json = serde_json::to_string(&definition)
        .map_err(|_| ApiError::Internal("Invalid stored workflow definition".into()))?;
    let workflow_def: nebula_workflow::WorkflowDefinition = serde_json::from_str(&raw_json)
        .map_err(|_| ApiError::validation_message("Workflow definition cannot be parsed"))?;

    let service = state.workflow_activation.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Workflow activation is not configured".into())
    })?;
    let receipt = service
        .activate(&scope, workflow_id, version, workflow_def)
        .await?;
    let persisted = serde_json::to_vec(&receipt.version().definition)
        .map_err(|_| ApiError::Internal("Invalid persisted activation receipt".into()))?;
    let persisted: nebula_workflow::WorkflowDefinition = serde_json::from_slice(&persisted)
        .map_err(|_| ApiError::Internal("Invalid persisted activation receipt".into()))?;

    Ok(Json(WorkflowResponse {
        id,
        name: persisted.name,
        description: persisted.description,
        created_at: persisted.created_at.timestamp(),
        updated_at: persisted.updated_at.timestamp(),
    }))
}

/// Execute workflow (enqueue and return 202 Accepted)
/// POST /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/execute
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/workflows/{wf}/execute",
    tag = "workspaces.workflows",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("wf" = String, Path, description = "Workflow identifier (`wf_<ULID>`)."),
        ("Idempotency-Key" = Option<String>, Header, description = "Caller start identity, shared with /executions. Replays retain the original execution across workflow republication."),
    ),
    request_body = StartExecutionRequest,
    responses(
        (status = 202, description = "Execution, exact contract bundle and Start command committed atomically. A keyed replay reports the original execution's persisted state.", body = ExecutionResponse),
        (status = 400, description = "Invalid workflow identifier, start key or input.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Workflow does not exist.", body = ProblemDetails),
        (status = 409, description = "Start key mismatch or exact revision admission conflict; nothing was written.", body = ProblemDetails),
        (status = 422, description = "The workflow is not activated or its recorded runtime requirements cannot be admitted.", body = ProblemDetails),
        (status = 500, description = "Stored workflow or receipt identities are inconsistent.", body = ProblemDetails),
        (status = 503, description = "Start admission is unavailable, its outcome is indeterminate, or an accepted execution's receipt cannot be read. Reconcile any returned original execution identity before submitting another start.", body = ProblemDetails),
    ),
)]
pub async fn execute_workflow(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, id)): Path<(String, String, String)>,
    headers: HeaderMap,
    Json(payload): Json<StartExecutionRequest>,
) -> ApiResult<(StatusCode, Json<ExecutionResponse>)> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    // Parse workflow ID
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {e}")))?;

    start_workflow(&state, &scope, workflow_id, &headers, payload).await
}

/// Validate workflow
/// POST /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/validate
///
/// Loads the stored workflow, deserializes it as a
/// [`nebula_workflow::WorkflowDefinition`], and runs structural validation
/// (DAG cycle check, node references, schema version, etc.).
///
/// Always returns **200 OK**. The response body indicates the outcome:
/// - `{valid: true, errors: []}` — definition is structurally valid.
/// - `{valid: false, errors: ["…"]}` — definition has validation errors.
///
/// A 422 is only returned when the stored JSON cannot be parsed at all (i.e.
/// the blob is not a `WorkflowDefinition`), which is treated as a validation
/// error rather than a not-found condition.
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/workflows/{wf}/validate",
    tag = "workspaces.workflows",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("wf" = String, Path, description = "Workflow identifier (`wf_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Validation ran; body indicates valid/invalid with error list.", body = WorkflowValidateResponse),
        (status = 400, description = "Invalid workflow identifier.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Workflow does not exist.", body = ProblemDetails),
        (status = 422, description = "Stored definition cannot be parsed as a workflow.", body = ProblemDetails),
        (status = 500, description = "Workflow repository unavailable.", body = ProblemDetails),
    ),
)]
pub async fn validate_workflow_handler(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, id)): Path<(String, String, String)>,
) -> ApiResult<Json<WorkflowValidateResponse>> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {e}")))?;

    let definition = state
        .workflow_definition_scoped(&scope, workflow_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Workflow {id} not found")))?;

    // Use the same streaming decode and frozen compiler snapshot as activation.
    // A mutable authoring registry can otherwise accept a definition that the
    // exact activation compiler rejects.
    let raw_json = serde_json::to_string(&definition)
        .map_err(|e| ApiError::Internal(format!("Failed to serialize workflow definition: {e}")))?;
    let workflow_def: nebula_workflow::WorkflowDefinition = serde_json::from_str(&raw_json)
        .map_err(|e| {
            ApiError::validation_message(format!(
                "Workflow definition cannot be parsed as WorkflowDefinition: {e}"
            ))
        })?;

    let service = state.workflow_activation.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Workflow activation is not configured".into())
    })?;
    let errors = match service.validate_definition(&workflow_def) {
        Ok(()) => Vec::new(),
        Err(error) => error
            .diagnostics()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
    };
    Ok(Json(WorkflowValidateResponse {
        valid: errors.is_empty(),
        errors,
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{IMMUTABLE_DEFINITION_FIELDS, extract_timestamp};

    #[test]
    fn extract_timestamp_parses_i64() {
        let v = json!({ "created_at": 1_700_000_000_i64 });
        assert_eq!(extract_timestamp(&v, "created_at"), Some(1_700_000_000));
    }

    #[test]
    fn extract_timestamp_parses_rfc3339_string() {
        // Regression for #343: canonical WorkflowDefinition stores
        // `DateTime<Utc>` as an RFC3339 string, not a unix-seconds i64.
        let v = json!({ "updated_at": "2024-01-15T12:34:56Z" });
        let ts = extract_timestamp(&v, "updated_at").expect("rfc3339 parses");
        assert_eq!(ts, 1_705_322_096);
    }

    #[test]
    fn extract_timestamp_rejects_garbage() {
        let v = json!({ "created_at": "not-a-date" });
        assert_eq!(extract_timestamp(&v, "created_at"), None);
    }

    #[test]
    fn extract_timestamp_handles_missing_field() {
        let v = json!({});
        assert_eq!(extract_timestamp(&v, "created_at"), None);
    }

    #[test]
    fn immutable_fields_cover_identity_and_metadata() {
        // Regression for #344: identity/control fields must be in the
        // blocklist so a nested `definition` payload cannot overwrite them.
        for key in ["id", "version", "owner_id", "schema_version"] {
            assert!(
                IMMUTABLE_DEFINITION_FIELDS.contains(&key),
                "identity field `{key}` must be immutable in update_workflow",
            );
        }
        for key in ["name", "description", "created_at", "updated_at"] {
            assert!(
                IMMUTABLE_DEFINITION_FIELDS.contains(&key),
                "metadata field `{key}` must be immutable in update_workflow",
            );
        }
    }
}
