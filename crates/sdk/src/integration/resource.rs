//! Resource and custom topology authoring for trusted in-process integrations.
//!
//! Implement [`Provider`] and select a built-in topology or implement [`Topology`].
//! Topology hooks receive a read-only [`StoreView`] of the framework idle store:
//! they can observe it and read idle entries in place, but checkout, return,
//! eviction and the revoke fence stay framework-owned, so an adapter cannot move
//! an idle entry out of framework accounting. Long-lived roots go through the
//! borrowed [`RetainedStore`]; its mutation methods are lifecycle capabilities
//! entrusted to the adapter, not global registry or tenant authority, and Rust
//! cannot prevent a trusted adapter from hiding aliases of a retained lease.
//!
//! Rate limits are declared by overriding [`Provider::resilience`] with a
//! [`ResiliencePolicy`]; the client built in `create` is wrapped once with
//! [`ResourceContext::limits`] and [`ResourceLimiter::wrap`], and a
//! [`Throttle`] tells the provider's "slow down" apart from other outcomes.
//!
//! Runtime registration, dispatch, and cleanup queues remain engine-owned.

pub use nebula_core::{ResourceKey, resource_key};
pub use nebula_resource::rate_limit::{
    DEFAULT_MAX_PENALTY, LimitScope, Limited, LimitedError, NoThrottle, OnError, Override, Rate,
    RateLimitSettings, ResiliencePolicy, ResourceLimiter, Throttle, Verdict, on_error,
    retry_after_from_header,
};
pub use nebula_resource::topology::{
    AdmissionPhase, BrokenCheck, CreatedEntry, HookFault, IdleRead, InstanceMetrics, Load,
    MaintenanceSchedule, NoTopology, PoolStrategy, RecycleDecision, ReplaceStatus, RetainStatus,
    RetainedId, RetainedLease, RetainedStore, RetireStatus, StoreRejection, StoreView, Ticket,
    Topology, Unavailable,
};
pub use nebula_resource::{
    Bounded, BoundedMode, BoundedProvider, CheckCost, ClassifyError, Error, ErrorKind,
    HasCredentialSlots, LeaseClosing, PoolConfig, PoolProvider, Pooled, Provider, ReleaseOutcome,
    Resident, ResidentConfig, ResidentProvider, Resource, ResourceConfig, ResourceContext,
    ResourceGuard, ResourceMetadataDraft, SlotCell, TeardownCx, TeardownReason, TopologyTag,
    no_credential_slots,
};
