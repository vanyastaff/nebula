//! Resident topology — one shared instance, clone on acquire.

use std::time::Duration;

use crate::resource::Resource;

/// Resident topology — one shared instance, clone on acquire.
///
/// The runtime is created once and shared across all callers via `Clone`.
/// Suitable for stateless or internally-pooled clients (e.g., `reqwest::Client`).
pub trait Resident: Resource
where
    Self::Lease: Clone,
{
    /// Sync O(1) liveness check. NO I/O, NO blocking.
    ///
    /// The default implementation always reports alive.
    fn is_alive_sync(&self, _runtime: &Self::Runtime) -> bool {
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
        /// Maximum time to wait for `Resource::create()` before aborting.
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
