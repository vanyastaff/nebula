//! Topology runtime implementations.
//!
//! Each topology trait ([`Pooled`], [`Resident`], [`Service`], [`Transport`],
//! [`Exclusive`], [`EventSource`], [`Daemon`]) has a corresponding runtime
//! struct that manages instance lifecycle, and a dispatch enum
//! ([`TopologyRuntime`]) that erases the topology at the registration level.
//!
//! [`Pooled`]: crate::topology::pooled::Pooled
//! [`Resident`]: crate::topology::resident::Resident
//! [`Service`]: crate::topology::service::Service
//! [`Transport`]: crate::topology::transport::Transport
//! [`Exclusive`]: crate::topology::exclusive::Exclusive
//! [`EventSource`]: crate::topology::event_source::EventSource
//! [`Daemon`]: crate::topology::daemon::Daemon

pub mod daemon;
pub mod event_source;
pub mod exclusive;
pub mod managed;
pub mod pool;
pub mod resident;
pub mod service;
pub mod transport;

use crate::resource::Resource;
use crate::topology_tag::TopologyTag;

/// Dispatch enum for all topology runtimes.
///
/// Each variant holds the runtime state for a specific topology. The
/// engine stores one `TopologyRuntime<R>` per registered resource,
/// inside [`ManagedResource`](managed::ManagedResource).
pub enum TopologyRuntime<R: Resource> {
    /// Pool of N interchangeable instances with checkout/recycle.
    Pool(pool::PoolRuntime<R>),
    /// Single shared instance, clone on acquire.
    Resident(resident::ResidentRuntime<R>),
    /// Long-lived runtime with short-lived tokens.
    Service(service::ServiceRuntime<R>),
    /// Shared connection with multiplexed sessions.
    Transport(transport::TransportRuntime<R>),
    /// One caller at a time via semaphore(1).
    Exclusive(exclusive::ExclusiveRuntime<R>),
    /// Pull-based event subscription (secondary topology).
    EventSource(event_source::EventSourceRuntime<R>),
    /// Background run loop with restart policy (secondary topology).
    Daemon(daemon::DaemonRuntime<R>),
}

impl<R: Resource> TopologyRuntime<R> {
    /// Returns the topology tag for this runtime variant.
    pub fn tag(&self) -> TopologyTag {
        match self {
            Self::Pool(_) => TopologyTag::Pool,
            Self::Resident(_) => TopologyTag::Resident,
            Self::Service(_) => TopologyTag::Service,
            Self::Transport(_) => TopologyTag::Transport,
            Self::Exclusive(_) => TopologyTag::Exclusive,
            Self::EventSource(_) => TopologyTag::EventSource,
            Self::Daemon(_) => TopologyTag::Daemon,
        }
    }
}
