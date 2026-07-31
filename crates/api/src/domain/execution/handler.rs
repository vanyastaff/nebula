//! Execution handlers

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use nebula_core::{ExecutionId, TenantContext, WorkflowId};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_storage_port::dto::ControlCommand;
use nebula_storage_port::store::{StartAcceptance, StartFingerprint};

use crate::{
    domain::{
        execution::dto::{
            ExecutionLogsResponse, ExecutionOutputsResponse, ExecutionResponse,
            ListExecutionsResponse, RunningExecutionSummary, StartExecutionRequest,
        },
        shared::PaginationParams,
        workflow::handler::extract_timestamp,
    },
    error::{ApiError, ApiResult, ProblemDetails},
    state::AppState,
    trace_capture::w3c_trace_context_for_control_queue,
};

/// List all executions (workspace-scoped) — returns running execution IDs with count.
///
/// # Errors
///
/// Returns [`ApiError::Internal`] if the execution repository is unavailable.
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/executions",
    tag = "workspaces.executions",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        PaginationParams,
    ),
    responses(
        (status = 200, description = "Page of running execution summaries.", body = ListExecutionsResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 500, description = "Execution repository unavailable.", body = ProblemDetails),
    ),
)]
pub async fn list_executions(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ListExecutionsResponse>> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let running_ids = state.list_running_executions_scoped(&scope).await?;

    let total = running_ids.len();

    // Apply pagination over the running list.
    let offset = params.offset();
    let limit = params.limit();
    let executions: Vec<RunningExecutionSummary> = running_ids
        .iter()
        .skip(offset)
        .take(limit)
        .map(|id| RunningExecutionSummary { id: id.to_string() })
        .collect();

    Ok(Json(ListExecutionsResponse {
        executions,
        total,
        page: params.page,
        page_size: params.limit(),
    }))
}

/// List executions for a workflow — returns running executions for the workflow.
///
/// # Errors
///
/// Returns [`ApiError::Internal`] if the execution repository is unavailable.
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/workflows/{wf}/executions",
    tag = "workspaces.executions",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("wf" = String, Path, description = "Workflow identifier (`wf_<ULID>`)."),
        PaginationParams,
    ),
    responses(
        (status = 200, description = "Page of running execution summaries scoped to this workflow.", body = ListExecutionsResponse),
        (status = 400, description = "Invalid workflow identifier.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 500, description = "Execution repository unavailable.", body = ProblemDetails),
    ),
)]
pub async fn list_executions_for_workflow(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, workflow_id)): Path<(String, String, String)>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ListExecutionsResponse>> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let workflow_id_parsed = WorkflowId::parse(&workflow_id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {e}")))?;

    // Scope the list to the requested workflow (#286, #288, #328) within
    // the caller's tenant — the per-request decorator confines the read,
    // closing the cross-tenant execution-ID leak the global
    // `list_running()` would have allowed.
    let running_ids = state
        .list_running_executions_for_workflow_scoped(&scope, workflow_id_parsed)
        .await?;

    let total = running_ids.len();
    let offset = params.offset();
    let limit = params.limit();
    let executions: Vec<RunningExecutionSummary> = running_ids
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|id| RunningExecutionSummary { id: id.to_string() })
        .collect();

    Ok(Json(ListExecutionsResponse {
        executions,
        total,
        page: params.page,
        page_size: params.limit(),
    }))
}

/// Get all node outputs for an execution.
///
/// Returns a map of `node_key → output_value` for every node that has
/// completed at least one attempt.
///
/// # Errors
///
/// - [`ApiError::Validation`] if `id` is not a valid execution ID.
/// - [`ApiError::NotFound`] if no execution with that ID exists.
/// - [`ApiError::Internal`] if the execution repository is unavailable.
pub async fn get_execution_outputs(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, id)): Path<(String, String, String)>,
) -> ApiResult<Json<ExecutionOutputsResponse>> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let execution_id = ExecutionId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid execution ID: {e}")))?;

    // Verify the execution exists in the caller's tenant before loading
    // outputs.
    state
        .execution_state_scoped(&scope, execution_id, "check")
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Execution {id} not found")))?;

    let outputs = state
        .execution_node_outputs_scoped(&scope, execution_id)
        .await?;

    // Convert NodeKey keys to strings for JSON serialisation.
    let string_outputs: std::collections::HashMap<String, serde_json::Value> = outputs
        .into_iter()
        .map(|(node_key, val)| (node_key.to_string(), val))
        .collect();

    Ok(Json(ExecutionOutputsResponse {
        execution_id: id,
        outputs: string_outputs,
    }))
}

