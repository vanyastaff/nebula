//! Deliberate operator projection of the execution owner's committed snapshot.
//!
//! Decode once through the canonical domain model. Do not recover individual
//! fields with defaults: an unreadable snapshot is a failure, not an empty run.

use chrono::{DateTime, SecondsFormat, Utc};
use nebula_execution::{ErrorEnvelope, ExecutionOutput, ExecutionState, NodeExecutionState};
use nebula_storage_port::dto::ExecutionRecord;
use serde::Deserialize;

use super::{dto::*, history};
use crate::error::{ApiError, ApiResult};

#[derive(Debug, thiserror::Error)]
enum InspectionError {
    #[error("execution snapshot has an invalid shape")]
    Shape,
    #[error("execution snapshot identities disagree with its record")]
    Identity,
    #[error("execution snapshot status disagrees with its record")]
    Status,
    #[error("execution snapshot has an unsupported node status")]
    NodeStatus,
    #[error("execution snapshot has invalid checkpoint evidence")]
    Checkpoint,
    #[error("execution snapshot has an unsupported output kind")]
    OutputKind,
}

impl From<InspectionError> for ApiError {
    fn from(error: InspectionError) -> Self {
        tracing::error!(reason = %error, "execution inspection rejected stored snapshot");
        Self::Internal("Stored execution snapshot is invalid".into())
    }
}

#[tracing::instrument(name = "execution.inspect", skip_all, fields(execution_id = %record.id))]
pub(super) fn detail(record: ExecutionRecord) -> ApiResult<ExecutionDetailResponse> {
    // Borrow the JSON deserializer: ErrorEnvelope's category accepts borrowed
    // strings, so from_value (which requires owned-only decoding) is unsuitable.
    let state = ExecutionState::deserialize(&record.state).map_err(|_| InspectionError::Shape)?;
    let same_execution = state.execution_id == record.id;
    let same_workflow = state.workflow_id == record.workflow_id;
    if !same_execution || !same_workflow {
        return Err(InspectionError::Identity.into());
    }
    if state.status.to_string() != record.status.as_str() {
        return Err(InspectionError::Status.into());
    }
    let mut outputs = nebula_engine::inspect_execution_outputs(&state)
        .map_err(|_| InspectionError::Checkpoint)?;
    let nodes = state
        .node_states
        .into_iter()
        .map(|(key, state)| node(state, outputs.remove(&key)).map(|node| (key.to_string(), node)))
        .collect::<Result<_, _>>()?;
    Ok(ExecutionDetailResponse {
        execution: ExecutionSummary {
            id: record.id,
            workflow_id: record.workflow_id,
            status: history::wire_status(record.status),
            created_at: instant(record.created_at),
            updated_at: instant(record.updated_at),
            started_at: state.started_at.map(instant),
            finished_at: state.completed_at.map(instant),
        },
        snapshot_version: record.version,
        input: state.workflow_input,
        nodes,
        total_retries: state.total_retries,
        total_output_bytes: state.total_output_bytes,
    })
}

fn node(
    state: NodeExecutionState,
    recorded_output: Option<nebula_action::ActionOutput<serde_json::Value>>,
) -> Result<ExecutionNode, InspectionError> {
    use nebula_workflow::NodeState as S;
    let status = match state.state {
        S::Pending => ExecutionNodeStatus::Pending,
        S::Ready => ExecutionNodeStatus::Ready,
        S::Running => ExecutionNodeStatus::Running,
        S::Completed => ExecutionNodeStatus::Completed,
        S::Failed => ExecutionNodeStatus::Failed,
        S::Skipped => ExecutionNodeStatus::Skipped,
        S::Cancelled => ExecutionNodeStatus::Cancelled,
        S::WaitingRetry => ExecutionNodeStatus::WaitingRetry,
        S::Waiting => ExecutionNodeStatus::Waiting,
        _ => return Err(InspectionError::NodeStatus),
    };
    Ok(ExecutionNode {
        status,
        scheduled_at: state.scheduled_at.map(instant),
        started_at: state.started_at.map(instant),
        finished_at: state.completed_at.map(instant),
        next_attempt_at: state.next_attempt_at.map(instant),
        attempts: state
            .attempts
            .into_iter()
            .map(|attempt| ExecutionAttempt {
                attempt_number: attempt.attempt_number,
                recorded_at: instant(attempt.started_at),
                finished_at: attempt.completed_at.map(instant),
                output: attempt.output.map(output),
                error: attempt.error.as_ref().map(failure),
                output_bytes: attempt.output_bytes,
            })
            .collect(),
        output: recorded_output.map(action_output).transpose()?,
        error: state.error_message.as_ref().map(failure),
    })
}

fn output(value: ExecutionOutput) -> ExecutionNodeOutput {
    match value {
        ExecutionOutput::Inline { value } => ExecutionNodeOutput::Inline { value },
        ExecutionOutput::BlobRef { size, mime, .. } => ExecutionNodeOutput::External {
            size: Some(size),
            mime: Some(mime),
        },
    }
}

fn action_output(
    value: nebula_action::ActionOutput<serde_json::Value>,
) -> Result<ExecutionNodeOutput, InspectionError> {
    use nebula_action::ActionOutput as O;
    Ok(match value {
        O::Value(value) => ExecutionNodeOutput::Inline { value },
        O::Reference(reference) => ExecutionNodeOutput::External {
            size: reference.size,
            mime: reference.content_type,
        },
        O::Binary(binary) => ExecutionNodeOutput::Binary {
            size: binary.effective_size(),
            mime: binary.content_type,
        },
        O::Collection(items) => ExecutionNodeOutput::Collection {
            items: items
                .into_iter()
                .map(action_output)
                .collect::<Result<_, _>>()?,
        },
        O::Deferred(_) => ExecutionNodeOutput::Deferred,
        O::Empty => ExecutionNodeOutput::Empty,
        _ => return Err(InspectionError::OutputKind),
    })
}

fn failure(error: &ErrorEnvelope) -> ExecutionFailure {
    ExecutionFailure {
        code: error.code().as_str().to_owned(),
        category: error.category().to_string(),
        retryable: error.is_retryable(),
        message: error.redacted_message().map(str::to_owned),
        source_codes: error
            .source_codes()
            .iter()
            .map(|code| code.as_str().to_owned())
            .collect(),
    }
}

fn instant(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Micros, true)
}
