//! Topology traits for resource management.
//!
//! Each access pattern pairs a **provider-hooks trait** (author-implemented,
//! extends [`Provider`](crate::resource::Provider)) with a **framework
//! topology struct** that implements the open [`Topology`] contract and drives
//! those hooks over a framework-owned [`InstanceStore`]:
//!
//! | Pattern | Provider hooks | Framework topology |
//! |---------|----------------|--------------------|
//! | Pool    | [`PoolProvider`] | [`Pooled<R>`](crate::runtime::pool::Pooled) |
//! | Resident| [`ResidentProvider`] | [`Resident<R>`](crate::runtime::resident::Resident) |
//! | Bounded | [`BoundedProvider`] | [`Bounded<R>`](crate::runtime::bounded::Bounded) |
//!
//! `Daemon` and `EventSource` live in `nebula_engine::daemon` per engine daemon topology —
//! integration model boundary reserves "Resource" for pool/SDK clients.
//!
//! For custom topologies see the open [`Topology`] trait in [`contract`] and the
//! framework-owned [`InstanceStore`] in [`store`].

pub mod bounded;
pub mod contract;
pub mod pooled;
pub mod resident;
pub mod store;

pub use bounded::{BoundedMode, BoundedProvider};
pub use contract::{
    AdmissionPhase, AdmissionStatus, Load, MaintenanceSchedule, NoTopology, Ticket, Topology,
    Unavailable,
};
pub use pooled::{BrokenCheck, InstanceMetrics, PoolProvider, RecycleDecision};
pub use resident::ResidentProvider;
pub use store::{CheckedOut, Checkout, InstanceStore, ReturnOutcome};

/// Framework topology structs that implement the open [`Topology`] contract.
///
/// These live in [`crate::runtime`] (they hold the resource handle and drive
/// the [`PoolProvider`] / [`ResidentProvider`] hooks over an
/// [`InstanceStore`]); re-exported here so authors write
/// `type Topology = nebula_resource::topology::Pooled<Self>`.
pub use crate::runtime::bounded::Bounded;
pub use crate::runtime::pool::Pooled;
pub use crate::runtime::resident::Resident;
