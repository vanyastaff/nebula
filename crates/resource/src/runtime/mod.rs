//! Topology runtime implementations.
//!
//! Each topology trait ([`Pooled`], [`Resident`], [`Service`]) has a
//! corresponding runtime struct that manages instance lifecycle, and a
//! dispatch enum ([`TopologyRuntime`]) that erases the topology at the
//! registration level.
//!
//! [`Pooled`]: crate::topology::pooled::Pooled
//! [`Resident`]: crate::topology::resident::Resident
//! [`Service`]: crate::topology::service::Service

pub mod managed;
pub mod pool;
pub mod resident;
pub mod service;

use crate::resource::Resource;

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
    // Transport, Exclusive, EventSource, Daemon — added in Phase 4b.
}