/// Get execution by ID
/// GET /api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/executions/{exec}",
    tag = "workspaces.executions",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("exec" = String, Path, description = "Execution identifier (`exe_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Execution detail.", body = ExecutionResponse),
        (status = 400, description = "Invalid execution identifier.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Execution does not exist.", body = ProblemDetails),
    ),
)]
pub async fn get_execution(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, id)): Path<(String, String, String)>,
) -> ApiResult<Json<ExecutionResponse>> {
    use nebula_core::ExecutionId;

    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    // Parse execution ID
    let execution_id = ExecutionId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid execution ID: {e}")))?;

    // Fetch execution state scoped to the caller's tenant
    let state_result = state
        .execution_state_scoped(&scope, execution_id, "get")
        .await?;

    // Check if execution exists (returns Option<(version, state)>)
    let (_version, execution_state) =
        state_result.ok_or_else(|| ApiError::NotFound(format!("Execution {id} not found")))?;

    // Extract fields from execution state JSON
    let workflow_id = execution_state
        .get("workflow_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let status = execution_state
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    // Canonical `ExecutionState` exposes `started_at` (engine run start,
    // `None` until transitioned to `Running`) and `created_at` (always set
    // at construction). Fall back to `created_at` so the API response
    // retains a meaningful timestamp for executions that have not yet been
    // dispatched (#327).
    let started_at = extract_timestamp(&execution_state, "started_at")
        .or_else(|| extract_timestamp(&execution_state, "created_at"))
        .unwrap_or(0);
    // Canonical engine state uses `completed_at` (see `ExecutionState` in
    // `crates/execution/src/state.rs`); legacy rows used `finished_at`.
    let finished_at = extract_timestamp(&execution_state, "completed_at")
        .or_else(|| extract_timestamp(&execution_state, "finished_at"));

    // Canonical field is `workflow_input`; legacy rows used `input`.
    let input = execution_state
        .get("workflow_input")
        .or_else(|| execution_state.get("input"))
        .cloned();

    let output = execution_state.get("output").cloned();

    Ok(Json(ExecutionResponse {
        id,
        workflow_id,
        status,
        started_at,
        finished_at,
        input,
        output,
    }))
}

/// Start workflow execution (enqueue and return 202 Accepted)
/// POST /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf}/executions
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/workflows/{wf}/executions",
    tag = "workspaces.executions",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("wf" = String, Path, description = "Workflow identifier (`wf_<ULID>`)."),
        (
            "Idempotency-Key" = Option<String>,
            Header,
            description = "Start key identifying one accepted command, 1..=255 printable ASCII \
                           characters. Retrying with the same key and an identical request \
                           returns the original acceptance receipt — same execution id, same \
                           timestamp — and creates nothing new; reusing it for a request that \
                           differs is refused with 409 and no durable change. Omitting it means \
                           every request creates its own execution.",
        ),
    ),
    request_body = StartExecutionRequest,
    responses(
        (status = 202, description = "Execution accepted; engine dispatch in flight. A replayed keyed start returns the original receipt with this same status.", body = ExecutionResponse),
        (status = 400, description = "Invalid workflow identifier, or the stored workflow definition cannot be parsed as a workflow.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Workflow does not exist.", body = ProblemDetails),
        (status = 409, description = "The `Idempotency-Key` is already reserved for a request that canonicalizes differently (`code: operation_mismatch`). Nothing was written.", body = ProblemDetails),
        (status = 422, description = "Workflow definition fails structural validation (shift-left gate).", body = ProblemDetails),
        (status = 503, description = "Control queue is unavailable; the engine cannot pick up the dispatch signal.", body = ProblemDetails),
    ),
)]
pub async fn start_execution(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, workflow_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    Json(payload): Json<StartExecutionRequest>,
) -> ApiResult<(StatusCode, Json<ExecutionResponse>)> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    // Parse workflow ID
    let workflow_id_parsed = WorkflowId::parse(&workflow_id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {e}")))?;

    // Verify the workflow exists in the caller's tenant, then run the
    // shift-left validation gate (ROADMAP M3.6 / canon §10): a structurally
    // invalid definition is rejected with RFC 9457 *before* any execution
    // state is created or any Start signal is enqueued. `enqueue_start_scoped`
    // requires the `ValidatedWorkflow` witness produced here, so the dispatch
    // path is type-prevented from skipping validation.
    let version = state
        .workflow_published_version_scoped(&scope, workflow_id_parsed)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Workflow {workflow_id} not found")))?;
    let definition = version.definition.clone();
    let validated = validate_for_dispatch(&definition)?;

    // Generate new execution ID
    let execution_id = ExecutionId::new();

    // Build the canonical execution state directly from the typed enum so
    // that the persisted row matches the schema the engine's
    // `resume_execution` reads (honest capability contract: public surface must be honored
    // end-to-end). The legacy hand-rolled JSON with `status: "pending"` was
    // a false capability — `ExecutionStatus` has no `Pending` variant, and
    // neither `list_running` (storage filter) nor `ExecutionState::deserialize`
    // (engine resume path) would accept it (#327).
    //
    // `ExecutionState::new` seeds with `ExecutionStatus::Created` — the only
    // correct initial state per the transition table. The node map is empty
    // at API-start time: the dispatcher will populate per-node rows once the
    // workflow is loaded and a plan is built. The workflow input (trigger
    // payload) is attached so resume can feed entry nodes the same value
    // (#311).
    let mut exec_state = ExecutionState::new(execution_id, workflow_id_parsed, &[]);
    exec_state.set_workflow_version_number(version.number);
    if let Some(input) = payload.input.clone() {
        exec_state.set_workflow_input(input);
    }

    let state_json = serde_json::to_value(&exec_state)
        .map_err(|e| ApiError::Internal(format!("serialize execution state: {e}")))?;

    let created_at = exec_state.created_at.timestamp();
    let w3c = w3c_trace_context_for_control_queue();

    // A start key makes this request one *accepted command* rather than one
    // delivery of it. Without a key there is nothing to converge on, so an
    // unkeyed request creates its own execution every time — that is the
    // caller's choice, not a defect.
    // Both the durable identity *and* the acceptance timestamp come from
    // whichever request actually created the execution — see the replay arm.
    let accepted_execution_id = if let Some(start_key) = start_key(&headers)? {
        let fingerprint = start_fingerprint(workflow_id_parsed, version.number, &payload);
        match state
            .accept_keyed_start_scoped(
                &scope,
                &start_key,
                fingerprint,
                execution_id,
                workflow_id_parsed,
                &state_json,
                w3c,
            )
            .await?
        {
            StartAcceptance::Accepted { execution_id } => {
                tracing::debug!(
                    execution_id = %execution_id,
                    node_count = validated.definition().nodes.len(),
                    "execution: keyed start accepted (reservation + aggregate + Start committed)"
                );
                execution_id
            },
            StartAcceptance::Replayed { execution_id } => {
                // The *original* receipt, which means every field comes from
                // the persisted execution — not just its id. Building the rest
                // from the state constructed above would report `created` and
                // this retry's timestamp for an execution that may already be
                // running or finished: a receipt that describes a request that
                // was never accepted.
                let replayed = ExecutionId::parse(&execution_id).map_err(|e| {
                    ApiError::Internal(format!("replayed execution id is not parseable: {e}"))
                })?;
                let (_, persisted) = state
                    .execution_state_scoped(&scope, replayed, "load the replayed receipt")
                    .await?
                    .ok_or_else(|| {
                        // The reservation names an execution that is not there:
                        // report it rather than answer with a fabricated state.
                        ApiError::Internal(
                            "start key reserved for an execution that no longer exists".to_owned(),
                        )
                    })?;
                tracing::info!(
                    execution_id = %execution_id,
                    "execution: keyed start replayed; returning the original receipt"
                );
                return Ok((
                    StatusCode::ACCEPTED,
                    Json(ExecutionResponse {
                        workflow_id,
                        ..execution_receipt(execution_id, &persisted)
                    }),
                ));
            },
            // Deliberately no execution id in the error: the caller proved
            // knowledge of a key, not of the execution behind it.
            StartAcceptance::FingerprintMismatch => return Err(ApiError::StartConflict),
        }
    } else {
        state
            .create_execution_scoped(&scope, execution_id, workflow_id_parsed, state_json)
            .await?;
        enqueue_start_scoped(&state, &scope, execution_id, &validated).await?;
        execution_id.to_string()
    };

    // Build response. `started_at` is omitted on a Created execution —
    // integration seam step 3 forbids synthetic timestamps for fields the engine
    // has not actually populated yet. `ExecutionState::started_at` is
    // `None` until the engine transitions the status to `Running`, and the
    // API response must reflect that.
    //
    // The legacy response returned `chrono::Utc::now().timestamp()` as a
    // placeholder, which conflated "row was created" with "engine started
    // the run" — two different events under lifecycle authority. Downstream tools
    // that graphed `started_at` therefore measured API-enqueue latency, not
    // engine dispatch latency. The DTO field stays `i64` (wire-compatible),
    // but we now return `created_at` as the observable timestamp so clients
    // still get a real time for "when did this execution exist?" — which
    // is what `started_at` was used for in practice pre-fix.
    let response = ExecutionResponse {
        id: accepted_execution_id,
        workflow_id,
        status: exec_state.status.to_string(),
        started_at: created_at,
        finished_at: None,
        input: payload.input,
        output: None,
    };

    Ok((StatusCode::ACCEPTED, Json(response)))
}

