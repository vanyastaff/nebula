//! Resource and custom topology authoring for trusted in-process integrations.
//!
//! Implement [`Provider`] and select a built-in topology or implement [`Topology`].
//! The framework supplies registration-local [`InstanceStore`] and [`RetainedStore`]
//! access. Their inherent mutation methods are lifecycle capabilities entrusted to
//! the adapter; they are not global registry or tenant authority. Authors must
//! preserve ownership and credential fences. Rust cannot prevent trusted adapters
//! from hiding aliases or dropping entries extracted from a store.
//!
//! Runtime registration, dispatch, and cleanup queues remain engine-owned.

pub use nebula_core::{ResourceKey, resource_key};
pub use nebula_resource::topology::{
    AdmissionPhase, BrokenCheck, CreatedEntry, HookFault, InstanceMetrics, InstanceStore, Load,
    MaintenanceSchedule, NoTopology, PoolStrategy, RecycleDecision, ReplaceStatus, RetainStatus,
    RetainedId, RetainedLease, RetainedStore, RetireStatus, StoreRejection, Ticket, Topology,
    Unavailable,
};
pub use nebula_resource::{
    Bounded, BoundedMode, BoundedProvider, CheckCost, ClassifyError, Error, ErrorKind,
    HasCredentialSlots, PoolConfig, PoolProvider, Pooled, Provider, ReleaseOutcome, Resident,
    ResidentConfig, ResidentProvider, Resource, ResourceConfig, ResourceContext, ResourceGuard,
    ResourceMetadata, SlotCell, TeardownCx, TeardownReason, TopologyTag, no_credential_slots,
};
