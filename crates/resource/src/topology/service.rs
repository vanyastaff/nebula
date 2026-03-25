//! Service topology — long-lived runtime, short-lived tokens for callers.

use std::future::Future;

use crate::ctx::Ctx;
use crate::resource::Resource;

/// How the service manages token lifecycle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TokenMode {
    /// Token is a cheap clone; release is a no-op. Results in an owned handle.
    #[default]
    Cloned,
    /// Token is a tracked resource; release is required. Results in a guarded handle.
    Tracked,
}

/// Service topology — long-lived runtime, short-lived tokens for callers.
///
/// The runtime lives for the duration of the resource, and callers acquire
/// lightweight tokens (e.g., API keys, session handles) scoped to their
/// execution context.
pub trait Service: Resource {
    /// How this service manages token lifecycle.
    const TOKEN_MODE: TokenMode = TokenMode::Cloned;

    /// Acquires a token from the running service.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if token acquisition fails.
    fn acquire_token(
        &self,
        runtime: &Self::Runtime,
        ctx: &dyn Ctx,
    ) -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;

    /// Releases a previously acquired token back to the service.
    ///
    /// The default implementation is a no-op (suitable for [`TokenMode::Cloned`]).
    fn release_token(
        &self,
        _runtime: &Self::Runtime,
        _token: Self::Lease,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }
}

/// Configuration types for service topology.
pub mod config {
    /// Service configuration.
    #[derive(Debug, Clone, Default)]
    pub struct Config {
        /// Timeout for draining active tokens during shutdown.
        /// `None` means wait indefinitely.
        pub drain_timeout: Option<std::time::Duration>,
    }
}
