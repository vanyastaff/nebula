//! Resource lifecycle state machine.
//!
//! [`Lifecycle`] models the observable state of a single pooled resource instance.
//! The pool and manager transition instances through these states; external code
//! reads the state for observability but should not drive transitions directly.
//!
//! ## State diagram
//!
//! ```text
//! Created ──► Initializing ──► Ready ◄──► Idle
//!    │              │            │  │       │
//!    │           Failed       InUse  │    Maintenance ──► Ready
//!    │                         │    │       │
//!    └──────────────────────►  │   Draining─┘
//!                           Failed    │
//!                              │    Cleanup ──► Terminated
//!                              └──► Cleanup
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};

/// Represents the current state of a resource in its lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Lifecycle {
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

impl Lifecycle {
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
    pub fn can_transition_to(&self, target: Lifecycle) -> bool {
        use Lifecycle::{
            Cleanup, Created, Draining, Failed, Idle, InUse, Initializing, Maintenance, Ready,
            Terminated,
        };

        match (self, target) {
            // Created: instance exists but setup hasn't started yet.
            // Can be discarded immediately (Terminated) without going through
            // Initializing if it is never needed (e.g. pool shrinks on startup).
            (Created, Initializing) => true,
            (Created, Failed) => true, // setup failed before init started
            (Created, Terminated) => true, // discarded before use

            // Initializing: Resource::create is in progress.
            // Only two outcomes: success → Ready, failure → Failed.
            (Initializing, Ready) => true,
            (Initializing, Failed) => true,

            // Ready: instance passed is_reusable and is waiting in the idle queue.
            // InUse: caller acquired the Guard.
            // Idle: pool explicitly marks it idle after a release (implementation detail).
            // Maintenance: background validation/recycle cycle started.
            // Draining: shutdown or reload initiated, no new acquisitions.
            (Ready, InUse) => true,
            (Ready, Idle) => true,
            (Ready, Maintenance) => true,
            (Ready, Draining) => true,
            (Ready, Failed) => true, // health check failed while idle

            // InUse: Guard is held by a caller.
            // Ready/Idle: Guard dropped without taint — recycle succeeded.
            // Cannot go to Maintenance or Draining directly; the pool waits
            // for the Guard to be dropped first.
            (InUse, Ready) => true,
            (InUse, Idle) => true,
            (InUse, Failed) => true, // guard.taint() was called, or recycle failed

            // Idle: in idle queue, not yet validated for reuse.
            // Maintenance: validation (is_reusable) is running.
            // Cleanup: idle_timeout or max_lifetime expired; evict directly.
            (Idle, InUse) => true,
            (Idle, Ready) => true, // validated by is_reusable
            (Idle, Maintenance) => true,
            (Idle, Draining) => true, // shutdown while idle
            (Idle, Cleanup) => true,  // idle/lifetime timeout
            (Idle, Failed) => true,   // health check failed while idle

            // Maintenance: is_reusable or recycle running in the background.
            // Returns to Ready on success, or Cleanup/Failed on error.
            (Maintenance, Ready) => true,
            (Maintenance, Failed) => true,
            (Maintenance, Cleanup) => true, // recycle failed; remove from pool

            // Draining: pool is shutting down or reloading config.
            // All in-flight acquire attempts are rejected. Waits for active
            // Guards to drop, then moves to Cleanup.
            (Draining, Cleanup) => true,
            (Draining, Failed) => true,

            // Cleanup: Resource::cleanup is running (socket close, buffer flush).
            // Terminated is the normal outcome; Failed if cleanup itself errors.
            (Cleanup, Terminated) => true,
            (Cleanup, Failed) => true,

            // Failed: instance is permanently unusable.
            // Cleanup runs Resource::cleanup for best-effort resource release.
            // Terminated skips cleanup when the resource was never fully created.
            (Failed, Cleanup) => true,
            (Failed, Terminated) => true,

            // Terminated is a sink — no further transitions are possible.
            (Terminated, _) => false,

            // Self-transitions are always valid (idempotent state assertions).
            (state, target) if *state == target => true,

            // Everything else is an invalid transition.
            _ => false,
        }
    }

    /// Get the next logical state(s) for this lifecycle state
    #[must_use]
    pub fn next_states(&self) -> &'static [Lifecycle] {
        use Lifecycle::{
            Cleanup, Created, Draining, Failed, Idle, InUse, Initializing, Maintenance, Ready,
            Terminated,
        };

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

impl fmt::Display for Lifecycle {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lifecycle_state_availability() {
        assert!(Lifecycle::Ready.is_available());
        assert!(Lifecycle::Idle.is_available());
        assert!(!Lifecycle::Created.is_available());
        assert!(!Lifecycle::Failed.is_available());
    }

    #[test]
    fn test_lifecycle_state_terminal() {
        assert!(Lifecycle::Terminated.is_terminal());
        assert!(Lifecycle::Failed.is_terminal());
        assert!(!Lifecycle::Ready.is_terminal());
    }

    #[test]
    fn test_lifecycle_state_transitions() {
        assert!(Lifecycle::Created.can_transition_to(Lifecycle::Initializing));
        assert!(Lifecycle::Initializing.can_transition_to(Lifecycle::Ready));
        assert!(Lifecycle::Ready.can_transition_to(Lifecycle::InUse));

        // Invalid transitions
        assert!(!Lifecycle::Created.can_transition_to(Lifecycle::InUse));
        assert!(!Lifecycle::Terminated.can_transition_to(Lifecycle::Ready));
    }

    #[test]
    fn test_lifecycle_state_can_acquire() {
        assert!(Lifecycle::Ready.can_acquire());
        assert!(Lifecycle::Idle.can_acquire());
        assert!(!Lifecycle::InUse.can_acquire());
        assert!(!Lifecycle::Failed.can_acquire());
    }
}
