use serde::{Deserialize, Serialize};

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
