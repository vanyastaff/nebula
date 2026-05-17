//! Workflow DTOs

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Create workflow request
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateWorkflowRequest {
    /// Workflow name
    pub name: String,

    /// Description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Workflow definition (JSON)
    pub definition: serde_json::Value,
}

/// Update workflow request
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateWorkflowRequest {
    /// Workflow name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Workflow definition (JSON)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<serde_json::Value>,
}

/// Workflow response
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkflowResponse {
    /// Workflow ID
    pub id: String,

    /// Workflow name
    pub name: String,

    /// Description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Created at (timestamp)
    pub created_at: i64,

    /// Updated at (timestamp)
    pub updated_at: i64,
}

/// List workflows response
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListWorkflowsResponse {
    /// Workflows
    pub workflows: Vec<WorkflowResponse>,

    /// Total count
    pub total: usize,

    /// Page number
    pub page: usize,

    /// Page size
    pub page_size: usize,
}
