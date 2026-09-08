//! Checked projections of execution-owned per-node replay evidence.

use super::*;
use nebula_execution::NodeCheckpoint;
use std::io::{self, Write};

/// Absolute serialized checkpoint ceiling, even when no execution output
/// budget was requested. Persisted checkpoints cross a trust boundary during
/// recovery, so the decoder must never inherit an unbounded default.
pub(super) const MAX_DURABLE_CHECKPOINT_BYTES: u64 = 16 * 1024 * 1024;

struct CheckpointSizeCounter {
    bytes: u64,
    limit: u64,
}

impl CheckpointSizeCounter {
    const fn new(limit: u64) -> Self {
        Self { bytes: 0, limit }
    }

    fn count(&mut self, value: &serde_json::Value) -> Result<(), EngineError> {
        serde_json::to_writer(self, value).map_err(|_| EngineError::CheckpointPayloadLimit)
    }
}

impl Write for CheckpointSizeCounter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let buffer_bytes = u64::try_from(buffer.len())
            .map_err(|_| io::Error::from(io::ErrorKind::FileTooLarge))?;
        let next = self
            .bytes
            .checked_add(buffer_bytes)
            .filter(|bytes| *bytes <= self.limit)
            .ok_or_else(|| io::Error::from(io::ErrorKind::FileTooLarge))?;
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Reject oversized persisted checkpoint evidence before decoding the full
/// execution state. This avoids allocating a second unbounded representation
/// for a corrupt or hostile durable row.
pub(super) fn validate_encoded_checkpoint_size(
    encoded_state: &serde_json::Value,
) -> Result<(), EngineError> {
    let state = encoded_state
        .as_object()
        .ok_or(EngineError::InvalidRecordedExecution)?;
    let mut counter = CheckpointSizeCounter::new(MAX_DURABLE_CHECKPOINT_BYTES);

    if let Some(checkpoint) = state.get("checkpoint").filter(|value| !value.is_null()) {
        counter.count(checkpoint)?;
    }

    let node_states = state
        .get("node_states")
        .and_then(serde_json::Value::as_object)
        .ok_or(EngineError::InvalidRecordedExecution)?;
    for node_state in node_states.values() {
        let node_state = node_state
            .as_object()
            .ok_or(EngineError::InvalidRecordedExecution)?;
        let attempts = node_state
            .get("attempts")
            .and_then(serde_json::Value::as_array)
            .ok_or(EngineError::InvalidRecordedExecution)?;
        for attempt in attempts {
            let attempt = attempt
                .as_object()
                .ok_or(EngineError::InvalidRecordedExecution)?;
            if let Some(output) = attempt.get("output").filter(|value| !value.is_null()) {
                counter.count(output)?;
            }
        }
        if let Some(output) = node_state
            .get("current_output")
            .filter(|value| !value.is_null())
        {
            counter.count(output)?;
        }
    }
    Ok(())
}

pub(super) enum CheckpointRouting {
    Action(Box<ActionResult<serde_json::Value>>),
    Main,
    Error,
    Bypass,
    None,
}

pub(super) fn action_checkpoint(
    result: &ActionResult<serde_json::Value>,
) -> Result<NodeCheckpoint, EngineError> {
    Ok(NodeCheckpoint::ActionResult {
        format_version: 1,
        value: serde_json::to_value(result).map_err(|_| EngineError::InvalidRecordedCheckpoint)?,
    })
}

pub(super) fn checkpoint_routing(
    checkpoint: &NodeCheckpoint,
) -> Result<CheckpointRouting, EngineError> {
    Ok(match checkpoint {
        NodeCheckpoint::ActionResult {
            format_version: 1,
            value,
        } => {
            let bytes =
                serde_json::to_vec(value).map_err(|_| EngineError::InvalidRecordedCheckpoint)?;
            CheckpointRouting::Action(Box::new(
                serde_json::from_slice(&bytes)
                    .map_err(|_| EngineError::InvalidRecordedCheckpoint)?,
            ))
        },
        NodeCheckpoint::ActionResult { .. } => return Err(EngineError::InvalidRecordedCheckpoint),
        NodeCheckpoint::TimerCompleted { .. } | NodeCheckpoint::Recovered {} => {
            CheckpointRouting::Main
        },
        NodeCheckpoint::Failed { .. } => CheckpointRouting::Error,
        NodeCheckpoint::Bypassed {} => CheckpointRouting::Bypass,
        NodeCheckpoint::Skipped {} => CheckpointRouting::None,
        _ => return Err(EngineError::InvalidRecordedCheckpoint),
    })
}

pub(super) fn checkpoint_output(
    checkpoint: &NodeCheckpoint,
) -> Result<Option<serde_json::Value>, EngineError> {
    Ok(match checkpoint_routing(checkpoint)? {
        CheckpointRouting::Action(result) => extract_primary_output(&result),
        CheckpointRouting::Main => match checkpoint {
            NodeCheckpoint::TimerCompleted { partial_output } => partial_output.clone(),
            NodeCheckpoint::Recovered {} => Some(serde_json::Value::Null),
            _ => return Err(EngineError::InvalidRecordedCheckpoint),
        },
        CheckpointRouting::Error => match checkpoint {
            NodeCheckpoint::Failed { error_port_output } => error_port_output.clone(),
            _ => return Err(EngineError::InvalidRecordedCheckpoint),
        },
        CheckpointRouting::None | CheckpointRouting::Bypass => None,
    })
}