/// Read the caller's start key from `Idempotency-Key`.
///
/// Bounded and charset-checked here rather than at the storage boundary: the
/// key becomes a primary-key component, and an unbounded or non-ASCII value is
/// a request defect, not a storage failure.
fn start_key(headers: &HeaderMap) -> ApiResult<Option<String>> {
    /// Long enough for a UUID, a ULID, or a caller's composite key; short
    /// enough that the reservation index stays small.
    const MAX_START_KEY_LEN: usize = 255;

    let Some(raw) = headers.get("idempotency-key") else {
        return Ok(None);
    };
    let key = raw.to_str().map_err(|_| {
        ApiError::validation_message("Idempotency-Key must be printable ASCII".to_owned())
    })?;
    if key.is_empty() || key.len() > MAX_START_KEY_LEN {
        return Err(ApiError::validation_message(format!(
            "Idempotency-Key must be 1..={MAX_START_KEY_LEN} characters"
        )));
    }
    if !key.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(ApiError::validation_message(
            "Idempotency-Key must be printable ASCII without spaces".to_owned(),
        ));
    }
    Ok(Some(key.to_owned()))
}

/// Canonicalization version for [`start_fingerprint`].
///
/// Bump this whenever the canonical form below changes. Two digests are only
/// comparable when they were produced under the same version, so a bump reads
/// as a mismatch rather than as a false match — the fail-closed direction.
const START_FINGERPRINT_VERSION: u16 = 1;

