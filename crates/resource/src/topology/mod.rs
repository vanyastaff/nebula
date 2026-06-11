//! Topology traits for resource management.
//!
//! Each topology describes a different access pattern for resources:
//!
//! | Topology | Pattern |
//! |----------|---------|
//! | [`Pooled`] | N interchangeable instances with checkout/recycle |
//! | [`Resident`] | One shared instance, clone on acquire |
//!
//! `Daemon` and `EventSource` live in `nebula_engine::daemon` per engine daemon topology —
//! integration model boundary reserves "Resource" for pool/SDK clients.
//!
//! For custom topologies see the open [`Topology`] trait in [`contract`] and the
//! framework-owned [`InstanceStore`] in [`store`].

pub mod contract;
pub mod pooled;
pub mod resident;
pub mod store;

pub use contract::{AdmissionPhase, Lease, Load, Ticket, Topology, Unavailable};
pub use pooled::{BrokenCheck, InstanceMetrics, Pooled, RecycleDecision};
pub use resident::Resident;
pub use store::{CheckedOut, Checkout, InstanceStore, ReturnOutcome};
