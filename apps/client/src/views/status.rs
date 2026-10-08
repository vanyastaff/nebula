//! Execution and node statuses in words and colours, the same on every page that shows a run.

use crate::widgets::Tone;
use nebula_api_contract::v1::execution::{ExecutionNodeStatus, ExecutionStatus};

pub(crate) const fn label(status: ExecutionStatus) -> &'static str {
    match status {
        ExecutionStatus::Created => "Queued",
        ExecutionStatus::Running => "Running",
        ExecutionStatus::Paused => "Paused",
        ExecutionStatus::Cancelling => "Cancelling",
        ExecutionStatus::Completed => "Completed",
        ExecutionStatus::Failed => "Failed",
        ExecutionStatus::Cancelled => "Cancelled",
        ExecutionStatus::TimedOut => "Timed out",
    }
}

pub(crate) const fn tone(status: ExecutionStatus) -> Tone {
    match status {
        ExecutionStatus::Completed => Tone::Success,
        ExecutionStatus::Failed | ExecutionStatus::TimedOut => Tone::Danger,
        ExecutionStatus::Running | ExecutionStatus::Created => Tone::Accent,
        ExecutionStatus::Paused | ExecutionStatus::Cancelling => Tone::Warning,
        ExecutionStatus::Cancelled => Tone::Neutral,
    }
}

pub(crate) const fn node_label(status: ExecutionNodeStatus) -> &'static str {
    match status {
        ExecutionNodeStatus::Pending => "Pending",
        ExecutionNodeStatus::Ready => "Ready",
        ExecutionNodeStatus::Running => "Running",
        ExecutionNodeStatus::Completed => "Completed",
        ExecutionNodeStatus::Failed => "Failed",
        ExecutionNodeStatus::Skipped => "Skipped",
        ExecutionNodeStatus::Cancelled => "Cancelled",
        ExecutionNodeStatus::WaitingRetry => "Retry scheduled",
        ExecutionNodeStatus::Waiting => "Waiting",
    }
}

pub(crate) const fn node_tone(status: ExecutionNodeStatus) -> Tone {
    match status {
        ExecutionNodeStatus::Completed => Tone::Success,
        ExecutionNodeStatus::Failed => Tone::Danger,
        ExecutionNodeStatus::Running | ExecutionNodeStatus::Ready => Tone::Accent,
        ExecutionNodeStatus::Waiting | ExecutionNodeStatus::WaitingRetry => Tone::Warning,
        ExecutionNodeStatus::Pending
        | ExecutionNodeStatus::Skipped
        | ExecutionNodeStatus::Cancelled => Tone::Neutral,
    }
}