/// Fingerprint what makes two start requests "the same command".
///
/// Covers the workflow identity, the exact published version the request
/// resolved to, and the caller's input. The version is inside the fingerprint
/// on purpose: replaying a key after the workflow was republished is a
/// *different* command, and silently returning the old receipt would hide that.
///
/// The digest is over `serde_json::to_vec` of a canonical array. `Value`
/// serializes maps in sorted key order (`serde_json` uses a `BTreeMap` unless
/// `preserve_order` is on, and this workspace does not enable it), so two
/// requests whose JSON objects differ only in key order agree here.
fn start_fingerprint(
    workflow_id: WorkflowId,
    version_number: u32,
    payload: &StartExecutionRequest,
) -> StartFingerprint {
    use sha2::{Digest as _, Sha256};

    let canonical = serde_json::json!([
        "start",
        workflow_id.to_string(),
        version_number,
        payload.input,
    ]);
    let mut digest = Sha256::new();
    // `Value` is always serializable; a failure here would be a serde defect,
    // not a request defect, so feed the debug form rather than fail the
    // request on an impossible branch.
    if let Ok(bytes) = serde_json::to_vec(&canonical) {
        digest.update(&bytes);
    } else {
        digest.update(format!("{canonical:?}").as_bytes());
    }
    StartFingerprint::new(START_FINGERPRINT_VERSION, digest.finalize().into())
}

