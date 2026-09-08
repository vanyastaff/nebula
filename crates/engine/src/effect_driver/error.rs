//! Payload-free operation-driver failures.

use nebula_action::effect::{EffectFailureCode, EffectPreparationError};
use nebula_core::OperationId;
use nebula_storage_port::dto::OperationLedgerError;

/// A durable remote effect could not produce an acknowledged, usable result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EffectExecutionError {
    /// The owning execution cancelled this turn.
    #[error("remote effect turn was cancelled")]
    Cancelled,
    /// No exact declared capability or owning ledger is available.
    #[error("remote effect lacks its exact runtime authority")]
    MissingAuthority,
    /// The admitted identity or adapter contract is inconsistent.
    #[error("remote effect contract is invalid")]
    InvalidContract,
    /// Request preparation failed before provider invocation.
    #[error("remote effect preparation failed: {0}")]
    Preparation(#[from] EffectPreparationError),
    /// The database operation failed; its typed acknowledgement semantics are retained.
    #[error("remote effect ledger failed: {0}")]
    Ledger(#[from] OperationLedgerError),
    /// Exhausted recovery leaves an honest, durable unknown provider outcome.
    #[error("remote effect outcome is unknown for operation {operation_id}")]
    OutcomeUnknown {
        /// Original operation, retained across recovery.
        operation_id: OperationId,
    },
    /// The effect was applied, but a usable output cannot be recovered.
    #[error("remote effect output is unavailable for operation {operation_id}")]
    OutputUnavailable {
        /// Known applied operation; it must never be repeated for a missing output.
        operation_id: OperationId,
    },
    /// The provider definitively rejected the original operation.
    #[error("remote effect was rejected for operation {operation_id}: {code:?}")]
    Rejected {
        /// Original operation identity.
        operation_id: OperationId,
        /// Bounded provider outcome classification.
        code: EffectFailureCode,
    },
    /// Persisted evidence cannot be interpreted without guessing.
    #[error("remote effect evidence is invalid")]
    InvalidEvidence,
}

impl EffectExecutionError {
    /// Stable bounded diagnosis for runtime logs and transport projections.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Cancelled => "ENGINE:EFFECT_CANCELLED",
            Self::MissingAuthority => "ENGINE:EFFECT_MISSING_AUTHORITY",
            Self::InvalidContract => "ENGINE:EFFECT_INVALID_CONTRACT",
            Self::Preparation(_) => "ENGINE:EFFECT_PREPARATION",
            Self::Ledger(_) => "ENGINE:EFFECT_LEDGER",
            Self::OutcomeUnknown { .. } => "ENGINE:EFFECT_OUTCOME_UNKNOWN",
            Self::OutputUnavailable { .. } => "ENGINE:EFFECT_OUTPUT_UNAVAILABLE",
            Self::Rejected { .. } => "ENGINE:EFFECT_REJECTED",
            Self::InvalidEvidence => "ENGINE:EFFECT_INVALID_EVIDENCE",
        }
    }
    /// Whether the turn must relinquish its lease without finalizing the node.
    ///
    /// Recovery may read the ledger again; this is not provider retry authority.
    #[must_use]
    pub const fn is_deferred(self) -> bool {
        matches!(
            self,
            Self::Preparation(EffectPreparationError::Unavailable)
                | Self::Ledger(
                    OperationLedgerError::Unavailable
                        | OperationLedgerError::AcknowledgementUnknown
                        | OperationLedgerError::ExecutionLeaseRejected
                        | OperationLedgerError::ProtocolConflict
                )
        )
    }
}
