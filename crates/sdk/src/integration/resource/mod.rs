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
//! Provider calls go through the managed call facade, a [`ResourceHandle`] —
//! the only resource capability an action can name. Each provider call is
//! an [`Operation`] submitted as a [`Submission`]: serializable intent under a
//! [`KEY`](Operation::KEY), which an execution journal records and replays
//! (derive serde with `#[serde(crate = "nebula_sdk::serde")]` when the SDK
//! is the only dependency). Inside it, [`OperationCx::attempt`]
//! admits one provider [`Attempt`], books its [`Cost`] and
//! hands out the instance and the unit's pinned credential slots
//! ([`PinSlots`]). Each attempt is settled with a [`SentState`], and a failed
//! unit's [`OperationError`] says from that and the operation's [`Effect`] whether a
//! retry is safe. On a strict manager every attempt also reads its bound
//! credentials' availability first. A row used this way reports the
//! `PerAttempt` (`per_attempt`) rate-limit profile. The facade is not frozen
//! yet, so it is not in the prelude.
//!
//! Each attempt checks
//! out an instance of its own only after its quota and row-gate waits, so a
//! unit waiting for its rate limit holds no connection. A raw lease over a
//! checked-out instance (`ResourceGuard`, `Lease`) is a host-only capability
//! of the engine and is not exported here: it bypasses the effect journal.
//! On a pooled
//! [`SessionProvider`] it also runs sessions — several native calls on one
//! connection as one unit: [`ResourceHandle::session`] books the
//! [`SessionSpec`]'s cost once (the spec also names the session and
//! carries its request), opens the session, runs the body with it
//! borrowed ([`SessionFuture`], [`SessionCx`]) and closes it with a commit
//! or a rollback ([`SessionEnd`], [`SessionClosed`]). A
//! [`SessionBinding::Connection`] provider only runs on a connection built
//! with the credentials the unit pinned. An action obtains a `ResourceHandle`
//! through a derived field — `#[resource] db: ResourceHandle<Db>` (or
//! `Option<ResourceHandle<Db>>`) on a `#[derive(Action)]` struct — never by
//! building one here: its units are cancelled with the execution until their
//! first grant and bounded by the execution's deadline.
//!
//! Rate limits are declared by overriding [`Provider::resilience`] with a
//! [`ResiliencePolicy`]. The facade books each attempt at its [`Cost`] —
//! [`Cost::keyed`] for a per-key limit such as one chat's — and a call made
//! through [`OperationCx::call`] that returns [`OperationError::throttled`]
//! (or [`OperationError::throttled_key`]) passes the provider's "slow down"
//! on. Without the facade a declared rate books one permit per acquire.
//!
//! A [`StreamOperation`] submitted with
//! [`ResourceHandle::submit_streaming`] runs as one unit that also sends items through a bounded [`StreamSink`]; the
//! caller reads them from [`Streaming`], then the unit's error, if any, once.
//!
//! Credentials are declared on the resource struct with
//! `#[derive(Resource)]` and `#[credential(key = "…")]` fields of type
//! [`CredentialSlot<C>`](CredentialSlot) — a [`SlotCell`] holding the
//! credential's projected [`CredentialGuard`]. A unit reads them only
//! through its pinned snapshot ([`Attempt::credentials`]).
//!
//! Runtime registration, dispatch, and cleanup queues remain engine-owned.

#[cfg(feature = "resource-http")]
pub mod http;

pub use nebula_core::{ResourceKey, resource_key};
pub use nebula_credential::CredentialGuard;
pub use nebula_resource::call::{
    Attempt, ConsumerGone, Cost, Effect, Operation, OperationCx, OperationError, PinSlots,
    ResourceHandle, SentState, SessionBinding, SessionClosed, SessionCx, SessionEnd, SessionFuture,
    SessionProvider, SessionSpec, StreamOperation, StreamSink, Streaming, Submission,
};
pub use nebula_resource::rate_limit::{
    DEFAULT_MAX_PENALTY, LimitScope, Override, Rate, RateLimitSettings, ResiliencePolicy,
    ResourceLimiter, retry_after_from_header,
};
pub use nebula_resource::topology::{
    AdmissionPhase, BrokenCheck, CreatedEntry, HookFault, IdleRead, InstanceMetrics, Load,
    MaintenanceSchedule, NoTopology, PoolStrategy, RecycleDecision, ReplaceStatus, RetainStatus,
    RetainedId, RetainedLease, RetainedStore, RetireStatus, StoreRejection, StoreView, Ticket,
    Topology, Unavailable,
};
pub use nebula_resource::{
    Bounded, BoundedMode, BoundedProvider, CheckCost, ClassifyError, CredentialSlot,
    CredentialUnavailableReason, Error, ErrorKind, HasCredentialSlots, LeaseClosing, PoolConfig,
    PoolProvider, Pooled, Provider, Resident, ResidentConfig, ResidentProvider, Resource,
    ResourceConfig, ResourceContext, ResourceMetadataDraft, SlotCell, TeardownCx, TeardownReason,
    TopologyTag, no_credential_slots,
};
