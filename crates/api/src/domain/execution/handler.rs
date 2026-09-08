//! Execution handlers

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use nebula_core::{ExecutionId, TenantContext, WorkflowId};
use nebula_execution::ExecutionStatus;
use nebula_storage_port::dto::ControlCommand;

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
                           returns the original execution's current persisted state with the same \
                           execution id and creates nothing new; reusing it for a request that \
                           differs is refused with 409 and no durable change. Omitting it means \
                           every request creates its own execution.",
        ),
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

    start_workflow(&state, &scope, workflow_id_parsed, &headers, payload).await
}

/// Translate HTTP caller intent into the runtime owner's complete start operation.
pub(crate) async fn start_workflow(
    state: &AppState,
    scope: &nebula_storage_port::Scope,
    workflow_id: WorkflowId,
    headers: &HeaderMap,
    payload: StartExecutionRequest,
) -> ApiResult<(StatusCode, Json<ExecutionResponse>)> {
    let key = start_key(headers)?;
    let service = state
        .workflow_start
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("Workflow start is not configured".into()))?;
    let receipt = service
        .start(
            scope,
            workflow_id,
            payload.input,
            key,
            w3c_trace_context_for_control_queue(),
        )
        .await?;
    let persisted = receipt.state();
    Ok((
        StatusCode::ACCEPTED,
        Json(ExecutionResponse {
            id: persisted.execution_id.to_string(),
            workflow_id: persisted.workflow_id.to_string(),
            status: persisted.status.to_string(),
            // The existing DTO uses creation time until the engine starts the run.
            started_at: persisted
                .started_at
                .unwrap_or(persisted.created_at)
                .timestamp(),
            finished_at: persisted.completed_at.map(|time| time.timestamp()),
            input: persisted.workflow_input.clone(),
            output: None,
        }),
    ))
}

/// Read the caller's start key from `Idempotency-Key`.
///
/// Bounded and charset-checked here rather than at the storage boundary: the
/// key becomes a primary-key component, and an unbounded or non-ASCII value is
/// a request defect, not a storage failure.
fn start_key(headers: &HeaderMap) -> ApiResult<Option<&str>> {
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
    Ok(Some(key))
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
