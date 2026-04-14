//! Exclusive topology — one caller at a time via semaphore.

use std::future::Future;

use crate::resource::Resource;

/// Exclusive topology — one caller at a time via semaphore.
///
/// The runtime is protected by a semaphore permit. Only one caller can
/// hold the lease at a time. Suitable for resources that are not
/// concurrency-safe (e.g., serial ports, single-writer databases).
///
/// # Acquire bounds
///
/// [`Manager::acquire_exclusive`](crate::Manager::acquire_exclusive) requires:
/// - `R: Send + Sync + 'static`
/// - `R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static`
/// - `R::Lease: Send + 'static`
pub trait Exclusive: Resource {
    /// Resets the resource state after each exclusive use.
    ///
    /// Called after the lease is released, before the **next** caller can
    /// acquire the exclusive permit. The runtime guarantees this ordering
    /// by holding the semaphore permit until `reset` resolves (#384).
    ///
    /// The default implementation is a no-op.
    fn reset(
        &self,
        _runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }
}

/// Configuration types for exclusive topology.
pub mod config {
    use std::time::Duration;

    /// Exclusive configuration.
    #[derive(Debug, Clone)]
    pub struct Config {
        /// Timeout for acquiring the exclusive lock.
        pub acquire_timeout: Duration,
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                acquire_timeout: Duration::from_secs(30),
            }
        }
    }
}
