//! Workflow route handlers.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use nebula_core::WorkflowId;
use nebula_ports::{PortsError, WorkflowRepo};
use std::sync::Arc;

use crate::{
    auth::Authenticated,
    contracts::{
        ApiErrorResponse, CreateWorkflowRequest, PaginatedResponse, PaginationQuery,
        UpdateWorkflowRequest, WorkflowDetail, WorkflowSummary,
    },
    state::ApiState,
};

const WORKFLOW_LIST_DEFAULT_OFFSET: usize = 0;
const WORKFLOW_LIST_DEFAULT_LIMIT: usize = 50;
const WORKFLOW_LIST_MAX_LIMIT: usize = 200;

fn parse_workflow_id(raw: &str) -> Result<WorkflowId, Response> {
    WorkflowId::parse(raw).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse::new(
                "invalid_workflow_id",
                "workflow id must be a UUID",
            )),
        )
            .into_response()
    })
}

fn workflow_repo(state: &ApiState) -> Result<Arc<dyn WorkflowRepo>, Response> {
    state.workflow_repo.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiErrorResponse::new(
                "workflow_repo_unavailable",
                "workflow repository is not configured",
            )),
        )
            .into_response()
    })
}

fn definition_name(id: &str, definition: &serde_json::Value) -> String {
    definition
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("workflow-{id}"))
}

fn definition_active(definition: &serde_json::Value) -> bool {
    definition
        .get("active")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn definition_updated_at(definition: &serde_json::Value) -> Option<String> {
    definition
        .get("updatedAt")
        .or_else(|| definition.get("updated_at"))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn apply_workflow_name(definition: &mut serde_json::Value, name: String) {
    if let Some(object) = definition.as_object_mut() {
        object.insert("name".to_string(), serde_json::Value::String(name));
        return;
    }

    *definition = serde_json::json!({
        "name": name
    });
}

fn apply_workflow_active(definition: &mut serde_json::Value, active: bool) {
    if let Some(object) = definition.as_object_mut() {
        object.insert("active".to_string(), serde_json::Value::Bool(active));
        return;
    }

    *definition = serde_json::json!({
        "active": active
    });
}

fn map_ports_error(error: PortsError) -> Response {
    match error {
        PortsError::NotFound { entity, id } => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse::new(
                "not_found",
                format!("{entity} not found: {id}"),
            )),
        )
            .into_response(),
        PortsError::Conflict { .. } => (
            StatusCode::CONFLICT,
            Json(ApiErrorResponse::new("conflict", "resource version conflict")),
        )
            .into_response(),
        PortsError::Connection(_)
        | PortsError::Serialization(_)
        | PortsError::Timeout { .. }
        | PortsError::LeaseUnavailable { .. }
        | PortsError::Internal(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResponse::new(
                "internal_error",
                "failed to access workflow storage",
            )),
        )
            .into_response(),
    }
}

pub(crate) async fn list_workflows(
    _auth: Authenticated,
    State(state): State<ApiState>,
    Query(query): Query<PaginationQuery>,
) -> Result<impl IntoResponse, Response> {
    let repo = workflow_repo(&state)?;
    let offset = query.offset.unwrap_or(WORKFLOW_LIST_DEFAULT_OFFSET);
    let limit = query
        .limit
        .unwrap_or(WORKFLOW_LIST_DEFAULT_LIMIT)
        .clamp(1, WORKFLOW_LIST_MAX_LIMIT);

    let rows = repo.list(offset, limit).await.map_err(map_ports_error)?;
    let items = rows
        .into_iter()
        .map(|(id, definition)| {
            let id_str = id.to_string();
            WorkflowSummary {
                id: id_str.clone(),
                name: definition_name(&id_str, &definition),
                active: definition_active(&definition),
                updated_at: definition_updated_at(&definition),
            }
        })
        .collect();

    Ok(Json(PaginatedResponse {
        items,
        offset,
        limit,
    }))
}

pub(crate) async fn get_workflow(
    _auth: Authenticated,
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, Response> {
    let repo = workflow_repo(&state)?;
    let workflow_id = parse_workflow_id(&id)?;
    let Some(definition) = repo.get(workflow_id).await.map_err(map_ports_error)? else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse::new(
                "workflow_not_found",
                format!("workflow {id} was not found"),
            )),
        )
            .into_response());
    };

    Ok(Json(WorkflowDetail {
        id: id.clone(),
        name: definition_name(&id, &definition),
        active: definition_active(&definition),
        definition,
    }))
}

pub(crate) async fn create_workflow(
    _auth: Authenticated,
    State(state): State<ApiState>,
    Json(req): Json<CreateWorkflowRequest>,
) -> Result<impl IntoResponse, Response> {
    let repo = workflow_repo(&state)?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse::new(
                "invalid_workflow_name",
                "workflow name must not be empty",
            )),
        )
            .into_response());
    }

    let id = WorkflowId::new();
    let id_str = id.to_string();
    let mut definition = req.definition;
    apply_workflow_name(&mut definition, name.to_string());
    if definition.get("active").is_none() {
        apply_workflow_active(&mut definition, false);
    }

    repo.save(id, 0, definition.clone())
        .await
        .map_err(map_ports_error)?;

    Ok((
        StatusCode::CREATED,
        Json(WorkflowDetail {
            id: id_str.clone(),
            name: definition_name(&id_str, &definition),
            active: definition_active(&definition),
            definition,
        }),
    ))
}

pub(crate) async fn update_workflow(
    _auth: Authenticated,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateWorkflowRequest>,
) -> Result<impl IntoResponse, Response> {
    let repo = workflow_repo(&state)?;
    let workflow_id = parse_workflow_id(&id)?;

    if req.name.is_none() && req.definition.is_none() && req.active.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse::new(
                "empty_update",
                "at least one field must be provided",
            )),
        )
            .into_response());
    }

    let Some((version, mut definition)) = repo
        .get_with_version(workflow_id)
        .await
        .map_err(map_ports_error)?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse::new(
                "workflow_not_found",
                format!("workflow {id} was not found"),
            )),
        )
            .into_response());
    };

    if let Some(new_definition) = req.definition {
        definition = new_definition;
    }

    if let Some(name) = req.name {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResponse::new(
                    "invalid_workflow_name",
                    "workflow name must not be empty",
                )),
            )
                .into_response());
        }
        apply_workflow_name(&mut definition, trimmed.to_string());
    }

    if let Some(active) = req.active {
        apply_workflow_active(&mut definition, active);
    }

    repo.save(workflow_id, version, definition.clone())
        .await
        .map_err(map_ports_error)?;

    Ok(Json(WorkflowDetail {
        id: id.clone(),
        name: definition_name(&id, &definition),
        active: definition_active(&definition),
        definition,
    }))
}

pub(crate) async fn delete_workflow(
    _auth: Authenticated,
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, Response> {
    let repo = workflow_repo(&state)?;
    let workflow_id = parse_workflow_id(&id)?;
    let deleted = repo.delete(workflow_id).await.map_err(map_ports_error)?;
    if !deleted {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse::new(
                "workflow_not_found",
                format!("workflow {id} was not found"),
            )),
        )
            .into_response());
    }

    Ok(StatusCode::NO_CONTENT)
}
