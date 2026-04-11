//! Daemon topology (secondary) — background run loop.

use std::{future::Future, time::Duration};

use tokio_util::sync::CancellationToken;

use crate::{ctx::Ctx, resource::Resource};

/// Policy for restarting a daemon after it exits.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RestartPolicy {
    /// Never restart — if the daemon exits, it stays down.
    Never,
    /// Restart only if the daemon exited with an error.
    #[default]
    OnFailure,
    /// Always restart, regardless of exit reason.
    Always,
}

/// Daemon topology (secondary) — background run loop.
///
/// A secondary topology for resources that need a long-running background
/// task (e.g., polling, streaming, periodic sync). The daemon runs until
/// cancelled or until it returns.
pub trait Daemon: Resource {
    /// Runs the daemon loop.
    ///
    /// The implementation should select on `cancel` for cooperative shutdown.
    /// When the token is cancelled, the daemon should clean up and return `Ok(())`.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the daemon encounters a fatal error.
    fn run(
        &self,
        runtime: &Self::Runtime,
        ctx: &dyn Ctx,
        cancel: CancellationToken,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// Configuration types for daemon topology.
pub mod config {
    use super::*;

    /// Daemon configuration.
    #[derive(Debug, Clone)]
    pub struct Config {
        /// When to restart the daemon after it exits.
        pub restart_policy: RestartPolicy,
        /// Maximum number of restarts before giving up.
        pub max_restarts: u32,
        /// Backoff duration between restarts.
        pub restart_backoff: Duration,
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                restart_policy: RestartPolicy::default(),
                max_restarts: 5,
                restart_backoff: Duration::from_secs(1),
            }
        }
    }
}
