//! Engine daemon module — long-running worker primitives (per ADR-0037).
//!
//! Hosts the `Daemon` trait + `DaemonRuntime` (per-daemon background task with
//! restart policy) + `DaemonRegistry` (engine-side dispatcher across all
//! registered daemons). EventSource adapter onto the `TriggerAction` substrate
//! lives in [`event_source`].
//!
//! Migrated from `nebula-resource` per ADR-0037 ("Daemon / EventSource engine
//! fold") to honor canon §3.5 ("Resource = pool/SDK client").
//!
//! # Cancellation
//!
//! `DaemonRegistry` owns a parent [`tokio_util::sync::CancellationToken`]. Each
//! `DaemonRuntime` registered through it inherits the parent token; calling
//! [`DaemonRegistry::shutdown`] cascades to every daemon loop. Per-run lifecycle
//! is managed by `DaemonRuntime` (see its module docs).
//!
//! # Module layout
//!
//! - [`mod@self`] — `Daemon` trait, `RestartPolicy`, `DaemonConfig`
//! - [`runtime`] — `DaemonRuntime<D>` per-daemon background task
//! - [`registry`] — `DaemonRegistry` engine-side dispatcher
//! - [`event_source`] — `EventSource` trait + `EventSourceAdapter<E>` (TriggerAction adapter)

pub mod event_source;
pub mod registry;
pub mod runtime;

use std::{future::Future, time::Duration};

use nebula_resource::{Resource, ResourceContext};
use tokio_util::sync::CancellationToken;

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

/// Daemon — long-running background worker.
///
/// A long-running worker that runs until cancelled or until it returns.
/// Implementations select on `cancel` for cooperative shutdown; `DaemonRuntime`
/// drives the restart loop per the configured [`RestartPolicy`].
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
        ctx: &ResourceContext,
        cancel: CancellationToken,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// Configuration types for the daemon module.
pub mod config {
    use super::{Duration, RestartPolicy};

    /// Daemon configuration.
    #[derive(Debug, Clone)]
    #[non_exhaustive]
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

pub use config::Config as DaemonConfig;
pub use event_source::{EventSource, EventSourceAdapter, EventSourceConfig, EventSourceRuntime};
pub use registry::{AnyDaemonHandle, DaemonError, DaemonRegistry};
pub use runtime::DaemonRuntime;
