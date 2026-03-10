//! Execution DTOs

use serde::{Deserialize, Serialize};

/// Start workflow execution request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartExecutionRequest {
    /// Input data for the workflow
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
}

/// Execution response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResponse {
    /// Execution ID
    pub id: String,

    /// Workflow ID
    pub workflow_id: String,

    /// Status
    pub status: String,

    /// Started at (timestamp)
    pub started_at: i64,

    /// Finished at (timestamp)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,

    /// Input data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,

    /// Output data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
}

/// List executions response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListExecutionsResponse {
    /// Executions
    pub executions: Vec<ExecutionResponse>,

    /// Total count
    pub total: usize,

    /// Page number
    pub page: usize,

    /// Page size
    pub page_size: usize,
}
