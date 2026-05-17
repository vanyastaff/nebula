//! Execution handlers

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use nebula_core::{ExecutionId, TenantContext, WorkflowId};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_storage::repos::ControlCommand;

use crate::{
    errors::{ApiError, ApiResult, ProblemDetails},
    handlers::workflow::{PaginationParams, extract_timestamp},
    models::{
        AckResponse, ExecutionLogsResponse, ExecutionOutputsResponse, ExecutionResponse,
        ListExecutionsResponse, RunningExecutionSummary, StartExecutionRequest,
    },
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
    Extension(_tenant): Extension<TenantContext>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ListExecutionsResponse>> {
    let running_ids = state.list_running_executions().await?;

    let total = running_ids.len();

    // Apply pagination over the running list.
    let offset = params.offset();
    let limit = params.limit();
    let page_ids: Vec<&ExecutionId> = running_ids.iter().skip(offset).take(limit).collect();

    let executions: Vec<RunningExecutionSummary> = page_ids
        .into_iter()
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
    Extension(_tenant): Extension<TenantContext>,
    Path((_org, _ws, workflow_id)): Path<(String, String, String)>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ListExecutionsResponse>> {
    let workflow_id_parsed = WorkflowId::parse(&workflow_id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {e}")))?;

    // Scope the list to the requested workflow (#286, #288, #328). Using the
    // global `list_running()` here would leak execution IDs from every other
    // workflow on the instance — a contained info leak today (shared-trust
    // JWT) but a tenant-crossing read the moment real multi-tenant auth
    // lands.
    let running_ids = state
        .list_running_executions_for_workflow(workflow_id_parsed)
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
    Extension(_tenant): Extension<TenantContext>,
    Path((_org, _ws, id)): Path<(String, String, String)>,
) -> ApiResult<Json<ExecutionOutputsResponse>> {
    let execution_id = ExecutionId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid execution ID: {e}")))?;

    // Verify the execution exists before loading outputs.
    state
        .execution_state(execution_id, "check")
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Execution {id} not found")))?;

    let outputs = state.execution_node_outputs(execution_id).await?;

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
    Extension(_tenant): Extension<TenantContext>,
    Path((_org, _ws, id)): Path<(String, String, String)>,
) -> ApiResult<Json<ExecutionResponse>> {
    use nebula_core::ExecutionId;

    // Parse execution ID
    let execution_id = ExecutionId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid execution ID: {e}")))?;

    // Fetch execution state from repository
    let state_result = state.execution_state(execution_id, "get").await?;

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
    ),
    request_body = StartExecutionRequest,
    responses(
        (status = 202, description = "Execution accepted; engine dispatch in flight.", body = ExecutionResponse),
        (status = 400, description = "Invalid workflow identifier.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Workflow does not exist.", body = ProblemDetails),
        (status = 503, description = "Control queue is unavailable; the engine cannot pick up the dispatch signal.", body = ProblemDetails),
    ),
)]
pub async fn start_execution(
    State(state): State<AppState>,
    Extension(_tenant): Extension<TenantContext>,
    Path((_org, _ws, workflow_id)): Path<(String, String, String)>,
    Json(payload): Json<StartExecutionRequest>,
) -> ApiResult<(StatusCode, Json<ExecutionResponse>)> {
    // Parse workflow ID
    let workflow_id_parsed = WorkflowId::parse(&workflow_id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {e}")))?;

    // Verify workflow exists via the accessor (dual-dispatch: scoped
    // spec-16 stores when wired, else the legacy `WorkflowRepo`).
    state
        .workflow_definition(workflow_id_parsed)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Workflow {workflow_id} not found")))?;

    // Generate new execution ID
    let execution_id = ExecutionId::new();

    // Build the canonical execution state directly from the typed enum so
    // that the persisted row matches the schema the engine's
    // `resume_execution` reads (canon §4.5: public surface must be honored
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
    if let Some(input) = payload.input.clone() {
        exec_state.set_workflow_input(input);
    }

    let state_json = serde_json::to_value(&exec_state)
        .map_err(|e| ApiError::Internal(format!("serialize execution state: {e}")))?;

    // Create execution record. We must call `create` here — the previous
    // implementation called `transition(id, expected_version = 0, ...)`,
    // which is a CAS UPDATE that can never match a brand-new ID (no row
    // exists yet), so every call returned `Ok(false)` and the handler
    // surfaced an Internal error unconditionally.
    state
        .create_execution(execution_id, workflow_id_parsed, state_json)
        .await?;

    // Enqueue the Start signal onto the durable control queue (canon §12.2,
    // §13 step 3, #332). Before this PR the API persisted the row but never
    // dispatched it — the §4.5 violation ("advertise capability engine
    // doesn't deliver end-to-end"). The engine-side `ControlConsumer`
    // (ADR-0008) drains this queue and drives the actual workflow run.
    //
    // Order matches the `cancel_execution` contract: create the row first,
    // then enqueue. If enqueue fails after a successful create, the row
    // exists but the engine will not see the Start signal — the handler
    // fails loudly so the caller can retry. The retry is idempotent
    // at the consumer layer via CAS (ADR-0008 §5).
    enqueue_start(&state, execution_id).await?;

    // Build response. `started_at` is omitted on a Created execution —
    // canon §13 step 3 forbids synthetic timestamps for fields the engine
    // has not actually populated yet. `ExecutionState::started_at` is
    // `None` until the engine transitions the status to `Running`, and the
    // API response must reflect that.
    //
    // The legacy response returned `chrono::Utc::now().timestamp()` as a
    // placeholder, which conflated "row was created" with "engine started
    // the run" — two different events under canon §11.1. Downstream tools
    // that graphed `started_at` therefore measured API-enqueue latency, not
    // engine dispatch latency. The DTO field stays `i64` (wire-compatible),
    // but we now return `created_at` as the observable timestamp so clients
    // still get a real time for "when did this execution exist?" — which
    // is what `started_at` was used for in practice pre-fix.
    let created_at = exec_state.created_at.timestamp();
    let response = ExecutionResponse {
        id: execution_id.to_string(),
        workflow_id,
        status: exec_state.status.to_string(),
        started_at: created_at,
        finished_at: None,
        input: payload.input,
        output: None,
    };

    Ok((StatusCode::ACCEPTED, Json(response)))
}

/// Enqueue a `ControlCommand::Start` onto the durable control queue (canon
/// §12.2, §13 step 3, #332).
///
/// Shared by `start_execution` (this module) and `execute_workflow`
/// (`handlers::workflow`) so the dispatch contract lives in exactly one
/// place. Any future start-path entry point MUST route through this helper
/// to preserve the §4.5 invariant that "persist a row" and "dispatch to the
/// engine" travel together.
///
/// Returns `ApiError::ServiceUnavailable` when the control-queue backend
/// is down (mirrors the 503 contract in `cancel_execution` — canon §13
/// step 6) and `ApiError::Internal` for other write failures so the caller
/// can retry. The engine-side consumer guards against double-start via CAS
/// (ADR-0008 §5), so a retry after a partial failure is safe.
///
/// M3.5: stamps optional [`nebula_core::W3cTraceContext`] on the row from the active HTTP span
/// when the global propagator yields a valid carrier; otherwise enqueues without one (never
/// fails the request for trace stamping alone).
pub(crate) async fn enqueue_start(state: &AppState, execution_id: ExecutionId) -> ApiResult<()> {
    let w3c_trace_context = w3c_trace_context_for_control_queue();
    tracing::debug!(
        execution_id = %execution_id,
        command = ControlCommand::Start.as_str(),
        has_trace_context = w3c_trace_context.is_some(),
        "execution: enqueue Start on control queue"
    );
    state
        .enqueue_control(ControlCommand::Start, execution_id, w3c_trace_context)
        .await
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
        (status = 200, description = "Execution cancelled; cancel signal enqueued for the engine.", body = ExecutionResponse),
        (status = 400, description = "Invalid execution identifier or already in a terminal state.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Execution does not exist.", body = ProblemDetails),
        (status = 409, description = "Concurrent modification detected.", body = ProblemDetails),
        (status = 503, description = "Control queue is unavailable; the cancel signal cannot reach the engine.", body = ProblemDetails),
    ),
)]
pub async fn cancel_execution(
    State(state): State<AppState>,
    Extension(_tenant): Extension<TenantContext>,
    Path((_org, _ws, id)): Path<(String, String, String)>,
) -> ApiResult<Json<ExecutionResponse>> {
    use nebula_core::ExecutionId;

    // Parse execution ID
    let execution_id = ExecutionId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid execution ID: {e}")))?;

    // Fetch current execution state from repository
    let state_result = state.execution_state(execution_id, "get").await?;

    // Check if execution exists
    let (version, mut execution_state) =
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

    // Update state to cancelled. Write the status as the canonical
    // snake-case string that `ExecutionStatus::Cancelled` serializes to,
    // so that engine-side reads via `ExecutionStatus::deserialize` round-
    // trip cleanly (#327, canon §4.5). Persist `completed_at` (not the
    // legacy `finished_at`) because that is the field `ExecutionState`
    // actually declares — see `crates/execution/src/state.rs`.
    if let Some(state_obj) = execution_state.as_object_mut() {
        state_obj.insert(
            "status".to_string(),
            serde_json::json!(ExecutionStatus::Cancelled.to_string()),
        );

        // Set completed_at timestamp. The canonical `ExecutionState`
        // serializes `Option::None` as `null`, not as an absent field —
        // so `contains_key` alone is not enough; we must also overwrite
        // explicit nulls. RFC 3339 string matches what `DateTime<Utc>`
        // serializes to via serde.
        let needs_write = state_obj
            .get("completed_at")
            .is_none_or(serde_json::Value::is_null);
        if needs_write {
            let now = chrono::Utc::now();
            state_obj.insert(
                "completed_at".to_string(),
                serde_json::json!(now.to_rfc3339()),
            );
        }
    }

    // Apply state transition using CAS.
    //
    // Order: transition first, then enqueue — per canon §12.2 and audit §2.2.
    // If enqueue fails after a successful transition the execution row is
    // already `cancelled` but the engine will not see the signal (orphan).
    // This is documented as a known limitation until a shared transaction
    // wrapper is available across ExecutionRepo and ControlQueueRepo.
    // The handler fails loudly on enqueue failure so the caller can retry.
    let transition_result = state
        .cas_transition(execution_id, version, execution_state.clone())
        .await?;

    if !transition_result {
        return Err(ApiError::Conflict(
            "concurrent modification detected; refetch execution state and retry".to_string(),
        ));
    }

    // Enqueue the Cancel signal to the durable control queue (canon §12.2).
    //
    // This MUST happen immediately after a successful CAS transition. If this
    // call fails, we return a 500 so the caller knows to retry the cancel
    // request — the retry will see the already-cancelled DB row and short-circuit
    // at the terminal-status guard above without re-enqueuing (idempotent).
    //
    // M3.5: same W3C stamping policy as [`enqueue_start`] — operator correlation for Cancel.
    let w3c_trace_context = w3c_trace_context_for_control_queue();
    tracing::debug!(
        execution_id = %execution_id,
        command = ControlCommand::Cancel.as_str(),
        has_trace_context = w3c_trace_context.is_some(),
        "execution: enqueue Cancel on control queue"
    );
    // Canon §13 step 6 503-vs-500 policy is centralized in
    // `enqueue_control`; the cancel row is already committed above, so
    // a failed enqueue still surfaces as the orphan-signal error.
    state
        .enqueue_control(ControlCommand::Cancel, execution_id, w3c_trace_context)
        .await?;

    // Extract fields from updated execution state
    let workflow_id = execution_state
        .get("workflow_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let status = execution_state
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("cancelled")
        .to_string();

    // Canonical `ExecutionState` exposes `started_at` (engine run start,
    // `None` until the engine transitions to `Running`) and `created_at`
    // (always set at construction). Fall back to `created_at` so the API
    // response retains a meaningful timestamp for executions that have
    // not yet been dispatched (#327).
    let started_at = extract_timestamp(&execution_state, "started_at")
        .or_else(|| extract_timestamp(&execution_state, "created_at"))
        .unwrap_or(0);
    // Canonical field is `completed_at`; legacy rows used `finished_at`.
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
    Extension(_tenant): Extension<TenantContext>,
    Path((_org, _ws, id)): Path<(String, String, String)>,
) -> ApiResult<Json<ExecutionLogsResponse>> {
    let execution_id = ExecutionId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid execution ID: {e}")))?;

    // Verify the execution exists before loading the journal.
    state
        .execution_state(execution_id, "check")
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Execution {id} not found")))?;

    let logs = state.execution_journal(execution_id).await?;

    Ok(Json(ExecutionLogsResponse {
        execution_id: id,
        logs,
    }))
}

/// Terminate execution — forceful stop.
/// POST /api/v1/orgs/{org}/workspaces/{ws}/executions/{exec}/terminate
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
        (status = 501, description = "Not yet implemented; tracked under engine terminate-action milestone.", body = AckResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 404, description = "Execution does not exist.", body = ProblemDetails),
        (status = 409, description = "Execution already in a terminal state.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once engine terminate-action milestone closes.")]
pub async fn terminate_execution(
    State(_state): State<AppState>,
    Extension(_tenant): Extension<TenantContext>,
    Path((_org, _ws, _exec)): Path<(String, String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Forcefully terminate execution (kill running nodes)
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
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
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}