/// Enqueue a `ControlCommand::Start` onto the durable control queue for
/// the caller's tenant (durable control queue, integration seam step 3, #332).
///
/// Shared by `start_execution` (this module) and `execute_workflow`
/// (`handlers::workflow`) so the dispatch contract lives in exactly one
/// place. Any future start-path entry point MUST route through this
/// helper to preserve the honest capability invariant that "persist a row" and
/// "dispatch to the engine" travel together. Stamps the Start control
/// row with the request tenant `scope` via `enqueue_control_scoped`.
///
/// Returns `ApiError::ServiceUnavailable` when the control-queue backend
/// is down (mirrors the 503 contract in `cancel_execution` — integration seam
/// step 6) and `ApiError::Internal` for other write failures so the
/// caller can retry. The engine-side consumer guards against
/// double-start via CAS (control-queue CAS), so a retry after a partial
/// failure is safe.
///
/// M3.5: stamps optional [`nebula_core::W3cTraceContext`] on the row from the active HTTP span
/// when the global propagator yields a valid carrier; otherwise enqueues without one (never
/// fails the request for trace stamping alone).
///
/// M3.6: takes a [`nebula_workflow::ValidatedWorkflow`] witness by reference.
/// The witness can only be produced by [`validate_for_dispatch`] (which runs
/// `validate_workflow`), so the type system forbids reaching dispatch with an
/// unvalidated definition — this is the structural "lint gate" against a
/// future start-path handler that forgets to shift-left validate.
pub(crate) async fn enqueue_start_scoped(
    state: &AppState,
    scope: &nebula_storage_port::Scope,
    execution_id: ExecutionId,
    validated: &nebula_workflow::ValidatedWorkflow,
) -> ApiResult<()> {
    let w3c_trace_context = w3c_trace_context_for_control_queue();
    tracing::debug!(
        execution_id = %execution_id,
        command = ControlCommand::Start.as_str(),
        has_trace_context = w3c_trace_context.is_some(),
        node_count = validated.definition().nodes.len(),
        "execution: enqueue Start on control queue (shift-left validated)"
    );
    state
        .enqueue_control_scoped(
            scope,
            ControlCommand::Start,
            execution_id,
            w3c_trace_context,
        )
        .await
}

