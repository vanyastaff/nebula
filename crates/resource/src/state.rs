//! Resource lifecycle state tracking.
//!
//! [`ResourcePhase`] represents the current lifecycle phase of a resource,
//! and [`ResourceStatus`] bundles phase with generation and error information.

/// Lifecycle phase of a managed resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourcePhase {
    /// Resource is being created for the first time.
    Initializing,
    /// Resource is healthy and accepting requests.
    Ready,
    /// Resource is being hot-reloaded with new config/credentials.
    Reloading,
    /// Resource is draining in-flight work before shutdown.
    Draining,
    /// Resource is shutting down.
    ShuttingDown,
    /// Resource has failed and is not usable.
    Failed,
}

impl ResourcePhase {
    /// Returns `true` if the resource can accept new acquire requests.
    pub fn is_accepting(&self) -> bool {
        matches!(self, Self::Ready | Self::Reloading)
    }

    /// Returns `true` if the resource is in a terminal or error state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::ShuttingDown | Self::Failed)
    }
}

impl std::fmt::Display for ResourcePhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Initializing => "initializing",
            Self::Ready => "ready",
            Self::Reloading => "reloading",
            Self::Draining => "draining",
            Self::ShuttingDown => "shutting_down",
            Self::Failed => "failed",
        };
        f.write_str(label)
    }
}

/// Snapshot of a resource's current status.
#[derive(Debug, Clone)]
pub struct ResourceStatus {
    /// Current lifecycle phase.
    pub phase: ResourcePhase,
    /// Monotonically increasing generation counter (incremented on reload).
    pub generation: u64,
    /// Human-readable description of the last error, if any.
    pub last_error: Option<String>,
}

impl ResourceStatus {
    /// Creates a new status in the initializing phase.
    pub fn new() -> Self {
        Self {
            phase: ResourcePhase::Initializing,
            generation: 0,
            last_error: None,
        }
    }
}

impl Default for ResourceStatus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_is_accepting() {
        assert!(ResourcePhase::Ready.is_accepting());
    }

    #[test]
    fn reloading_is_accepting() {
        assert!(ResourcePhase::Reloading.is_accepting());
    }

    #[test]
    fn initializing_is_not_accepting() {
        assert!(!ResourcePhase::Initializing.is_accepting());
    }

    #[test]
    fn draining_is_not_accepting() {
        assert!(!ResourcePhase::Draining.is_accepting());
    }

    #[test]
    fn shutting_down_is_terminal() {
        assert!(ResourcePhase::ShuttingDown.is_terminal());
    }

    #[test]
    fn failed_is_terminal() {
        assert!(ResourcePhase::Failed.is_terminal());
    }

    #[test]
    fn ready_is_not_terminal() {
        assert!(!ResourcePhase::Ready.is_terminal());
    }

    #[test]
    fn display_formats_correctly() {
        assert_eq!(ResourcePhase::Ready.to_string(), "ready");
        assert_eq!(ResourcePhase::ShuttingDown.to_string(), "shutting_down");
        assert_eq!(ResourcePhase::Failed.to_string(), "failed");
    }

    #[test]
    fn default_status_is_initializing() {
        let status = ResourceStatus::default();
        assert_eq!(status.phase, ResourcePhase::Initializing);
        assert_eq!(status.generation, 0);
        assert!(status.last_error.is_none());
    }
}
