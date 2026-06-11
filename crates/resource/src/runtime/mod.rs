//! Topology runtime implementations.
//!
//! Each topology trait ([`Pooled`], [`Resident`]) has a corresponding runtime
//! struct that manages instance lifecycle, and a dispatch enum
//! ([`TopologyRuntime`]) that erases the topology at the registration level.
//!
//! [`Pooled`]: crate::topology::pooled::Pooled
//! [`Resident`]: crate::topology::resident::Resident

pub mod managed;
pub mod pool;
pub mod resident;

use crate::{resource::Resource, topology_tag::TopologyTag};

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
}

impl<R: Resource> TopologyRuntime<R> {
    /// Returns the topology tag for this runtime variant.
    pub fn tag(&self) -> TopologyTag {
        match self {
            Self::Pool(_) => TopologyTag::Pool,
            Self::Resident(_) => TopologyTag::Resident,
        }
    }
}
