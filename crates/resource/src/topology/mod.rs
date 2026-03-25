//! Topology traits for resource management.
//!
//! Each topology describes a different access pattern for resources:
//!
//! | Topology | Pattern |
//! |----------|---------|
//! | [`Pooled`] | N interchangeable instances with checkout/recycle |
//! | [`Resident`] | One shared instance, clone on acquire |
//! | [`Service`] | Long-lived runtime, short-lived tokens |
//! | [`Transport`] | Shared connection, multiplexed sessions |
//! | [`Exclusive`] | One caller at a time via semaphore |
//! | [`EventSource`] | Pull-based event subscription (secondary) |
//! | [`Daemon`] | Background run loop (secondary) |

pub mod daemon;
pub mod event_source;
pub mod exclusive;
pub mod pooled;
pub mod resident;
pub mod service;
pub mod transport;

pub use daemon::{Daemon, RestartPolicy};
pub use event_source::EventSource;
pub use exclusive::Exclusive;
pub use pooled::{BrokenCheck, InstanceMetrics, Pooled, RecycleDecision};
pub use resident::Resident;
pub use service::{Service, TokenMode};
pub use transport::Transport;
