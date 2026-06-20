//! Control-queue message DTO (spec-16 §12.2).
use crate::Scope;
use serde::{Deserialize, Serialize};

/// Which parked signal wait a [`ControlCommand::Resume`] targets.
///
/// A storage-port-level mirror of the engine's persisted resume-identity
/// (`nebula_execution::state::WaitSignal`). It lives here — below the execution
/// crate — because the control-queue DTO cannot depend on `nebula-execution`;
/// the engine matches a `ResumeTarget` against a parked node's `WaitSignal` by
/// kind + identity. Carrying the kind (not a bare string) is the structural
/// safety control: a webhook Resume can never match an approval gate.
///
/// `ControlMsg.resume_target == None` means an untargeted Resume — it arms
/// every signal-driven wait of the execution (the W-S2b behavior), preserved
/// for back-compat and for callers that do not yet target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[non_exhaustive]
pub enum ResumeTarget {
    /// Target a webhook wait by its author-declared callback label.
    Webhook {
        /// Must equal the parked `WaitSignal::Webhook::callback_id`.
        callback_id: String,
    },
    /// Target an approval gate by approver identity.
    Approval {
        /// Must equal the parked `WaitSignal::Approval::approver`.
        approver: String,
    },
    /// Target an execution-completion wait by the awaited execution id.
    Execution {
        /// Must equal the parked `WaitSignal::Execution::execution_id`
        /// (opaque string form).
        execution_id: String,
    },
}

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
    /// Which parked signal wait a [`ControlCommand::Resume`] targets (ADR-0099
    /// W-S3a). `None` is an untargeted Resume (arms every signal wait — the
    /// W-S2b behavior); `Some(target)` arms only the parked node whose
    /// persisted `WaitSignal` matches by kind + identity.
    ///
    /// `#[serde(default)]` so legacy queue payloads that predate the field
    /// deserialize as `None`. W-S3a threads this in-memory from the control
    /// consumer to `dispatch_resume`; the durable backends do not yet persist a
    /// column for it (no producer exists), so the SQL-backed `claim_pending`
    /// paths reconstruct it as `None`. Durable carry is W-S3c (with the real
    /// schema work).
    #[serde(default)]
    pub resume_target: Option<ResumeTarget>,
}
