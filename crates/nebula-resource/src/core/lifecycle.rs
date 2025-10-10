//! Resource lifecycle management

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Represents the current state of a resource in its lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default)]
pub enum LifecycleState {
    /// Resource has been created but not yet initialized
    #[default]
    Created,
    /// Resource is currently being initialized
    Initializing,
    /// Resource is ready and available for use
    Ready,
    /// Resource is currently being used
    InUse,
    /// Resource is available but not currently in use
    Idle,
    /// Resource is under maintenance (temporarily unavailable)
    Maintenance,
    /// Resource is being drained (no new acquisitions allowed)
    Draining,
    /// Resource is being cleaned up
    Cleanup,
    /// Resource has been fully terminated
    Terminated,
    /// Resource is in a failed state
    Failed,
}

impl LifecycleState {
    /// Check if the resource is available for use
    #[must_use] 
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Ready | Self::Idle)
    }

    /// Check if the resource is in a terminal state
    #[must_use] 
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Terminated | Self::Failed)
    }

    /// Check if the resource is in a transitional state
    #[must_use] 
    pub fn is_transitional(&self) -> bool {
        matches!(self, Self::Initializing | Self::Draining | Self::Cleanup)
    }

    /// Check if the resource can be acquired
    #[must_use] 
    pub fn can_acquire(&self) -> bool {
        matches!(self, Self::Ready | Self::Idle)
    }

    /// Check if the resource can transition to the target state
    #[must_use] 
    pub fn can_transition_to(&self, target: LifecycleState) -> bool {
        use LifecycleState::{Created, Initializing, Failed, Terminated, Ready, InUse, Idle, Maintenance, Draining, Cleanup};

        match (self, target) {
            // From Created
            (Created, Initializing) => true,
            (Created, Failed) => true,
            (Created, Terminated) => true,

            // From Initializing
            (Initializing, Ready) => true,
            (Initializing, Failed) => true,

            // From Ready
            (Ready, InUse) => true,
            (Ready, Idle) => true,
            (Ready, Maintenance) => true,
            (Ready, Draining) => true,
            (Ready, Failed) => true,

            // From InUse
            (InUse, Ready) => true,
            (InUse, Idle) => true,
            (InUse, Failed) => true,

            // From Idle
            (Idle, InUse) => true,
            (Idle, Ready) => true,
            (Idle, Maintenance) => true,
            (Idle, Draining) => true,
            (Idle, Cleanup) => true,
            (Idle, Failed) => true,

            // From Maintenance
            (Maintenance, Ready) => true,
            (Maintenance, Failed) => true,
            (Maintenance, Cleanup) => true,

            // From Draining
            (Draining, Cleanup) => true,
            (Draining, Failed) => true,

            // From Cleanup
            (Cleanup, Terminated) => true,
            (Cleanup, Failed) => true,

            // From Failed
            (Failed, Cleanup) => true,
            (Failed, Terminated) => true,

            // No transitions from Terminated
            (Terminated, _) => false,

            // Self-transitions are always allowed
            (state, target) if *state == target => true,

            // All other transitions are invalid
            _ => false,
        }
    }

    /// Get the next logical state(s) for this lifecycle state
    #[must_use] 
    pub fn next_states(&self) -> &'static [LifecycleState] {
        use LifecycleState::{Created, Initializing, Failed, Terminated, Ready, InUse, Idle, Maintenance, Draining, Cleanup};

        match self {
            Created => &[Initializing, Failed, Terminated],
            Initializing => &[Ready, Failed],
            Ready => &[InUse, Idle, Maintenance, Draining, Failed],
            InUse => &[Ready, Idle, Failed],
            Idle => &[InUse, Ready, Maintenance, Draining, Cleanup, Failed],
            Maintenance => &[Ready, Failed, Cleanup],
            Draining => &[Cleanup, Failed],
            Cleanup => &[Terminated, Failed],
            Failed => &[Cleanup, Terminated],
            Terminated => &[],
        }
    }

    /// Get a human-readable description of the state
    #[must_use] 
    pub fn description(&self) -> &'static str {
        match self {
            Self::Created => "Resource has been created but not initialized",
            Self::Initializing => "Resource is being initialized",
            Self::Ready => "Resource is ready and available for use",
            Self::InUse => "Resource is currently being used",
            Self::Idle => "Resource is available but not in use",
            Self::Maintenance => "Resource is under maintenance",
            Self::Draining => "Resource is being drained (no new acquisitions)",
            Self::Cleanup => "Resource is being cleaned up",
            Self::Terminated => "Resource has been fully terminated",
            Self::Failed => "Resource is in a failed state",
        }
    }
}

impl fmt::Display for LifecycleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Created => "Created",
            Self::Initializing => "Initializing",
            Self::Ready => "Ready",
            Self::InUse => "InUse",
            Self::Idle => "Idle",
            Self::Maintenance => "Maintenance",
            Self::Draining => "Draining",
            Self::Cleanup => "Cleanup",
            Self::Terminated => "Terminated",
            Self::Failed => "Failed",
        };
        write!(f, "{name}")
    }
}


/// Lifecycle event that can be observed
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LifecycleEvent {
    /// The resource identifier
    pub resource_id: String,
    /// The previous state
    pub from_state: LifecycleState,
    /// The new state
    pub to_state: LifecycleState,
    /// Timestamp of the transition
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Optional metadata about the transition
    pub metadata: Option<serde_json::Value>,
}

impl LifecycleEvent {
    /// Create a new lifecycle event
    #[must_use] 
    pub fn new(resource_id: String, from_state: LifecycleState, to_state: LifecycleState) -> Self {
        Self {
            resource_id,
            from_state,
            to_state,
            timestamp: chrono::Utc::now(),
            metadata: None,
        }
    }

    /// Create a new lifecycle event with metadata
    #[must_use] 
    pub fn with_metadata(
        resource_id: String,
        from_state: LifecycleState,
        to_state: LifecycleState,
        metadata: serde_json::Value,
    ) -> Self {
        Self {
            resource_id,
            from_state,
            to_state,
            timestamp: chrono::Utc::now(),
            metadata: Some(metadata),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lifecycle_state_availability() {
        assert!(LifecycleState::Ready.is_available());
        assert!(LifecycleState::Idle.is_available());
        assert!(!LifecycleState::Created.is_available());
        assert!(!LifecycleState::Failed.is_available());
    }

    #[test]
    fn test_lifecycle_state_terminal() {
        assert!(LifecycleState::Terminated.is_terminal());
        assert!(LifecycleState::Failed.is_terminal());
        assert!(!LifecycleState::Ready.is_terminal());
    }

    #[test]
    fn test_lifecycle_state_transitions() {
        assert!(LifecycleState::Created.can_transition_to(LifecycleState::Initializing));
        assert!(LifecycleState::Initializing.can_transition_to(LifecycleState::Ready));
        assert!(LifecycleState::Ready.can_transition_to(LifecycleState::InUse));

        // Invalid transitions
        assert!(!LifecycleState::Created.can_transition_to(LifecycleState::InUse));
        assert!(!LifecycleState::Terminated.can_transition_to(LifecycleState::Ready));
    }

    #[test]
    fn test_lifecycle_state_can_acquire() {
        assert!(LifecycleState::Ready.can_acquire());
        assert!(LifecycleState::Idle.can_acquire());
        assert!(!LifecycleState::InUse.can_acquire());
        assert!(!LifecycleState::Failed.can_acquire());
    }
}