/// Parse a stored workflow definition blob and run the shift-left structural
/// validation gate, returning a [`nebula_workflow::ValidatedWorkflow`] dispatch
/// witness or an RFC 9457 error (canon §10 / §12.2, ROADMAP M3.6).
///
/// Every start-path handler (`execute_workflow`, `start_execution`) MUST turn
/// the stored definition into a `ValidatedWorkflow` via this helper *before* it
/// creates an execution row or enqueues a Start signal. Because
/// [`enqueue_start_scoped`] requires the witness, the compiler rejects any
/// dispatch path that skips this call.
///
/// Error mapping:
/// - A blob that cannot be parsed as a `WorkflowDefinition` → **400** via
///   [`ApiError::validation_message`] (a request-level / format error), using
///   the same `to_string`→`from_str` round-trip `activate_workflow` relies on
///   (`from_value` cannot zero-copy-borrow `&str` for `Key<T>` fields, #343).
/// - A parseable-but-structurally-invalid definition → **422**
///   [`ApiError::InvalidWorkflowDefinition`], carrying every typed
///   [`nebula_workflow::WorkflowError`] so the problem+json body gets
///   field-level RFC 6901 pointers.
pub(crate) fn validate_for_dispatch(
    definition: &serde_json::Value,
) -> ApiResult<nebula_workflow::ValidatedWorkflow> {
    let raw_json = serde_json::to_string(definition)
        .map_err(|e| ApiError::Internal(format!("Failed to serialize workflow definition: {e}")))?;
    let workflow_def: nebula_workflow::WorkflowDefinition = serde_json::from_str(&raw_json)
        .map_err(|e| {
            ApiError::validation_message(format!(
                "Workflow definition cannot be parsed as WorkflowDefinition: {e}"
            ))
        })?;
    nebula_workflow::ValidatedWorkflow::validate(workflow_def).map_err(|errors| {
        ApiError::InvalidWorkflowDefinition {
            detail: format!("Workflow definition is invalid ({} error(s))", errors.len()),
            errors,
        }
    })
}

/// Cancel execution
/// DELETE /api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}
#[utoipa::path(
    delete,
    path = "/orgs/{org}/workspaces/{ws}/executions/{exec}",
    tag = "workspaces.executions",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("exec" = String, Path, description = "Execution identifier (`exe_<ULID>`)."),
    ),
    responses(
        (status = 202, description = "Cancellation accepted and durably enqueued. The body reports the execution as it stands right now; the runtime performs the transition to `cancelled` under its own lease, so poll the execution to observe it.", body = ExecutionResponse),
        (status = 400, description = "Invalid execution identifier or already in a terminal state.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Execution does not exist.", body = ProblemDetails),
        (status = 409, description = "Concurrent modification detected.", body = ProblemDetails),
        (status = 500, description = "Failed to enqueue the control command.", body = ProblemDetails),
    ),
)]
pub async fn cancel_execution(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, id)): Path<(String, String, String)>,
) -> ApiResult<(StatusCode, Json<ExecutionResponse>)> {
    use nebula_core::ExecutionId;

    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    // Parse execution ID
    let execution_id = ExecutionId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid execution ID: {e}")))?;

    // Fetch current execution state scoped to the caller's tenant
    let state_result = state
        .execution_state_scoped(&scope, execution_id, "get")
        .await?;

    // Check if execution exists
    let (_version, execution_state) =
        state_result.ok_or_else(|| ApiError::NotFound(format!("Execution {id} not found")))?;

    // Check if execution is already in a terminal state
    let current_status = execution_state
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if matches!(
        current_status,
        "completed" | "failed" | "cancelled" | "timed_out"
    ) {
        return Err(ApiError::validation_message(format!(
            "Cannot cancel execution in '{current_status}' state"
        )));
    }

    // Duplicate Cancel is idempotent: the command is already in flight and
    // runtime control owns the outcome, so re-requesting it must not enqueue a
    // second command row. Report the state as it stands.
    if current_status == ExecutionStatus::Cancelling.to_string() {
        tracing::debug!(
            execution_id = %execution_id,
            "execution: cancellation already requested; returning the in-flight state"
        );
        return Ok((
            StatusCode::ACCEPTED,
            Json(execution_receipt(id, &execution_state)),
        ));
    }

    // Submit the intent; write nothing.
    //
    // The execution aggregate has exactly one writer — the runtime, holding
    // the lease and the fencing token that proves it. This handler holds
    // neither. It used to commit the status transition itself, reconstructing
    // a fencing token out of the generation it had just *read*; a token
    // rebuilt from a read is not proof of anything, and it let an API request
    // land a write that a live runner's fence was supposed to exclude.
    //
    // So the boundary is the control queue, exactly as it is for Resume: the
    // API authorizes the cancel and records durable intent, and the runtime
    // performs the `Running → Cancelling → Cancelled` transition under its own
    // lease once it has actually honored the command.
    let w3c_trace_context = w3c_trace_context_for_control_queue();
    tracing::debug!(
        execution_id = %execution_id,
        command = ControlCommand::Cancel.as_str(),
        has_trace_context = w3c_trace_context.is_some(),
        "execution: enqueue Cancel control command"
    );
    state
        .enqueue_control_scoped(
            &scope,
            ControlCommand::Cancel,
            execution_id,
            w3c_trace_context,
        )
        .await?;

    // 202 with the state as it stands: the cancel is accepted, not done. The
    // reported `status` is whatever is durably true right now, so a client that
    // polls it observes the runtime's own transition rather than a status this
    // handler asserted on the runtime's behalf.
    Ok((
        StatusCode::ACCEPTED,
        Json(execution_receipt(id, &execution_state)),
    ))
}

