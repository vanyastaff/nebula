//! Versioned observations of execution-owner control decisions.
//!
//! These values describe a decision; constructing one grants no persistence
//! authority. Storage derives rejected-actor observations under its aggregate
//! lock. Successful handoff observations commit with the accepted turn.

use nebula_core::{NodeKey, WorkerFlavorRevisionId};
use serde::{Deserialize, Serialize};

use crate::ErrorEnvelope;

/// The only execution-control observation version this build accepts.
pub const EXECUTION_CONTROL_OBSERVATION_VERSION: u8 = 1;

/// Closed operator vocabulary for execution-control decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionControlOutcome {
    /// Runtime ownership was durably accepted, independently of later action success.
    Accepted,
    /// Superseded authority was refused.
    Fenced,
    /// Work remains durable for later delivery.
    Deferred,
    /// Execution-owned admission postponed work.
    Throttled,
    /// Recovery ownership of a retained accepted turn was durably accepted.
    Recovered,
    /// Exact runtime flavor readmission refused a persisted execution.
    FlavorMismatch,
}

impl ExecutionControlOutcome {
    /// Bounded metric/span label; identities never become metric labels.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Fenced => "fenced",
            Self::Deferred => "deferred",
            Self::Throttled => "throttled",
            Self::Recovered => "recovered",
            Self::FlavorMismatch => "flavor-mismatch",
        }
    }
}

/// Retained queue origin of an accepted execution turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionControlQueueKind {
    /// Execution control queue.
    ControlQueue,
    /// Workflow job dispatch queue.
    JobDispatch,
}

/// Durable decision identity, scoped by the containing execution journal row.
/// Queue claim generations and execution lease generations are distinct axes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionControlSource {
    /// A generation-bound execution control delivery.
    ControlQueue {
        /// Persisted queue row identity.
        row_id: [u8; 16],
        /// Delivery claim generation, never an execution lease token.
        queue_claim_generation: u64,
    },
    /// A generation-bound workflow job delivery.
    JobDispatch {
        /// Persisted queue row identity.
        row_id: [u8; 16],
        /// Delivery claim generation, never an execution lease token.
        queue_claim_generation: u64,
    },
    /// The exact accepted-turn marker retained by storage.
    AcceptedTurn {
        /// Queue owning the retained source row.
        source_kind: ExecutionControlQueueKind,
        /// Original accepted delivery, not a new synthesized queue row.
        source_row_id: [u8; 16],
        /// Historical marker generation rechecked by recovery acceptance.
        accepted_execution_lease_generation: u64,
    },
}

/// Framework-authored reasons; no provider text or open string reason enters the journal.
///
/// Decoding goes through [`RecordedExecutionControlReason`]: serde does not
/// apply `deny_unknown_fields` to unit variants of an internally tagged enum,
/// so a unit reason would otherwise accept and silently drop extra fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    deny_unknown_fields,
    from = "RecordedExecutionControlReason"
)]
pub enum ExecutionControlReason {
    /// Queue delivery and execution ownership committed atomically.
    ControlAccepted,
    /// A delivery no longer holds its queue claim generation.
    ClaimSuperseded {
        /// Queue generation supplied by the refused delivery.
        attempted_queue_claim_generation: u64,
        /// Current persisted queue generation observed under the owner lock.
        current_queue_claim_generation: u64,
    },
    /// A delivery no longer holds the execution lease generation.
    LeaseFenced {
        /// Execution generation supplied by the refused actor.
        attempted_execution_lease_generation: u64,
        /// Current execution generation observed under the owner lock.
        current_execution_lease_generation: u64,
    },
    /// The matching generation no longer has a live execution lease.
    LeaseExpired {
        /// Execution generation presented by the refused actor.
        attempted_execution_lease_generation: u64,
        /// Persisted generation whose deadline has passed.
        current_execution_lease_generation: u64,
    },
    /// The execution row has no held lease, even if its historical generation matches.
    LeaseAbsent {
        /// Execution generation presented by the refused actor.
        attempted_execution_lease_generation: u64,
        /// Historical generation retained after release.
        current_execution_lease_generation: u64,
    },
    /// The live owner command requires a fresh execution checkpoint version.
    ExecutionVersionConflict {
        /// Execution version supplied by the validated command owner.
        expected_version: u64,
        /// Current execution version read under the aggregate lock.
        actual_version: u64,
    },
    /// Execution-owned admission reached its configured bound before provider dispatch.
    AdmissionThrottled,
    /// Recovery acceptance replaced the retained accepted-turn generation.
    AcceptedTurnRecovered,
    /// An already persisted execution requires an exact different runtime flavor.
    ExactFlavorMismatch {
        /// Flavor retained by the execution aggregate.
        expected: WorkerFlavorRevisionId,
        /// Flavor presented by the deciding runtime.
        actual: WorkerFlavorRevisionId,
    },
}

