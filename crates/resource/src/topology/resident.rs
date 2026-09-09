//! Resident topology — one retained instance shared by owning lease entries.

use std::time::Duration;

use crate::resource::Provider;

/// Resident provider hooks — one instance shared through framework-owned Arcs.
///
/// The framework creates and retains the instance, then lends shared access
/// through guards. The instance does not need to implement `Clone`.
/// Suitable for stateless or internally-pooled clients (e.g., `reqwest::Client`).
/// A resource that declares `type Topology = Resident<Self>` implements this
/// trait so the framework [`Resident`](crate::topology::Resident) topology can
/// drive its liveness policy.
///
/// # Acquire bounds
///
/// [`Manager::acquire_resident`](crate::Manager::acquire_resident) requires:
/// - `R: Send + Sync + 'static`
/// - `R::Instance: Send + Sync + 'static`
pub trait ResidentProvider: Provider {
    /// Sync O(1) liveness check. NO I/O, NO blocking.
    ///
    /// The default implementation always reports alive.
    fn is_alive_sync(&self, _instance: &Self::Instance) -> bool {
        true
    }

    /// How long before the shared instance is considered stale.
    ///
    /// Returns `None` for stateless clients that never go stale.
    fn stale_after(&self) -> Option<Duration> {
        None
    }
}

/// Configuration types for resident topology.
pub mod config {
    use std::time::Duration;

    /// Resident configuration.
    #[derive(Debug, Clone)]
    pub struct Config {
        /// Whether to automatically recreate the instance on failure.
        pub recreate_on_failure: bool,
        /// Maximum time to wait for `Provider::create()` before aborting.
        ///
        /// Prevents a hanging backend from holding the create lock forever,
        /// which would deadlock all subsequent acquires.
        pub create_timeout: Duration,
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                recreate_on_failure: false,
                create_timeout: Duration::from_secs(30),
            }
        }
    }
}
