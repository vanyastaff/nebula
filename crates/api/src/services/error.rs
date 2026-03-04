//! Service-layer errors independent from HTTP transport.

use nebula_storage::{ExecutionRepoError, WorkflowRepoError};

/// Canonical result type for services.
pub(crate) type ServiceResult<T> = Result<T, ServiceError>;

/// Service error categories.
#[derive(Debug)]
pub(crate) enum ServiceError {
    /// Invalid input provided to the service.
    InvalidInput { code: &'static str, message: String },
    /// Entity was not found.
    NotFound { code: &'static str, message: String },
    /// Optimistic concurrency conflict.
    Conflict { code: &'static str, message: String },
    /// Internal service/storage failure.
    Internal { code: &'static str, message: String },
}

impl ServiceError {
    pub(crate) fn from_workflow_repo(error: WorkflowRepoError) -> Self {
        match error {
            WorkflowRepoError::NotFound { entity, id } => Self::NotFound {
                code: "not_found",
                message: format!("{entity} not found: {id}"),
            },
            WorkflowRepoError::Conflict { .. } => Self::Conflict {
                code: "conflict",
                message: "resource version conflict".to_string(),
            },
            WorkflowRepoError::Connection(_)
            | WorkflowRepoError::Serialization(_)
            | WorkflowRepoError::Internal(_) => Self::Internal {
                code: "internal_error",
                message: "failed to access workflow storage".to_string(),
            },
        }
    }

    pub(crate) fn from_execution_repo(error: ExecutionRepoError) -> Self {
        match error {
            ExecutionRepoError::NotFound { entity, id } => Self::NotFound {
                code: "not_found",
                message: format!("{entity} not found: {id}"),
            },
            ExecutionRepoError::Conflict { .. } => Self::Conflict {
                code: "conflict",
                message: "resource version conflict".to_string(),
            },
            ExecutionRepoError::Connection(_)
            | ExecutionRepoError::Serialization(_)
            | ExecutionRepoError::Timeout { .. }
            | ExecutionRepoError::LeaseUnavailable { .. }
            | ExecutionRepoError::Internal(_) => Self::Internal {
                code: "internal_error",
                message: "failed to access execution storage".to_string(),
            },
        }
    }
}
