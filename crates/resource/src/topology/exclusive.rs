//! Exclusive topology — one caller at a time via semaphore.

use std::future::Future;

use crate::resource::Resource;

/// Exclusive topology — one caller at a time via semaphore.
///
/// The runtime is protected by a semaphore permit. Only one caller can
/// hold the lease at a time. Suitable for resources that are not
/// concurrency-safe (e.g., serial ports, single-writer databases).
pub trait Exclusive: Resource {
    /// Resets the resource state after each exclusive use.
    ///
    /// Called when the lease is released, before the next caller can acquire.
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
    /// Exclusive configuration.
    #[derive(Debug, Clone, Default)]
    pub struct Config {}
}
