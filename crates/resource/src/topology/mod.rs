//! Topology traits for resource management.
//!
//! Each access pattern pairs a **provider-hooks trait** (author-implemented,
//! extends [`Provider`](crate::resource::Provider)) with a **framework
//! topology struct** that implements the open [`Topology`] contract and drives
//! those hooks over a framework-owned [`InstanceStore`]:
//!
//! | Pattern | Provider hooks | Framework topology |
//! |---------|----------------|--------------------|
//! | Pool    | [`PoolProvider`] | [`Pooled<R>`] |
//! | Resident| [`ResidentProvider`] | [`Resident<R>`] |
//! | Bounded | [`BoundedProvider`] | [`Bounded<R>`] |
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
pub use store::{CheckedOut, Checkout, InstanceStore, PoolStrategy, ReturnOutcome};

/// Framework topology structs that implement the open [`Topology`] contract.
///
/// These live in a private `runtime` implementation module (they hold the
/// resource handle and drive the [`PoolProvider`] / [`ResidentProvider`]
/// hooks over an [`InstanceStore`]) and are re-exported here — and again at
/// the crate root — as the two blessed public paths; authors write
/// `type Topology = nebula_resource::topology::Pooled<Self>` (or the
/// crate-root `nebula_resource::Pooled` alias) without reaching into the
/// implementation module directly.
///
/// **Rejected alternative: physically co-locating `runtime/` under
/// `topology/`.** A prior draft proposed moving the ~2,500 lines of
/// `runtime::{pool,resident,bounded,managed,acquire_loop}` into this module
/// so "the topology" lived in one directory. Rejected: the visibility-first
/// shape above achieves the same one-canonical-path goal in a ~20-line diff
/// (the std/tokio pattern — private implementation modules, public
/// re-exports) without burying the 218-line author-facing [`PoolProvider`] /
/// [`ResidentProvider`] / [`BoundedProvider`] contract under framework
/// acquire-loop internals in a file-tree move. No consumer referenced
/// `nebula_resource::runtime::*` directly, so there was no compatibility
/// reason to prefer the larger move either.
pub use crate::runtime::bounded::Bounded;
pub use crate::runtime::pool::Pooled;
pub use crate::runtime::resident::Resident;