/// Durable decoding mirror of [`ExecutionControlReason`]. Every variant is a
/// struct variant, so unknown fields are refused for field-less reasons too.
#[derive(Deserialize)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
enum RecordedExecutionControlReason {
    ControlAccepted {},
    ClaimSuperseded {
        attempted_queue_claim_generation: u64,
        current_queue_claim_generation: u64,
    },
    LeaseFenced {
        attempted_execution_lease_generation: u64,
        current_execution_lease_generation: u64,
    },
    LeaseExpired {
        attempted_execution_lease_generation: u64,
        current_execution_lease_generation: u64,
    },
    LeaseAbsent {
        attempted_execution_lease_generation: u64,
        current_execution_lease_generation: u64,
    },
    ExecutionVersionConflict {
        expected_version: u64,
        actual_version: u64,
    },
    AdmissionThrottled {},
    AcceptedTurnRecovered {},
    ExactFlavorMismatch {
        expected: WorkerFlavorRevisionId,
        actual: WorkerFlavorRevisionId,
    },
}

impl From<RecordedExecutionControlReason> for ExecutionControlReason {
    fn from(recorded: RecordedExecutionControlReason) -> Self {
        use RecordedExecutionControlReason as Recorded;
        match recorded {
            Recorded::ControlAccepted {} => Self::ControlAccepted,
            Recorded::ClaimSuperseded {
                attempted_queue_claim_generation,
                current_queue_claim_generation,
            } => Self::ClaimSuperseded {
                attempted_queue_claim_generation,
                current_queue_claim_generation,
            },
            Recorded::LeaseFenced {
                attempted_execution_lease_generation,
                current_execution_lease_generation,
            } => Self::LeaseFenced {
                attempted_execution_lease_generation,
                current_execution_lease_generation,
            },
            Recorded::LeaseExpired {
                attempted_execution_lease_generation,
                current_execution_lease_generation,
            } => Self::LeaseExpired {
                attempted_execution_lease_generation,
                current_execution_lease_generation,
            },
            Recorded::LeaseAbsent {
                attempted_execution_lease_generation,
                current_execution_lease_generation,
            } => Self::LeaseAbsent {
                attempted_execution_lease_generation,
                current_execution_lease_generation,
            },
            Recorded::ExecutionVersionConflict {
                expected_version,
                actual_version,
            } => Self::ExecutionVersionConflict {
                expected_version,
                actual_version,
            },
            Recorded::AdmissionThrottled {} => Self::AdmissionThrottled,
            Recorded::AcceptedTurnRecovered {} => Self::AcceptedTurnRecovered,
            Recorded::ExactFlavorMismatch { expected, actual } => {
                Self::ExactFlavorMismatch { expected, actual }
            },
        }
    }
}

impl ExecutionControlReason {
    /// Bounded framework reason code for operator projections and spans.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ControlAccepted => "control_accepted",
            Self::ClaimSuperseded { .. } => "claim_superseded",
            Self::LeaseFenced { .. } => "lease_fenced",
            Self::LeaseExpired { .. } => "lease_expired",
            Self::LeaseAbsent { .. } => "lease_absent",
            Self::ExecutionVersionConflict { .. } => "execution_version_conflict",
            Self::AdmissionThrottled => "admission_throttled",
            Self::AcceptedTurnRecovered => "accepted_turn_recovered",
            Self::ExactFlavorMismatch { .. } => "exact_flavor_mismatch",
        }
    }

    /// Outcome implied by this reason; the persisted record cannot disagree.
    #[must_use]
    pub const fn outcome(&self) -> ExecutionControlOutcome {
        match self {
            Self::ControlAccepted => ExecutionControlOutcome::Accepted,
            Self::ClaimSuperseded { .. }
            | Self::LeaseFenced { .. }
            | Self::LeaseExpired { .. }
            | Self::LeaseAbsent { .. } => ExecutionControlOutcome::Fenced,
            Self::ExecutionVersionConflict { .. } => ExecutionControlOutcome::Deferred,
            Self::AdmissionThrottled => ExecutionControlOutcome::Throttled,
            Self::AcceptedTurnRecovered => ExecutionControlOutcome::Recovered,
            Self::ExactFlavorMismatch { .. } => ExecutionControlOutcome::FlavorMismatch,
        }
    }
}

