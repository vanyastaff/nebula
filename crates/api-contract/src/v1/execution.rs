//! Execution DTOs

use std::collections::BTreeMap;

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

/// Operator view of one committed execution snapshot.
///
/// Unlike a command acknowledgement, this includes persisted node evidence.
/// Times use the same RFC 3339 representation as execution history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExecutionDetailResponse {
    /// Identity, lifecycle status and timestamps.
    #[serde(flatten)]
    pub execution: ExecutionSummary,
    /// Storage revision of this snapshot; all included nodes belong to it.
    pub snapshot_version: u64,
    /// Original workflow input, if supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    /// Node evidence ordered by node key, independent of process-local caches.
    pub nodes: BTreeMap<String, ExecutionNode>,
    /// Total retries scheduled by the execution owner.
    pub total_retries: u32,
    /// Output bytes accounted for by the execution owner.
    pub total_output_bytes: u64,
}

/// Persisted lifecycle of one workflow node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExecutionNodeStatus {
    /// Waiting for predecessors.
    Pending,
    /// Eligible for dispatch.
    Ready,
    /// Executing.
    Running,
    /// Finished successfully.
    Completed,
    /// Failed without a pending retry.
    Failed,
    /// Skipped by routing.
    Skipped,
    /// Cancelled.
    Cancelled,
    /// Retry scheduled.
    WaitingRetry,
    /// Parked for a timer or external signal.
    Waiting,
}

/// Node state and its recorded attempts, without internal replay identities.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExecutionNode {
    /// Current lifecycle status.
    pub status: ExecutionNodeStatus,
    /// When the node was scheduled (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_at: Option<String>,
    /// First dispatch time (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Terminal transition time (RFC 3339), when recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// Scheduled retry or parked-wait wake time (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<String>,
    /// Attempts in the order recorded by the execution owner.
    pub attempts: Vec<ExecutionAttempt>,
    /// Current primary output, when recorded. Named output ports are not included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<ExecutionNodeOutput>,
    /// Current safe failure record, when recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ExecutionFailure>,
}

/// One recorded node attempt. Absence of a finish time means it is unfinished.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExecutionAttempt {
    /// One-based attempt number assigned by the execution owner.
    pub attempt_number: u32,
    /// When the owner created this attempt record (RFC 3339). The current engine
    /// records attempts after dispatch resolves; this is not dispatch timing.
    pub recorded_at: String,
    /// Attempt completion time (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// Successful attempt data, when recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<ExecutionNodeOutput>,
    /// Safe failure record, when recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ExecutionFailure>,
    /// Output bytes accounted for by the execution owner.
    pub output_bytes: u64,
}

/// Materialized output. External data is described without exposing storage keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionNodeOutput {
    /// Complete inline data, including primitive and null values.
    Inline {
        /// The recorded value.
        value: serde_json::Value,
    },
    /// Stored externally; this inspection response does not retrieve its content.
    External {
        /// Stored content size in bytes, when known.
        #[serde(skip_serializing_if = "Option::is_none")]
        size: Option<u64>,
        /// Recorded MIME type, when known.
        #[serde(skip_serializing_if = "Option::is_none")]
        mime: Option<String>,
    },
    /// Binary content described without embedding bytes or storage locations.
    Binary {
        /// Content size in bytes.
        size: u64,
        /// Recorded MIME type.
        mime: String,
    },
    /// An ordered collection retaining each item's data kind.
    Collection {
        /// Recorded items.
        #[cfg_attr(feature = "openapi", schema(no_recursion))]
        items: Vec<ExecutionNodeOutput>,
    },
    /// A deferred result; resolution handles and callback credentials stay private.
    Deferred,
    /// Explicitly empty collection item. An empty primary output is omitted.
    Empty,
}

/// Validated framework failure identity; never arbitrary provider error prose.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExecutionFailure {
    /// Machine-readable failure code.
    pub code: String,
    /// Framework failure category.
    pub category: String,
    /// Whether the recorded failure was retryable; not permission to replay effects.
    pub retryable: bool,
    /// Bounded framework-authored diagnostic, if recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Typed source identities, without source error text.
    pub source_codes: Vec<String>,
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
