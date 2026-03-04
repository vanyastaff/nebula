//! Workflow business service.

use nebula_core::WorkflowId;
use nebula_ports::{PortsError, WorkflowRepo};
use std::sync::Arc;

use crate::{
    contracts::{CreateWorkflowRequest, UpdateWorkflowRequest, WorkflowDetail, WorkflowSummary},
    error::{ApiHttpError, ApiResult},
    state::ApiState,
};

const WORKFLOW_LIST_DEFAULT_OFFSET: usize = 0;
const WORKFLOW_LIST_DEFAULT_LIMIT: usize = 50;
const WORKFLOW_LIST_MAX_LIMIT: usize = 200;

pub(crate) struct WorkflowService {
    repo: Arc<dyn WorkflowRepo>,
}

impl WorkflowService {
    pub(crate) fn from_state(state: &ApiState) -> ApiResult<Self> {
        let repo = state.workflow_repo.clone().ok_or_else(|| {
            ApiHttpError::service_unavailable(
                "workflow_repo_unavailable",
                "workflow repository is not configured",
            )
        })?;
        Ok(Self { repo })
    }

    pub(crate) fn parse_workflow_id(raw: &str) -> ApiResult<WorkflowId> {
        WorkflowId::parse(raw).map_err(|_| {
            ApiHttpError::bad_request("invalid_workflow_id", "workflow id must be a UUID")
        })
    }

    pub(crate) fn normalize_pagination(
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> (usize, usize) {
        let offset = offset.unwrap_or(WORKFLOW_LIST_DEFAULT_OFFSET);
        let limit = limit
            .unwrap_or(WORKFLOW_LIST_DEFAULT_LIMIT)
            .clamp(1, WORKFLOW_LIST_MAX_LIMIT);
        (offset, limit)
    }

    pub(crate) async fn list(
        &self,
        offset: usize,
        limit: usize,
    ) -> ApiResult<Vec<WorkflowSummary>> {
        let rows = self
            .repo
            .list(offset, limit)
            .await
            .map_err(map_ports_error)?;

        Ok(rows
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
            .collect())
    }

    pub(crate) async fn get(&self, id: WorkflowId, id_str: &str) -> ApiResult<WorkflowDetail> {
        let Some(definition) = self.repo.get(id).await.map_err(map_ports_error)? else {
            return Err(ApiHttpError::not_found(
                "workflow_not_found",
                format!("workflow {id_str} was not found"),
            ));
        };

        Ok(WorkflowDetail {
            id: id_str.to_string(),
            name: definition_name(id_str, &definition),
            active: definition_active(&definition),
            definition,
        })
    }

    pub(crate) async fn create(&self, req: CreateWorkflowRequest) -> ApiResult<WorkflowDetail> {
        let name = req.name.trim();
        if name.is_empty() {
            return Err(ApiHttpError::bad_request(
                "invalid_workflow_name",
                "workflow name must not be empty",
            ));
        }

        let id = WorkflowId::new();
        let id_str = id.to_string();
        let mut definition = req.definition;
        apply_workflow_name(&mut definition, name.to_string());
        if definition.get("active").is_none() {
            apply_workflow_active(&mut definition, false);
        }

        self.repo
            .save(id, 0, definition.clone())
            .await
            .map_err(map_ports_error)?;

        Ok(WorkflowDetail {
            id: id_str.clone(),
            name: definition_name(&id_str, &definition),
            active: definition_active(&definition),
            definition,
        })
    }

    pub(crate) async fn update(
        &self,
        id: WorkflowId,
        id_str: &str,
        req: UpdateWorkflowRequest,
    ) -> ApiResult<WorkflowDetail> {
        if req.name.is_none() && req.definition.is_none() && req.active.is_none() {
            return Err(ApiHttpError::bad_request(
                "empty_update",
                "at least one field must be provided",
            ));
        }

        let Some((version, mut definition)) = self
            .repo
            .get_with_version(id)
            .await
            .map_err(map_ports_error)?
        else {
            return Err(ApiHttpError::not_found(
                "workflow_not_found",
                format!("workflow {id_str} was not found"),
            ));
        };

        if let Some(new_definition) = req.definition {
            definition = new_definition;
        }

        if let Some(name) = req.name {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Err(ApiHttpError::bad_request(
                    "invalid_workflow_name",
                    "workflow name must not be empty",
                ));
            }
            apply_workflow_name(&mut definition, trimmed.to_string());
        }

        if let Some(active) = req.active {
            apply_workflow_active(&mut definition, active);
        }

        self.repo
            .save(id, version, definition.clone())
            .await
            .map_err(map_ports_error)?;

        Ok(WorkflowDetail {
            id: id_str.to_string(),
            name: definition_name(id_str, &definition),
            active: definition_active(&definition),
            definition,
        })
    }

    pub(crate) async fn delete(&self, id: WorkflowId, id_str: &str) -> ApiResult<()> {
        let deleted = self.repo.delete(id).await.map_err(map_ports_error)?;
        if !deleted {
            return Err(ApiHttpError::not_found(
                "workflow_not_found",
                format!("workflow {id_str} was not found"),
            ));
        }
        Ok(())
    }
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

fn map_ports_error(error: PortsError) -> ApiHttpError {
    match error {
        PortsError::NotFound { entity, id } => {
            ApiHttpError::not_found("not_found", format!("{entity} not found: {id}"))
        }
        PortsError::Conflict { .. } => {
            ApiHttpError::conflict("conflict", "resource version conflict")
        }
        PortsError::Connection(_)
        | PortsError::Serialization(_)
        | PortsError::Timeout { .. }
        | PortsError::LeaseUnavailable { .. }
        | PortsError::Internal(_) => {
            ApiHttpError::internal("internal_error", "failed to access workflow storage")
        }
    }
}
