//! Transport topology — shared connection, multiplexed sessions.

use std::future::Future;

use crate::{context::ResourceContext, resource::Resource};

/// Transport topology — shared connection, multiplexed sessions.
///
/// A single long-lived transport (e.g., HTTP/2, gRPC channel, AMQP connection)
/// multiplexes many short-lived sessions (streams, channels) for callers.
///
/// # Acquire bounds
///
/// [`Manager::acquire_transport`](crate::Manager::acquire_transport) requires:
/// - `R: Clone + Send + Sync + 'static`
/// - `R::Runtime: Send + Sync + 'static`
/// - `R::Lease: Send + 'static`
pub trait Transport: Resource {
    /// Opens a new session on the transport.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the session cannot be opened.
    fn open_session(
        &self,
        transport: &Self::Runtime,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;

    /// Closes a session, optionally reporting whether it ended healthily.
    ///
    /// The default implementation is a no-op.
    fn close_session(
        &self,
        _transport: &Self::Runtime,
        _session: Self::Lease,
        _healthy: bool,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }

    /// Sends a keepalive on the transport to prevent idle disconnection.
    ///
    /// The default implementation is a no-op.
    fn keepalive(
        &self,
        _transport: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }
}

/// Configuration types for transport topology.
pub mod config {
    use std::time::Duration;

    /// Transport configuration.
    #[derive(Debug, Clone)]
    pub struct Config {
        /// Maximum number of concurrent sessions on the transport.
        pub max_sessions: u32,
        /// Interval for sending keepalives. `None` disables keepalives.
        pub keepalive_interval: Option<Duration>,
        /// Timeout for acquiring a session semaphore permit.
        ///
        /// When all sessions are in use, acquire calls will wait at most this
        /// long before returning a backpressure error. Callers may override
        /// this per-request via acquire options.
        pub acquire_timeout: Duration,
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                max_sessions: 10,
                keepalive_interval: Some(Duration::from_secs(30)),
                acquire_timeout: Duration::from_secs(30),
            }
        }
    }
}
