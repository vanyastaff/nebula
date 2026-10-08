//! Workflow DTOs

use serde::{Deserialize, Deserializer, Serialize};
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

fn deserialize_present_json_value<'de, D>(
    deserializer: D,
) -> Result<Option<serde_json::Value>, D::Error>
where
    D: Deserializer<'de>,
{
    serde_json::Value::deserialize(deserializer).map(Some)
}

/// Create workflow request
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateWorkflowRequest {
    /// Revision observed by the editor. A stale revision returns 409 without writing.
    /// Omitted only by legacy clients, which retain server-side race protection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    /// Workflow name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Workflow definition (JSON)
    #[serde(
        default,
        deserialize_with = "deserialize_present_json_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub definition: Option<serde_json::Value>,
}

/// Workflow response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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

/// Editable workflow snapshot. Revision is the storage CAS counter, not semver.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WorkflowDocumentResponse {
    /// Workflow metadata, flattened for compatibility with metadata-only readers.
    #[serde(flatten)]
    pub workflow: WorkflowResponse,
    /// Full persisted definition, including server-owned identity fields.
    pub definition: serde_json::Value,
    /// Revision to supply as `expected_revision` on the next save or activation.
    pub revision: u64,
}

/// Optional editor revision fence for activation; legacy callers may omit it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct ActivateWorkflowParams {
    /// Activate only the revision the operator reviewed.
    #[serde(default)]
    pub expected_revision: Option<u64>,
}

/// List workflows response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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

/// Workflow validate response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WorkflowValidateResponse {
    /// Whether the workflow definition is valid
    pub valid: bool,

    /// List of human-readable validation error messages (empty when `valid` is `true`)
    pub errors: Vec<String>,
}