/// Build a response describing an execution exactly as it is persisted.
///
/// Used wherever the answer must describe durable state rather than something
/// the handler just constructed — a replayed start receipt and a cancellation
/// acknowledgement both have to report what is stored, not what this request
/// would have created.
fn execution_receipt(id: String, execution_state: &serde_json::Value) -> ExecutionResponse {
    let workflow_id = execution_state
        .get("workflow_id")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_owned();
    let status = execution_state
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_owned();
    // `started_at` is the engine run start and stays `None` until the engine
    // reaches `Running`; `created_at` always exists, so it is the honest
    // fallback for an execution that has not been dispatched yet.
    let started_at = extract_timestamp(execution_state, "started_at")
        .or_else(|| extract_timestamp(execution_state, "created_at"))
        .unwrap_or(0);
    // Canonical field is `completed_at`; legacy rows used `finished_at`. Both
    // stay absent until the engine has actually finished.
    let finished_at = extract_timestamp(execution_state, "completed_at")
        .or_else(|| extract_timestamp(execution_state, "finished_at"));

    ExecutionResponse {
        id,
        workflow_id,
        status,
        started_at,
        finished_at,
        // Canonical field is `workflow_input`; legacy rows used `input`.
        input: execution_state
            .get("workflow_input")
            .or_else(|| execution_state.get("input"))
            .cloned(),
        output: execution_state.get("output").cloned(),
    }
}

/// Return journal (log) entries for an execution.
///
/// Journal entries are appended by the engine as execution progresses.
/// Each entry is an arbitrary JSON object — the shape is engine-defined.
///
/// # Errors
///
/// - [`ApiError::Validation`] if `id` is not a valid execution ID.
/// - [`ApiError::NotFound`] if no execution with that ID exists.
/// - [`ApiError::Internal`] if the execution repository is unavailable.
pub async fn get_execution_logs(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, id)): Path<(String, String, String)>,
) -> ApiResult<Json<ExecutionLogsResponse>> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let execution_id = ExecutionId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid execution ID: {e}")))?;

    // Verify the execution exists in the caller's tenant before loading
    // the journal.
    state
        .execution_state_scoped(&scope, execution_id, "check")
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Execution {id} not found")))?;

    let logs = state.execution_journal_scoped(&scope, execution_id).await?;

    Ok(Json(ExecutionLogsResponse {
        execution_id: id,
        logs,
    }))
}

