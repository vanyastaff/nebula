//! Shared API request/response contracts.
//!
//! Phase 1 contract baseline:
//! - common error envelope `{ error, message }`
//! - pagination query/response
//! - workflow/run DTOs used by upcoming REST endpoints

use serde::{Deserialize, Serialize};

/// Standard API error shape used on non-2xx responses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiErrorResponse {
    /// Stable machine-readable error code.
    pub error: String,
    /// Human-readable message.
    pub message: String,
}

impl ApiErrorResponse {
    /// Build a new error envelope.
    pub fn new(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            message: message.into(),
        }
    }
}

/// Standard pagination query parameters.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginationQuery {
    /// Number of items to skip.
    pub offset: Option<usize>,
    /// Number of items to return.
    pub limit: Option<usize>,
}

/// Standard paginated response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedResponse<T> {
    /// Page items.
    pub items: Vec<T>,
    /// Request offset used for this page.
    pub offset: usize,
    /// Request limit used for this page.
    pub limit: usize,
}

/// Workflow summary DTO for list endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSummary {
    /// Workflow identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Whether workflow is active.
    pub active: bool,
    /// Last update timestamp (RFC3339).
    pub updated_at: Option<String>,
}

/// Workflow details DTO for get endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDetail {
    /// Workflow identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Whether workflow is active.
    pub active: bool,
    /// Serialized workflow definition (phase-1 compatibility shape).
    pub definition: serde_json::Value,
}

/// Create workflow request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkflowRequest {
    /// Display name.
    pub name: String,
    /// Serialized workflow definition.
    pub definition: serde_json::Value,
}

/// Patch/update workflow request body.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkflowRequest {
    /// Optional new workflow name.
    pub name: Option<String>,
    /// Optional new workflow definition.
    pub definition: Option<serde_json::Value>,
    /// Optional active flag.
    pub active: Option<bool>,
}

/// Run summary DTO for list endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    /// Run identifier.
    pub id: String,
    /// Workflow identifier.
    pub workflow_id: String,
    /// Run status (queued/running/completed/failed/canceled).
    pub status: String,
    /// Start timestamp (RFC3339).
    pub started_at: Option<String>,
    /// End timestamp (RFC3339).
    pub finished_at: Option<String>,
}
