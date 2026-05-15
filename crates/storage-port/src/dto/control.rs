//! Control-queue message DTO (spec-16 §12.2).
use crate::Scope;
use serde::{Deserialize, Serialize};

/// Control commands delivered through the durable outbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlCommand {
    /// First-time dispatch of a newly-created execution.
    Start,
    /// Cooperative cancel (graceful shutdown of running work).
    Cancel,
    /// Forced termination (escalation after grace period).
    Terminate,
    /// Resume a suspended execution.
    Resume,
    /// Restart an execution from the beginning.
    Restart,
}

impl ControlCommand {
    /// Stable text form stored in the backend `command` column.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Cancel => "Cancel",
            Self::Terminate => "Terminate",
            Self::Resume => "Resume",
            Self::Restart => "Restart",
        }
    }
}

/// One queued control message.
///
/// `id` is a typed 16-byte ULID (raw bytes — **not** the UTF-8 of the ULID
/// string; the legacy string-encoding hack is removed). `execution_id` is the
/// opaque string form of the target execution. `scope` makes every enqueue
/// tenant-scoped so a low-privilege tenant cannot enqueue a Cancel/Terminate
/// for another tenant's execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlMsg {
    /// 16-byte ULID primary key (raw bytes).
    pub id: [u8; 16],
    /// Target execution id (opaque string form).
    pub execution_id: String,
    /// Command to deliver.
    pub command: ControlCommand,
    /// Tenant scope this message belongs to.
    pub scope: Scope,
    /// Optional W3C `traceparent` captured at enqueue time.
    pub w3c_traceparent: Option<String>,
    /// Times this row was reclaimed back to `Pending` after a crashed runner.
    pub reclaim_count: u32,
}
