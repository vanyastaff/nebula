//! Execution DTOs

use std::collections::HashMap;

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

/// Minimal summary returned inside a list of running executions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningExecutionSummary {
    /// Execution ID
    pub id: String,
}

/// List executions response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListExecutionsResponse {
    /// Running executions (summary)
    pub executions: Vec<RunningExecutionSummary>,

    /// Total count (across all pages)
    pub total: usize,

    /// Page number (1-indexed)
    pub page: usize,

    /// Page size
    pub page_size: usize,
}

/// All node outputs for an execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionOutputsResponse {
    /// Execution ID
    pub execution_id: String,

    /// Map of node_key (string) → latest output value
    pub outputs: HashMap<String, serde_json::Value>,
}

/// Execution log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionLogEntry {
    /// Raw journal entry value
    #[serde(flatten)]
    pub data: serde_json::Value,
}

/// Execution logs response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionLogsResponse {
    /// Execution ID
    pub execution_id: String,

    /// Ordered journal entries
    pub logs: Vec<serde_json::Value>,
}
