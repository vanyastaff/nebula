//! Workflow handlers

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::Utc;
use nebula_core::{ExecutionId, WorkflowId};
use nebula_execution::ExecutionState;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    errors::{ApiError, ApiResult},
    models::{
        CreateWorkflowRequest, ExecutionResponse, ListWorkflowsResponse, StartExecutionRequest,
        UpdateWorkflowRequest, WorkflowResponse, WorkflowValidateResponse,
    },
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

/// Pagination query parameters
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    /// Page number (1-indexed)
    #[serde(default = "default_page")]
    pub page: usize,
    /// Page size (default 10, max 100)
    #[serde(default = "default_page_size")]
    pub page_size: usize,
}

fn default_page() -> usize {
    1
}

fn default_page_size() -> usize {
    10
}

impl PaginationParams {
    /// Calculate offset for database query (0-indexed)
    pub fn offset(&self) -> usize {
        self.page.saturating_sub(1).saturating_mul(self.page_size)
    }

    /// Get validated limit (capped at 100)
    pub fn limit(&self) -> usize {
        self.page_size.min(100)
    }
}

/// List workflows
/// GET /api/v1/workflows
pub async fn list_workflows(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ListWorkflowsResponse>> {
    let offset = params.offset();
    let limit = params.limit();

    // Fetch workflows from repository
    let workflows = state
        .workflow_repo
        .list(offset, limit)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to list workflows: {}", e)))?;

    let total = state
        .workflow_repo
        .count()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to count workflows: {}", e)))?;

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
                .map(|s| s.to_string());

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
/// GET /api/v1/workflows/:id
pub async fn get_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<WorkflowResponse>> {
    // Parse workflow ID
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {}", e)))?;

    // Fetch workflow from repository
    let definition = state
        .workflow_repo
        .get(workflow_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get workflow: {}", e)))?;

    // Check if workflow exists
    let definition =
        definition.ok_or_else(|| ApiError::NotFound(format!("Workflow {} not found", id)))?;

    // Extract fields from workflow definition JSON
    let name = definition
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unnamed Workflow")
        .to_string();

    let description = definition
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

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
/// POST /api/v1/workflows
pub async fn create_workflow(
    State(state): State<AppState>,
    Json(payload): Json<CreateWorkflowRequest>,
) -> ApiResult<(StatusCode, Json<WorkflowResponse>)> {
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
    let now = Utc::now().timestamp();

    // Build workflow definition by merging request definition with metadata
    let mut definition = payload.definition.clone();
    if let Some(obj) = definition.as_object_mut() {
        obj.insert("name".to_string(), serde_json::json!(payload.name));
        if let Some(desc) = &payload.description {
            obj.insert("description".to_string(), serde_json::json!(desc));
        }
        obj.insert("created_at".to_string(), serde_json::json!(now));
        obj.insert("updated_at".to_string(), serde_json::json!(now));
    } else {
        // If definition is not an object, wrap it with metadata
        definition = serde_json::json!({
            "name": payload.name,
            "description": payload.description,
            "created_at": now,
            "updated_at": now,
            "definition": definition,
        });
    }

    // Save workflow with version 0 (new workflow)
    state
        .workflow_repo
        .save(workflow_id, 0, definition.clone())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create workflow: {}", e)))?;

    // Build response
    let response = WorkflowResponse {
        id: workflow_id.to_string(),
        name: payload.name,
        description: payload.description,
        created_at: now,
        updated_at: now,
    };

    Ok((StatusCode::CREATED, Json(response)))
}

/// Update workflow
/// PUT /api/v1/workflows/:id
pub async fn update_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateWorkflowRequest>,
) -> ApiResult<Json<WorkflowResponse>> {
    // Parse workflow ID
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {}", e)))?;

    // Get current workflow with version
    let (version, mut definition) = state
        .workflow_repo
        .get_with_version(workflow_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get workflow: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Workflow {} not found", id)))?;

    // Current timestamp — `chrono::Utc::now()` is monotonic through time
    // shifts and does not panic on clocks set before 1970, unlike
    // `SystemTime::duration_since(UNIX_EPOCH).unwrap()`.
    let now = Utc::now().timestamp();

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
        if let Some(new_def) = &payload.definition
            && let Some(new_obj) = new_def.as_object()
        {
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
        obj.insert("updated_at".to_string(), serde_json::json!(now));
    } else {
        return Err(ApiError::Internal(
            "Invalid workflow definition format".to_string(),
        ));
    }

    // Save with optimistic concurrency control
    state
        .workflow_repo
        .save(workflow_id, version, definition.clone())
        .await
        .map_err(|e| {
            use nebula_storage::WorkflowRepoError;
            match e {
                WorkflowRepoError::Conflict { .. } => {
                    ApiError::Conflict("Workflow was modified by another request".to_string())
                },
                _ => ApiError::Internal(format!("Failed to update workflow: {}", e)),
            }
        })?;

    // Extract fields for response
    let name = definition
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unnamed Workflow")
        .to_string();

    let description = definition
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

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
/// DELETE /api/v1/workflows/:id
pub async fn delete_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    // Parse workflow ID
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {}", e)))?;

    // Delete workflow from repository
    let existed = state
        .workflow_repo
        .delete(workflow_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete workflow: {}", e)))?;

    // Return 404 if workflow didn't exist, 204 No Content if it was deleted
    if existed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound(format!("Workflow {} not found", id)))
    }
}

/// Activate workflow
/// POST /api/v1/workflows/:id/activate
pub async fn activate_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<WorkflowResponse>> {
    // Parse workflow ID
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {}", e)))?;

    // Get current workflow with version for optimistic concurrency
    let (version, mut definition) = state
        .workflow_repo
        .get_with_version(workflow_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get workflow: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Workflow {} not found", id)))?;

    // Canon §10 step 2: validate the workflow definition before flipping the
    // active flag.  Invalid definitions are rejected with RFC 9457 422
    // (Unprocessable Entity) — activation must never silently enable a
    // workflow that cannot pass structural validation.
    //
    // NOTE: `serde_json::from_value` cannot zero-copy borrow `&str` from a
    // `Value::String`, which causes failures for types like `domain_key::Key<T>`
    // that use `<&str>::deserialize` on human-readable formats.  Round-tripping
    // through a JSON string (`to_string` → `from_str`) gives a proper streaming
    // deserializer that does support `visit_borrowed_str`, so all key types
    // parse correctly.
    let raw_json = serde_json::to_string(&definition).map_err(|e| {
        ApiError::Internal(format!("Failed to serialize workflow definition: {}", e))
    })?;
    let workflow_def: nebula_workflow::WorkflowDefinition = serde_json::from_str(&raw_json)
        .map_err(|e| {
            ApiError::validation_message(format!(
                "Workflow definition cannot be parsed as WorkflowDefinition: {}",
                e
            ))
        })?;

    let validation_errors = nebula_workflow::validate_workflow(&workflow_def);
    if !validation_errors.is_empty() {
        let detail = format!(
            "Workflow definition is invalid ({} error(s))",
            validation_errors.len()
        );
        return Err(ApiError::InvalidWorkflowDefinition {
            detail,
            errors: validation_errors,
        });
    }

    // Current timestamp — `chrono::Utc::now()` is monotonic through time
    // shifts and does not panic on clocks set before 1970, unlike
    // `SystemTime::duration_since(UNIX_EPOCH).unwrap()`.
    let now = Utc::now().timestamp();

    // Update definition to set active flag
    if let Some(obj) = definition.as_object_mut() {
        obj.insert("active".to_string(), serde_json::json!(true));
        obj.insert("updated_at".to_string(), serde_json::json!(now));
    } else {
        return Err(ApiError::Internal(
            "Invalid workflow definition format".to_string(),
        ));
    }

    // Save with optimistic concurrency control
    state
        .workflow_repo
        .save(workflow_id, version, definition.clone())
        .await
        .map_err(|e| {
            use nebula_storage::WorkflowRepoError;
            match e {
                WorkflowRepoError::Conflict { .. } => {
                    ApiError::Conflict("Workflow was modified by another request".to_string())
                },
                _ => ApiError::Internal(format!("Failed to activate workflow: {}", e)),
            }
        })?;

    // Extract fields for response
    let name = definition
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unnamed Workflow")
        .to_string();

    let description = definition
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

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

/// Execute workflow (enqueue and return 202 Accepted)
/// POST /api/v1/workflows/:id/execute
pub async fn execute_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<StartExecutionRequest>,
) -> ApiResult<(StatusCode, Json<ExecutionResponse>)> {
    // Parse workflow ID
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {}", e)))?;

    // Verify workflow exists
    state
        .workflow_repo
        .get(workflow_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get workflow: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Workflow {} not found", id)))?;

    // Generate new execution ID
    let execution_id = ExecutionId::new();

    // Build the canonical execution state — same rationale as
    // `start_execution` in `handlers/execution.rs` (#327, canon §4.5): the
    // persisted row must match `ExecutionState` so the engine's
    // `resume_execution` can deserialize it, and the status must be the
    // canonical `Created`, not the non-existent `"pending"` that the
    // storage `list_running` filter would also drop.
    let mut exec_state = ExecutionState::new(execution_id, workflow_id, &[]);
    if let Some(input) = payload.input.clone() {
        exec_state.set_workflow_input(input);
    }

    let state_json = serde_json::to_value(&exec_state)
        .map_err(|e| ApiError::Internal(format!("serialize execution state: {}", e)))?;

    // Create execution record via `create` — `transition` is a CAS UPDATE
    // and was hitting zero rows for every brand-new ID, so every call to
    // this handler previously returned a 500.
    state
        .execution_repo
        .create(execution_id, workflow_id, state_json)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create execution: {}", e)))?;

    // Report `created_at` as the observable timestamp — the engine has not
    // transitioned `started_at` yet (that happens at dispatch time). See
    // the parallel comment in `handlers::execution::start_execution` for
    // the full rationale.
    let created_at = exec_state.created_at.timestamp();
    let response = ExecutionResponse {
        id: execution_id.to_string(),
        workflow_id: id,
        status: exec_state.status.to_string(),
        started_at: created_at,
        finished_at: None,
        input: payload.input,
        output: None,
    };

    Ok((StatusCode::ACCEPTED, Json(response)))
}

/// Validate workflow definition without executing it.
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
///
/// # Errors
///
/// - [`ApiError::Validation`] if `id` is not a valid workflow ID or the definition cannot be
///   parsed.
/// - [`ApiError::NotFound`] if the workflow does not exist.
/// - [`ApiError::Internal`] if the repository is unavailable.
pub async fn validate_workflow_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<WorkflowValidateResponse>> {
    let workflow_id = WorkflowId::parse(&id)
        .map_err(|e| ApiError::validation_message(format!("Invalid workflow ID: {}", e)))?;

    let definition = state
        .workflow_repo
        .get(workflow_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to get workflow: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("Workflow {} not found", id)))?;

    // Deserialise the stored JSON into a WorkflowDefinition.
    let workflow_def: nebula_workflow::WorkflowDefinition = serde_json::from_value(definition)
        .map_err(|e| {
            ApiError::validation_message(format!(
                "Workflow definition cannot be parsed as WorkflowDefinition: {}",
                e
            ))
        })?;

    let errors = nebula_workflow::validate_workflow(&workflow_def);
    if errors.is_empty() {
        Ok(Json(WorkflowValidateResponse {
            valid: true,
            errors: vec![],
        }))
    } else {
        let error_messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        Ok(Json(WorkflowValidateResponse {
            valid: false,
            errors: error_messages,
        }))
    }
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
