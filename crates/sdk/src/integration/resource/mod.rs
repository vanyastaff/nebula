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
//! Provider calls go through the managed call facade: a lease becomes a
//! [`Managed`] with [`ResourceGuard::into_managed`], and each provider call is
//! an [`Operation`] submitted as a [`Unit`]. Inside it, [`OpCx::attempt`]
//! admits one provider [`Attempt`] against the lease, books its [`Cost`] and
//! hands out the instance and the unit's pinned credential slots
//! ([`PinSlots`]). Each attempt is settled with a [`SentState`], and a failed
//! unit's [`OpError`] says from that and the operation's [`Effect`] whether a
//! retry is safe. On a strict manager every attempt also reads its bound
//! credentials' availability first. A row used this way reports the
//! `PerAttempt` (`per_attempt`) rate-limit profile. The facade is not frozen
//! yet, so it is not in the prelude.
//!
//! Rate limits are declared by overriding [`Provider::resilience`] with a
//! [`ResiliencePolicy`]. The facade books each attempt at its [`Cost`] —
//! [`Cost::keyed`] for a per-key limit such as one chat's — and
//! [`Attempt::report`] passes the provider's "slow down" on as a [`Verdict`].
//! Without the facade a declared rate books one permit per acquire.
//!
//! The closure family — [`ResourceLimiter::wrap`], [`Limited`] and
//! [`LimitedError`] — is deprecated since 0.21.0 and removed before the API
//! freeze: `run` becomes `cx.attempt(Cost::ONE)`, `run_for` becomes
//! [`Cost::keyed`], `run_until` becomes [`Unit::with_deadline`], a
//! [`Throttle`] becomes [`Attempt::report`], and `unlimited` has no
//! replacement by design.
//!
//! A [`StreamOperation`] submitted with [`Managed::submit_streaming`] runs as
//! one unit that also sends items through a bounded [`StreamSink`]; the
//! caller reads them from [`Streaming`], then the unit's error, if any, once.
//!
//! Credentials are declared on the resource struct with
//! `#[derive(Resource)]` and `#[credential(key = "…")]` fields of type
//! [`CredentialSlot<C>`](CredentialSlot) — a [`SlotCell`] holding the
//! credential's projected [`CredentialGuard`]. A unit reads them only
//! through its pinned snapshot ([`Attempt::slots`]).
//!
//! Runtime registration, dispatch, and cleanup queues remain engine-owned.

#[cfg(feature = "resource-http")]
pub mod http;

pub use nebula_core::{ResourceKey, resource_key};
pub use nebula_credential::CredentialGuard;
pub use nebula_resource::call::{
    Attempt, ConsumerGone, Cost, Effect, Managed, OpCx, OpError, Operation, PinSlots, SentState,
    StreamOperation, StreamSink, Streaming, Unit,
};
pub use nebula_resource::rate_limit::{
    DEFAULT_MAX_PENALTY, LimitScope, NoThrottle, OnError, Override, Rate, RateLimitSettings,
    ResiliencePolicy, ResourceLimiter, Throttle, Verdict, on_error, retry_after_from_header,
};
#[expect(
    deprecated,
    reason = "the deprecated closure family stays curated until its removal (MIGRATION P10)"
)]
pub use nebula_resource::rate_limit::{Limited, LimitedError};
pub use nebula_resource::topology::{
    AdmissionPhase, BrokenCheck, CreatedEntry, HookFault, IdleRead, InstanceMetrics, Load,
    MaintenanceSchedule, NoTopology, PoolStrategy, RecycleDecision, ReplaceStatus, RetainStatus,
    RetainedId, RetainedLease, RetainedStore, RetireStatus, StoreRejection, StoreView, Ticket,
    Topology, Unavailable,
};
pub use nebula_resource::{
    Bounded, BoundedMode, BoundedProvider, CheckCost, ClassifyError, CredentialSlot,
    CredentialUnavailableReason, Error, ErrorKind, HasCredentialSlots, LeaseClosing, PoolConfig,
    PoolProvider, Pooled, Provider, ReleaseOutcome, Resident, ResidentConfig, ResidentProvider,
    Resource, ResourceConfig, ResourceContext, ResourceGuard, ResourceMetadataDraft, SlotCell,
    TeardownCx, TeardownReason, TopologyTag, no_credential_slots,
};
