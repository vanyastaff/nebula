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
//! The wrapped-client calls — [`Limited::run`] and its `run_*` variants, and
//! [`Limited::unlimited`] — are interim surface: each closure books one permit
//! and counts as one provider call. A row that wraps a client reports the
//! `InterimPerClosure` (`interim_per_closure`) rate-limit profile in resource
//! status; the managed call facade replaces the closure family. Without a
//! wrap, a declared rate books one permit per acquire.
//!
//! The managed call facade turns a lease into [`Managed`] with
//! [`ResourceGuard::into_managed`]. Each provider call is an [`Operation`]
//! submitted as a [`Unit`]; inside it, [`OpCx::attempt`] admits one provider
//! [`Attempt`] against the lease, books its [`Cost`] and hands out the
//! instance and the unit's pinned credential slots ([`PinSlots`]). Each
//! attempt is settled with a [`SentState`], and a failed unit's [`OpError`]
//! says from that and the operation's [`Effect`] whether a retry is safe. A
//! row used this way reports the `PerAttempt` (`per_attempt`) profile. The
//! facade is not frozen yet, so it is not in the prelude.
//!
//! Runtime registration, dispatch, and cleanup queues remain engine-owned.

pub use nebula_core::{ResourceKey, resource_key};
pub use nebula_resource::call::{
    Attempt, Cost, Effect, Managed, OpCx, OpError, Operation, PinSlots, SentState, Unit,
};
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
    Bounded, BoundedMode, BoundedProvider, CheckCost, ClassifyError, CredentialUnavailableReason,
    Error, ErrorKind, HasCredentialSlots, LeaseClosing, PoolConfig, PoolProvider, Pooled, Provider,
    ReleaseOutcome, Resident, ResidentConfig, ResidentProvider, Resource, ResourceConfig,
    ResourceContext, ResourceGuard, ResourceMetadataDraft, SlotCell, TeardownCx, TeardownReason,
    TopologyTag, no_credential_slots,
};