pub(super) fn checkpoint_bytes(
    checkpoint: &nebula_execution::ExecutionCheckpoint,
) -> Result<u64, EngineError> {
    serde_json::to_vec(checkpoint)
        .map(|bytes| bytes.len() as u64)
        .map_err(|_| EngineError::InvalidRecordedCheckpoint)
}

pub(super) fn validate_checkpoint_size(state: &ExecutionState) -> Result<(), EngineError> {
    let limit = state
        .budget
        .as_ref()
        .and_then(|budget| budget.max_output_bytes)
        .map_or(MAX_DURABLE_CHECKPOINT_BYTES, |configured| {
            configured.min(MAX_DURABLE_CHECKPOINT_BYTES)
        });
    let mut bytes = match &state.checkpoint {
        Some(checkpoint) if !checkpoint.nodes().is_empty() => checkpoint_bytes(checkpoint)?,
        _ => 0,
    };
    for node in state.node_states.values() {
        for output in node
            .attempts
            .iter()
            .filter_map(|attempt| attempt.output.as_ref())
        {
            let encoded =
                serde_json::to_vec(output).map_err(|_| EngineError::InvalidRecordedCheckpoint)?;
            bytes = bytes
                .checked_add(encoded.len() as u64)
                .ok_or(EngineError::CheckpointPayloadLimit)?;
        }
        if let Some(output) = &node.current_output {
            let encoded =
                serde_json::to_vec(output).map_err(|_| EngineError::InvalidRecordedCheckpoint)?;
            bytes = bytes
                .checked_add(encoded.len() as u64)
                .ok_or(EngineError::CheckpointPayloadLimit)?;
        }
    }
    if bytes > limit {
        return Err(EngineError::CheckpointPayloadLimit);
    }
    Ok(())
}

pub(super) fn failure_checkpoint(
    outcome: FailureOutcome,
    outputs: &DashMap<NodeKey, serde_json::Value>,
    node: &NodeKey,
) -> NodeCheckpoint {
    match outcome {
        FailureOutcome::Recover => NodeCheckpoint::Recovered {},
        FailureOutcome::Fail => NodeCheckpoint::Failed {
            error_port_output: outputs.get(node).map(|output| output.value().clone()),
        },
    }
}

pub(super) fn validated_checkpoint_outputs(
    state: &ExecutionState,
    node_keys: &HashSet<NodeKey>,
    disabled_nodes: &HashSet<NodeKey>,
) -> Result<Vec<(NodeKey, serde_json::Value)>, EngineError> {
    let Some(checkpoint) = &state.checkpoint else {
        return if state.status == ExecutionStatus::Created && state.node_states.is_empty() {
            Ok(Vec::new())
        } else {
            Err(EngineError::InvalidRecordedCheckpoint)
        };
    };
    if checkpoint.format_version() != 1
        || checkpoint
            .nodes()
            .keys()
            .any(|node| !node_keys.contains(node))
    {
        return Err(EngineError::InvalidRecordedCheckpoint);
    }
    validate_checkpoint_size(state)?;
    for (node, node_state) in &state.node_states {
        if matches!(
            node_state.state,
            NodeState::Completed | NodeState::Waiting | NodeState::Failed
        ) && !checkpoint.nodes().contains_key(node)
        {
            return Err(EngineError::InvalidRecordedCheckpoint);
        }
    }
    let mut outputs = Vec::new();
    for (node, evidence) in checkpoint.nodes() {
        let node_state = state
            .node_states
            .get(node)
            .ok_or(EngineError::InvalidRecordedCheckpoint)?;
        let routing = checkpoint_routing(evidence)?;
        if let NodeCheckpoint::Failed {
            error_port_output: Some(output),
        } = evidence
        {
            let payload: nebula_workflow::ErrorPortPayload = serde_json::from_value(output.clone())
                .map_err(|_| EngineError::InvalidRecordedCheckpoint)?;
            if payload.node_id != node.as_str() {
                return Err(EngineError::InvalidRecordedCheckpoint);
            }
        }
        let compatible = match &routing {
            CheckpointRouting::Action(result)
                if matches!(result.as_ref(), ActionResult::Wait { .. }) =>
            {
                !node_state.state.is_terminal()
            },
            CheckpointRouting::Action(_) | CheckpointRouting::Main => {
                node_state.state == NodeState::Completed
            },
            CheckpointRouting::Error => {
                !matches!(node_state.state, NodeState::Completed | NodeState::Skipped)
            },
            CheckpointRouting::None => {
                matches!(node_state.state, NodeState::Skipped | NodeState::Cancelled)
            },
            CheckpointRouting::Bypass => {
                node_state.state == NodeState::Skipped && disabled_nodes.contains(node)
            },
        };
        if !compatible {
            return Err(EngineError::InvalidRecordedCheckpoint);
        }
        let output = checkpoint_output(evidence)?;
        if let Some(current) = &node_state.current_output
            && current.data.as_inline() != output.as_ref()
        {
            return Err(EngineError::InvalidRecordedCheckpoint);
        }
        if matches!(routing, CheckpointRouting::Action(ref result) if !matches!(result.as_ref(), ActionResult::Wait { .. }))
            && let Some(attempt_output) = node_state
                .latest_attempt()
                .and_then(|attempt| attempt.output.as_ref())
            && attempt_output.as_inline()
                != Some(output.as_ref().unwrap_or(&serde_json::Value::Null))
        {
            return Err(EngineError::InvalidRecordedCheckpoint);
        }
        if let Some(output) = output {
            outputs.push((node.clone(), output));
        }
    }
    Ok(outputs)
}
