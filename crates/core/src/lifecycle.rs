//! Hierarchical cancellation primitive (spec 08).

use std::time::Duration;

use tokio_util::{sync::CancellationToken, task::TaskTracker};

/// One layer in the cancellation hierarchy (spec 08).
pub struct LayerLifecycle {
    /// Cancellation token for this layer.
    pub token: CancellationToken,
    /// Task tracker for child tasks.
    pub tasks: TaskTracker,
}

impl LayerLifecycle {
    /// Create root layer (process level).
    pub fn root() -> Self {
        Self {
            token: CancellationToken::new(),
            tasks: TaskTracker::new(),
        }
    }

    /// Create child layer -- inherits cancellation from parent.
    pub fn child(&self) -> Self {
        Self {
            token: self.token.child_token(),
            tasks: TaskTracker::new(),
        }
    }

    /// Two-phase graceful shutdown.
    pub async fn shutdown(&self, grace: Duration) -> ShutdownOutcome {
        self.token.cancel();
        self.tasks.close();
        tokio::select! {
            () = self.tasks.wait() => ShutdownOutcome::Graceful,
            () = tokio::time::sleep(grace) => ShutdownOutcome::GraceExceeded,
        }
    }
}

/// Result of a graceful shutdown attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShutdownOutcome {
    /// All children completed within grace period.
    Graceful,
    /// Grace period elapsed with children still running.
    GraceExceeded,
}
