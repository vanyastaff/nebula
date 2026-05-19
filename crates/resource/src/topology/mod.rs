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
//! | [`Bounded`] | One runtime, capped leases — folds Service/Transport/Exclusive behind a [`Cap`](bounded::CapMarker) typestate |
//!
//! [`Bounded`] is the parameterized successor to `Service`/`Transport`/
//! `Exclusive`; the three are kept alongside it during the migration.
//!
//! `Daemon` and `EventSource` live in `nebula_engine::daemon` per engine daemon topology —
//! integration model boundary reserves "Resource" for pool/SDK clients.

pub mod bounded;
pub mod exclusive;
pub mod pooled;
pub mod resident;
pub mod service;
pub mod transport;

pub use bounded::{
    Bounded, BoundedRelease, CapMarker, Capped, Exclusive as ExclusiveCap, Unbounded,
};
pub use exclusive::Exclusive;
pub use pooled::{BrokenCheck, InstanceMetrics, Pooled, RecycleDecision};
pub use resident::Resident;
pub use service::{Service, TokenMode};
pub use transport::Transport;
