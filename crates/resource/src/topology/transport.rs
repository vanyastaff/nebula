//! Transport topology — shared connection, multiplexed sessions.

use std::future::Future;

use crate::ctx::Ctx;
use crate::resource::Resource;

/// Transport topology — shared connection, multiplexed sessions.
///
/// A single long-lived transport (e.g., HTTP/2, gRPC channel, AMQP connection)
/// multiplexes many short-lived sessions (streams, channels) for callers.
pub trait Transport: Resource {
    /// Opens a new session on the transport.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the session cannot be opened.
    fn open_session(
        &self,
        transport: &Self::Runtime,
        ctx: &dyn Ctx,
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
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                max_sessions: 10,
                keepalive_interval: Some(Duration::from_secs(30)),
            }
        }
    }
}