/// Optional node-attempt attribution kept together rather than partly populated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionControlAttempt {
    /// Node receiving the execution-owned decision.
    ///
    /// Decoded from an owned string: journal payloads reach readers as
    /// `serde_json::Value`, which cannot lend the borrowed `str` that
    /// `NodeKey`'s own `Deserialize` requires.
    #[serde(deserialize_with = "owned_node_key")]
    pub node_key: NodeKey,
    /// Zero-based node attempt.
    pub attempt: u32,
}

fn owned_node_key<'de, D>(deserializer: D) -> Result<NodeKey, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    NodeKey::new(&raw).map_err(serde::de::Error::custom)
}

/// Closed, versioned payload for an execution journal control observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RecordedExecutionControlObservation")]
pub struct ExecutionControlObservationV1 {
    version: u8,
    outcome: ExecutionControlOutcome,
    reason: ExecutionControlReason,
    source: ExecutionControlSource,
    execution_lease_generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    attempt: Option<ExecutionControlAttempt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ErrorEnvelope>,
}

impl ExecutionControlObservationV1 {
    /// Build framework decision data. The owner persistence seam still verifies authority.
    #[must_use]
    pub fn new(
        source: ExecutionControlSource,
        execution_lease_generation: u64,
        reason: ExecutionControlReason,
    ) -> Self {
        Self {
            version: EXECUTION_CONTROL_OBSERVATION_VERSION,
            outcome: reason.outcome(),
            reason,
            source,
            execution_lease_generation,
            attempt: None,
            error: None,
        }
    }

    /// Attribute a decision to one exact node attempt.
    #[must_use]
    pub fn with_attempt(mut self, node_key: NodeKey, attempt: u32) -> Self {
        self.attempt = Some(ExecutionControlAttempt { node_key, attempt });
        self
    }

    /// Attach existing typed failure context under the owning runtime error policy.
    #[must_use]
    pub fn with_error(mut self, error: ErrorEnvelope) -> Self {
        self.error = Some(error);
        self
    }

    /// Persisted observation protocol version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }
    /// Persisted control outcome.
    #[must_use]
    pub const fn outcome(&self) -> ExecutionControlOutcome {
        self.outcome
    }
    /// Closed framework reason.
    #[must_use]
    pub const fn reason(&self) -> &ExecutionControlReason {
        &self.reason
    }
    /// Retained decision identity.
    #[must_use]
    pub const fn source(&self) -> &ExecutionControlSource {
        &self.source
    }
    /// Execution lease generation, distinct from any queue claim generation.
    #[must_use]
    pub const fn execution_lease_generation(&self) -> u64 {
        self.execution_lease_generation
    }
    /// Optional node-attempt attribution.
    #[must_use]
    pub const fn attempt(&self) -> Option<&ExecutionControlAttempt> {
        self.attempt.as_ref()
    }
    /// Optional typed error context.
    #[must_use]
    pub const fn error(&self) -> Option<&ErrorEnvelope> {
        self.error.as_ref()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedExecutionControlObservation {
    version: u8,
    outcome: ExecutionControlOutcome,
    reason: ExecutionControlReason,
    source: ExecutionControlSource,
    execution_lease_generation: u64,
    #[serde(default)]
    attempt: Option<ExecutionControlAttempt>,
    #[serde(default)]
    error: Option<ErrorEnvelope>,
}

impl TryFrom<RecordedExecutionControlObservation> for ExecutionControlObservationV1 {
    type Error = &'static str;

    fn try_from(record: RecordedExecutionControlObservation) -> Result<Self, Self::Error> {
        if record.version != EXECUTION_CONTROL_OBSERVATION_VERSION {
            return Err("unsupported execution-control observation version");
        }
        if record.outcome != record.reason.outcome() {
            return Err("execution-control outcome does not match its reason");
        }
        Ok(Self {
            version: record.version,
            outcome: record.outcome,
            reason: record.reason,
            source: record.source,
            execution_lease_generation: record.execution_lease_generation,
            attempt: record.attempt,
            error: record.error,
        })
    }
}