/// Terminate execution — forced shutdown.
/// POST /api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}/terminate
///
/// Forced-terminate is a *forced* shutdown contrasted with
/// [`cancel_execution`]'s *cooperative* drain. Per cooperative cancel the engine
/// has no distinct forced-shutdown path today: `ControlCommand::Terminate`
/// is wired end-to-end (`ControlConsumer` → `EngineControlDispatch::
/// dispatch_terminate` → `dispatch_cancel` → the engine cancel registry's
/// live `CancellationToken`), and in-flight work aborts via the same
/// cooperative token that `Cancel` trips. The operator-visible terminal
/// state is therefore `ExecutionStatus::Cancelled` — `ExecutionStatus`
/// has no distinct `Terminated` variant (see
/// `crates/execution/src/state.rs` / `status.rs`), so pre-setting any
/// other status string would be a #327 / honest capability contract false capability the
/// engine would not round-trip. This mirrors `cancel_execution` exactly
/// except for the durable command kind.
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/executions/{exec}/terminate",
    tag = "workspaces.executions",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("exec" = String, Path, description = "Execution identifier (`exe_<ULID>`)."),
    ),
    responses(
        (status = 202, description = "Termination accepted and durably enqueued. `Terminate` is a cooperative-cancel synonym — the engine has no forced-shutdown path — so the run has not stopped yet; the body reports the execution as it stands and the runtime terminalizes it under its own lease.", body = ExecutionResponse),
        (status = 400, description = "Invalid execution identifier or already in a terminal state.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Execution does not exist.", body = ProblemDetails),
        (status = 409, description = "Concurrent modification detected.", body = ProblemDetails),
        (status = 500, description = "Failed to enqueue the control command.", body = ProblemDetails),
    ),
)]
pub async fn terminate_execution(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, id)): Path<(String, String, String)>,
) -> ApiResult<(StatusCode, Json<ExecutionResponse>)> {
    use nebula_core::ExecutionId;

    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    // Parse execution ID
    let execution_id = ExecutionId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid execution ID: {e}")))?;

    // Fetch current execution state through the scoped storage port
    // (same accessor the port-rewired `get_execution` / `cancel_execution`
    // use), confined to the caller's tenant.
    let state_result = state
        .execution_state_scoped(&scope, execution_id, "get")
        .await?;

    // Check if execution exists
    let (_version, execution_state) =
        state_result.ok_or_else(|| ApiError::NotFound(format!("Execution {id} not found")))?;

    // Check if execution is already in a terminal state
    let current_status = execution_state
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if matches!(
        current_status,
        "completed" | "failed" | "cancelled" | "timed_out"
    ) {
        return Err(ApiError::validation_message(format!(
            "Cannot terminate execution in '{current_status}' state"
        )));
    }

    // Submit the intent; write nothing — same boundary as cooperative cancel.
    //
    // This handler used to write `Cancelled` **and** a `completed_at` of its
    // own making, then commit both under a fencing token rebuilt from the
    // generation it had just read. Every part of that was a claim it was not
    // entitled to make: the engine has no forced-shutdown path (`Terminate` is
    // a cooperative-cancel synonym), so at this instant the run has not
    // stopped, nothing has completed, and the runtime — not this request —
    // holds the lease that authorizes the write.
    //
    // The runtime performs the transition to the terminal `Cancelled` under
    // its own lease once it has honored the command, and stamps the completion
    // time then.
    let w3c_trace_context = w3c_trace_context_for_control_queue();
    tracing::debug!(
        execution_id = %execution_id,
        command = ControlCommand::Terminate.as_str(),
        has_trace_context = w3c_trace_context.is_some(),
        "execution: enqueue Terminate control command"
    );
    state
        .enqueue_control_scoped(
            &scope,
            ControlCommand::Terminate,
            execution_id,
            w3c_trace_context,
        )
        .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(execution_receipt(id, &execution_state)),
    ))
}

/// Restart execution from the beginning.
/// POST /api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}/restart
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/executions/{exec}/restart",
    tag = "workspaces.executions",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("exec" = String, Path, description = "Execution identifier (`exe_<ULID>`)."),
    ),
    responses(
        (status = 501, description = "Not yet implemented; tracked under engine execution-restart semantics milestone. Planned response carries the new execution identifier.", body = ExecutionResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 404, description = "Execution does not exist.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once engine execution-restart milestone closes.")]
pub async fn restart_execution(
    State(_state): State<AppState>,
    Extension(_tenant): Extension<TenantContext>,
    Path((_org, _ws, _exec)): Path<(String, String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Restart a failed/cancelled execution
    Err(ApiError::NotImplemented(
        "handler stub — tracked under stub endpoint policy".to_string(),
    ))
}
