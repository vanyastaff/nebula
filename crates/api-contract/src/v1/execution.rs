//! Execution DTOs

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Start workflow execution request
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct StartExecutionRequest {
    /// Input data for the workflow
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
}

/// Execution response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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

/// Lifecycle status of an execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    /// Created, not started yet.
    Created,
    /// Running nodes.
    Running,
    /// Paused, waiting for a signal, approval, timer, or webhook.
    Paused,
    /// Cancellation requested; active nodes are draining.
    Cancelling,
    /// Every node completed successfully.
    Completed,
    /// A node failed and the execution could not continue.
    Failed,
    /// Cancelled.
    Cancelled,
    /// The wall-clock budget ran out.
    TimedOut,
}

/// One execution in a history page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExecutionSummary {
    /// Execution ID (`exe_<ULID>`).
    pub id: String,
    /// Workflow ID (`wf_<ULID>`).
    pub workflow_id: String,
    /// Current status.
    pub status: ExecutionStatus,
    /// Creation instant (RFC 3339, microsecond precision).
    pub created_at: String,
    /// When the execution started running (RFC 3339); absent before it ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// When the execution reached a terminal status (RFC 3339); absent
    /// while it is not terminal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// Last state change (RFC 3339).
    pub updated_at: String,
}

/// Filters and cursor for the execution history endpoints.
///
/// Results are ordered newest first. `limit` defaults to 20 and is capped at
/// 100. Unknown parameters — including the retired `page`/`page_size` —
/// are rejected rather than silently ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct ExecutionHistoryParams {
    /// Only executions of this workflow (`wf_<ULID>`). On the workflow-scoped
    /// route it must be absent or equal to the path workflow.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", param(nullable = false))]
    pub workflow_id: Option<String>,
    /// Opaque cursor from a previous response's `next_cursor`.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", param(nullable = false))]
    pub cursor: Option<String>,
    /// Page size, 1..=100 (larger values are capped; default 20).
    #[serde(default)]
    #[cfg_attr(feature = "openapi", param(nullable = false, minimum = 1))]
    pub limit: Option<u32>,
    /// Comma-separated statuses to include, e.g. `failed,timed_out`.
    /// Absent or empty means every status.
    #[serde(default)]
    #[cfg_attr(
        feature = "openapi",
        param(nullable = false, example = "failed,timed_out")
    )]
    pub status: Option<String>,
    /// Only executions created at or after this instant (RFC 3339).
    #[serde(default)]
    #[cfg_attr(feature = "openapi", param(nullable = false))]
    pub created_after: Option<String>,
    /// Only executions created before this instant (RFC 3339, exclusive).
    #[serde(default)]
    #[cfg_attr(feature = "openapi", param(nullable = false))]
    pub created_before: Option<String>,
}

/// One page of execution history, newest first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListExecutionsResponse {
    /// Executions on this page.
    pub items: Vec<ExecutionSummary>,
    /// Opaque cursor for the next page; absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Whether another page exists.
    pub has_more: bool,
}

/// All node outputs for an execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExecutionOutputsResponse {
    /// Execution ID
    pub execution_id: String,

    /// Map of node_key (string) → latest output value
    pub outputs: HashMap<String, serde_json::Value>,
}

/// Execution log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExecutionLogEntry {
    /// Raw journal entry value
    #[serde(flatten)]
    pub data: serde_json::Value,
}

/// Execution logs response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExecutionLogsResponse {
    /// Execution ID
    pub execution_id: String,

    /// Ordered journal entries
    pub logs: Vec<serde_json::Value>,
}
