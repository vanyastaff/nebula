//! Rotation State Machine
//!
//! Tracks the state of a credential rotation operation.

use serde::{Deserialize, Serialize};

use super::error::{RotationError, RotationResult};

/// State of a rotation operation
///
/// # State Transitions
///
/// ```text
/// Pending → Creating → Validating → Committing → Committed
///     ↓         ↓           ↓            ↓
///     → RolledBack ← ← ← ← ← (failure at any stage)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationState {
    /// Queued for rotation but not yet started
    Pending,

    /// Generating new credential
    Creating,

    /// Testing new credential (validation in progress)
    Validating,

    /// Storing new credential and updating metadata
    Committing,

    /// Rotation complete, grace period active
    Committed,

    /// Validation failed or error occurred, restored old credential
    RolledBack,
}

impl RotationState {
    /// Check if transition to the target state is valid
    #[must_use]
    pub fn can_transition_to(&self, target: RotationState) -> bool {
        use RotationState::*;

        match (self, target) {
            // Forward progress
            (Pending, Creating) => true,
            (Creating, Validating) => true,
            (Validating, Committing) => true,
            (Committing, Committed) => true,

            // Rollback from any active state
            (Pending, RolledBack) => true,
            (Creating, RolledBack) => true,
            (Validating, RolledBack) => true,
            (Committing, RolledBack) => true,

            // Terminal states cannot transition
            (Committed, _) => false,
            (RolledBack, _) => false,

            // All other transitions are invalid
            _ => false,
        }
    }

    /// Validate and perform state transition
    pub fn transition_to(&self, target: RotationState) -> RotationResult<RotationState> {
        if self.can_transition_to(target) {
            Ok(target)
        } else {
            Err(RotationError::InvalidStateTransition {
                from: format!("{:?}", self),
                to: format!("{:?}", target),
            })
        }
    }

    /// Check if state is terminal (no more transitions possible)
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, RotationState::Committed | RotationState::RolledBack)
    }

    /// Check if rotation is in progress
    #[must_use]
    pub fn is_in_progress(&self) -> bool {
        matches!(
            self,
            RotationState::Pending
                | RotationState::Creating
                | RotationState::Validating
                | RotationState::Committing
        )
    }

    /// Check if rotation completed successfully
    pub fn is_committed(&self) -> bool {
        matches!(self, RotationState::Committed)
    }

    /// Check if rotation was rolled back
    pub fn is_rolled_back(&self) -> bool {
        matches!(self, RotationState::RolledBack)
    }
}

impl std::fmt::Display for RotationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RotationState::Pending => write!(f, "pending"),
            RotationState::Creating => write!(f, "creating"),
            RotationState::Validating => write!(f, "validating"),
            RotationState::Committing => write!(f, "committing"),
            RotationState::Committed => write!(f, "committed"),
            RotationState::RolledBack => write!(f, "rolled_back"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_forward_transitions() {
        assert!(RotationState::Pending.can_transition_to(RotationState::Creating));
        assert!(RotationState::Creating.can_transition_to(RotationState::Validating));
        assert!(RotationState::Validating.can_transition_to(RotationState::Committing));
        assert!(RotationState::Committing.can_transition_to(RotationState::Committed));
    }

    #[test]
    fn test_valid_rollback_transitions() {
        assert!(RotationState::Pending.can_transition_to(RotationState::RolledBack));
        assert!(RotationState::Creating.can_transition_to(RotationState::RolledBack));
        assert!(RotationState::Validating.can_transition_to(RotationState::RolledBack));
        assert!(RotationState::Committing.can_transition_to(RotationState::RolledBack));
    }

    #[test]
    fn test_invalid_transitions() {
        // Cannot skip states
        assert!(!RotationState::Pending.can_transition_to(RotationState::Validating));
        assert!(!RotationState::Creating.can_transition_to(RotationState::Committing));

        // Cannot transition from terminal states
        assert!(!RotationState::Committed.can_transition_to(RotationState::Pending));
        assert!(!RotationState::Committed.can_transition_to(RotationState::RolledBack));
        assert!(!RotationState::RolledBack.can_transition_to(RotationState::Pending));
        assert!(!RotationState::RolledBack.can_transition_to(RotationState::Committed));
    }

    #[test]
    fn test_terminal_states() {
        assert!(!RotationState::Pending.is_terminal());
        assert!(!RotationState::Creating.is_terminal());
        assert!(!RotationState::Validating.is_terminal());
        assert!(!RotationState::Committing.is_terminal());
        assert!(RotationState::Committed.is_terminal());
        assert!(RotationState::RolledBack.is_terminal());
    }

    #[test]
    fn test_transition_validation() {
        let state = RotationState::Pending;

        // Valid transition
        let next = state.transition_to(RotationState::Creating);
        assert!(next.is_ok());
        assert_eq!(next.unwrap(), RotationState::Creating);

        // Invalid transition
        let invalid = state.transition_to(RotationState::Committed);
        assert!(invalid.is_err());
    }
}
