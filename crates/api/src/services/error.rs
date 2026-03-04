//! Service-layer errors independent from HTTP transport.

use nebula_ports::PortsError;

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
    pub(crate) fn from_ports(error: PortsError) -> Self {
        match error {
            PortsError::NotFound { entity, id } => Self::NotFound {
                code: "not_found",
                message: format!("{entity} not found: {id}"),
            },
            PortsError::Conflict { .. } => Self::Conflict {
                code: "conflict",
                message: "resource version conflict".to_string(),
            },
            PortsError::Connection(_)
            | PortsError::Serialization(_)
            | PortsError::Timeout { .. }
            | PortsError::LeaseUnavailable { .. }
            | PortsError::Internal(_) => Self::Internal {
                code: "internal_error",
                message: "failed to access workflow storage".to_string(),
            },
        }
    }
}
