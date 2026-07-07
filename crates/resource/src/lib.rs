//! # nebula-resource
//!
//! **Role:** Bulkhead Pool — engine-owned resource lifecycle (acquire / health /
//! release). Bulkhead isolation (integration seam step 3). Pattern: Release It! "Bulkhead."
//!
//! The engine is the owner of the resource lifecycle: acquire, health-check,
//! hot-reload via `ReloadOutcome`, and scope-bounded release. Action code
//! receives a `ResourceGuard` that derefs to `R::Instance` and releases on
//! drop. Three built-in topologies cover the integration space: `Pooled`
//! (N interchangeable instances), `Resident` (one shared instance, cloned
//! per acquire), and `Bounded` (a concurrency cap with no warm idle pool).
//!
//! ## Quick start
//!
//! The 90% path: `#[derive(Resource)]` emits slot plumbing (empty here — no
//! `#[credential]` field), a hand-written [`Provider`] impl supplies the
//! lifecycle, [`Manager::register`] files it under `R::key()`, and
//! `acquire_<topology>` hands back a [`ResourceGuard`] that derefs to
//! `R::Instance` and releases (recycle or destroy, per topology) on drop:
//!
//! ```
//! use async_trait::async_trait;
//! use nebula_core::{ResourceKey, ScopeLevel, resource_key};
//! use nebula_resource::{
//!     AcquireOptions, Error, Manager, PoolConfig, PoolProvider, Pooled, Provider,
//!     RegistrationSpec, Resource, ResourceContext, SlotIdentity,
//! };
//!
//! #[derive(Resource, Clone)]
//! struct HttpClient;
//!
//! #[async_trait]
//! impl Provider for HttpClient {
//!     type Config = ();
//!     type Instance = ();
//!     type Topology = Pooled<Self>;
//!
//!     fn key() -> ResourceKey {
//!         resource_key!("quickstart.http_client")
//!     }
//!
//!     async fn create(&self, _config: &(), _ctx: &ResourceContext) -> Result<(), Error> {
//!         Ok(())
//!     }
//! }
//!
//! // Every hook has a default — an empty impl opts `HttpClient` into pool
//! // topology as-is.
//! impl PoolProvider for HttpClient {}
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), Error> {
//! let manager = Manager::new();
//! manager.register(RegistrationSpec {
//!     resource: HttpClient,
//!     config: (),
//!     scope: ScopeLevel::Global,
//!     slot_identity: SlotIdentity::Unbound,
//!     topology: Pooled::<HttpClient>::new(PoolConfig::default(), 0),
//!     recovery_gate: None,
//! })?;
//!
//! let ctx = ResourceContext::minimal(
//!     nebula_core::scope::Scope::default(),
//!     tokio_util::sync::CancellationToken::new(),
//! );
//! let guard = manager
//!     .acquire_pooled::<HttpClient>(&ctx, &AcquireOptions::default())
//!     .await?;
//! let _instance: &() = &*guard; // guard derefs to `R::Instance`
//! drop(guard); // release: recycled back into the pool, not destroyed
//! # Ok(())
//! # }
//! ```
//!
//! See [`Manager::register`] / [`Manager::acquire_pooled`] for the full
//! error and cancel-safety contract. Action code running inside the engine
//! does not usually call [`Manager`] directly — see the `ext` module for the
//! `ctx.resource::<R>().await?` access surface instead.
//!
//! ## Choosing a topology
//!
//! `type Topology` is static per resource type (a Postgres resource is
//! always [`Pooled`]); only its config is a runtime value.
//!
//! | Topology | Instance model | Use when |
//! |----------|-----------------|----------|
//! | [`Pooled`] | N interchangeable instances, checkout/recycle | Stateful, interchangeable connections (DB, gRPC channel) |
//! | [`Resident`] | One shared instance, `Arc::clone` on acquire | A cheap-to-clone client shared widely (`reqwest::Client`, in-memory cache) |
//! | [`Bounded`] | Concurrency-capped, no warm idle pool | Scarce non-warmable capacity (license seats, a serial-exclusive device) |
//!
//! See `crates/resource/docs/topology-reference.md` for a per-topology trait
//! skeleton and the friction points of each, and the "Tuning" section below
//! for the config knobs.
//!
//! ## Error taxonomy
//!
//! Resource errors are typed and self-classifying — the caller reads the
//! [`ErrorKind`] to decide whether to retry, back off, or give up (see the
//! `error` module docs for the full kind → caller-action table). Authors
//! bridge a domain error enum in with `#[derive(ClassifyError)]` rather than
//! hand-writing a `From` impl:
//!
//! ```
//! use nebula_resource::{ClassifyError, Error, ErrorKind};
//!
//! #[derive(Debug, thiserror::Error, ClassifyError)]
//! enum PostgresError {
//!     #[error("connection failed: {0}")]
//!     #[classify(transient)]
//!     Connect(String),
//!     #[error("authentication failed: {0}")]
//!     #[classify(permanent)]
//!     Auth(String),
//!     #[error("rate limited")]
//!     #[classify(exhausted, retry_after = "30s")]
//!     RateLimited,
//! }
//!
//! let err: Error = PostgresError::Connect("timed out".into()).into();
//! assert!(err.is_retryable());
//! assert!(matches!(err.kind(), ErrorKind::Transient));
//! ```
//!
//! ## Key types
//!
//! | Type | Purpose |
//! |------|---------|
//! | `Provider` | Lifecycle trait — `Config`/`Instance` + lifecycle + slot-rotation hooks |
//! | `Resource` | Derive macro — emits slot plumbing (`HasCredentialSlots`, accessors) |
//! | `ResourceGuard` | RAII instance guard with Owned/Guarded modes |
//! | `Manager` | Central registry with acquire dispatch and shutdown |
//! | `ReleaseQueue` | Background worker pool for async cleanup (best-effort on crash) |
//! | `DrainTimeoutPolicy` | Drain operation timeout policy |
//! | `SlotCell` | Lock-free generation-stamped holder for a resolved credential slot |
//! | `Error`, `ErrorKind` | Unified typed error with retry classification |
//! | `ResourceContext` | Execution context with cancellation and capabilities |
//! | `ResourceEvent` | Lifecycle events for observability |
//! | `ResourceOpsMetrics` | Registry-backed operation counters |
//!
//! ## Cancel safety
//!
//! Public async methods document their cancellation contract in a
//! `# Cancel safety` section (the tokio convention, per [`tokio`'s own
//! guidance](https://docs.rs/tokio/latest/tokio/macro.select.html#cancellation-safety)).
//! See "Guarantees" below for the two load-bearing cross-cutting ones.
//!
//! ## Guarantees
//!
//! Tokio-grade libraries state their cross-cutting invariants once, in one
//! place, each backed by a named test — not scattered as prose across every
//! call site. These hold for every acquire path and every topology:
//!
//! 1. **A dropped acquire future never leaks an instance.** An instance in
//!    flight between checkout/create and the returned guard is destroyed
//!    asynchronously via the [`ReleaseQueue`] on cancellation, never orphaned.
//!    Tested by `cancelled_acquire_during_accept_destroys_the_popped_entry`
//!    and `cancelled_warmup_between_create_and_deposit_destroys_the_entry`
//!    in `src/runtime/acquire_loop.rs`.
//! 2. **After a credential is revoked, no new lease is ever handed out on
//!    it** — the taint runs synchronously before the first `.await`, so a
//!    dropped or timed-out revoke future can never leave the credential
//!    silently servable. Tested by `tests/revoke_recycle_toctou.rs` and
//!    `probe_revoke_mid_probe_destroys_probed_entries_not_redeposited` in
//!    `src/runtime/acquire_loop.rs`. See
//!    `crates/resource/docs/credential-rotation.md` for the full sequence.
//! 3. **Exactly one [`Provider::create`] runs per `(key, scope,
//!    slot_identity)` under concurrent acquire** — every other concurrent
//!    caller receives a lease pointing at the same backing instance rather
//!    than triggering a redundant create. Tested by
//!    `shared_resource::cross_workflow_resource_sharing` in
//!    `crates/engine/tests/resource_integration.rs`.
//! 4. **Pool exhaustion is a typed [`ErrorKind::Backpressure`], never a
//!    generic timeout** — a full pool tells the caller *why* it failed to
//!    acquire, not just that it took too long. Tested by
//!    `pool_backpressure_when_full` in `tests/scope_and_concurrency.rs`.
//! 5. **Shutdown drains in-flight leases or reports the outstanding count
//!    within [`DrainTimeoutPolicy`]** — a stuck drain surfaces as a typed
//!    outcome, never a silent hang. Tested by
//!    `graceful_shutdown_abort_on_drain_timeout_preserves_registry` and
//!    `graceful_shutdown_force_clears_registry_on_timeout` in
//!    `tests/recovery_and_shutdown.rs`.
//!
//! ## Feature flags
//!
//! - `rotation` — enables the credential-rotation fan-out module
//!   (`credential_fanout`): the `CredentialId` → resource-rows reverse
//!   index, the rotation orchestrator, and
//!   `ResourceActivatorRegistry::register_and_bind`. Off by default so the
//!   base build pays no eventbus-subscriber or extra-task overhead; the
//!   engine enables it together with its own `rotation` feature. See
//!   `crates/resource/docs/credential-rotation.md` for the full
//!   rotate → slot-swap → refresh/revoke-hook sequence.
//!
//! ## Tuning
//!
//! | Knob | Default | Rationale |
//! |------|---------|-----------|
//! | [`ManagerConfig::acquire_slow_threshold`] | `None` (WARN disabled) | Per-manager default for the slow-acquire WARN log (sqlx `slow_acquire` precedent); override per call via [`AcquireOptions::acquire_slow_threshold`]. |
//! | [`Provider::max_hold_duration`] | `None` (watchdog disabled) | HikariCP `leakDetectionThreshold` lineage: a lease held past this logs `ResourceEvent::HoldDeadlineExceeded` (with the acquiring execution/workflow/span ids) and bumps a `hold_deadline_exceeded` counter — diagnostic only, never force-releases. |
//! | `PoolConfig::idle_timeout` | `Some(5 min)` | Evicts idle instances so a burst-sized pool shrinks back down between bursts. |
//! | `PoolConfig::max_lifetime` | `Some(30 min)` | Forces periodic reconnect (picks up DNS/LB changes, bounds worst-case staleness); evicted with a small per-entry jitter band (`[0.95×, 1.0×]`, drawn once at creation) so a warmup cohort does not all expire on the same maintenance tick. |
//! | `PoolConfig::min_size` (the pool's `warmup_target`) | `1` | The reaper's min-idle floor: after a maintenance sweep evicts idle entries below this, `refill_min_idle` tops the store back up (HikariCP `minimumIdle` lineage) — gated through the resource's `RecoveryGate` (if attached) so a flapping backend is not hammered by refill attempts. |
//! | [`RecoveryGateConfig::max_attempts`] | `5` | Caps thundering-herd probe retries before requiring a manual `RecoveryGate::reset`. |
//! | [`RecoveryGateConfig::base_backoff`] | `1 s` | Doubled per attempt (capped at 5 min), then equal-jittered (`[nominal/2, nominal]`) so a synchronized-failure cohort does not retry in lockstep. |
//! | `ShutdownConfig::drain_timeout` | `30 s` | Bounds how long [`Manager::graceful_shutdown`] waits for in-flight handles before applying [`DrainTimeoutPolicy`]. |
//!
//! Full field references: [`pooling.md`](../docs/pooling.md) for the pool
//! config, [`recovery.md`](../docs/recovery.md) for the gate, and the
//! rustdoc on each type above for the rest.
//!
//! ## Canon note — §11.4
//!
//! Async release is best-effort on crash. Orphaned resources rely on the next
//! process to drain via `ReleaseQueue`. Authors must not assume "release ran"
//! without an explicit checkpoint.
//!
//! See `crates/resource/README.md` for the full contract, topology reference,
//! and drain mechanism details.

#![deny(missing_docs)]
#![warn(unreachable_pub)]
#![warn(missing_debug_implementations)]
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::missing_errors_doc)]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub(crate) mod cell;
pub mod context;
#[cfg(feature = "rotation")]
pub mod credential_fanout;
pub mod dedup;
pub mod error;
pub mod events;
pub mod ext;
pub mod factory;
pub mod guard;
pub(crate) mod hook_guard;
pub(crate) mod jitter;
pub mod manager;
pub mod metrics;
pub mod options;
pub mod recovery;
pub mod registry;
pub mod release_queue;
pub mod reload;
pub mod resource;
pub mod resource_ref;
pub(crate) mod runtime;
pub mod slot;
pub mod state;
pub mod topology;
pub mod topology_tag;

// NOTE: `cell::Cell` is intentionally NOT re-exported. It is an internal
// lock-free `ArcSwapOption` holder for the resident runtime; it carries no
// generation/epoch and is a strict subset of the public `SlotCell`. The
// `cell` module is crate-internal (`pub(crate) mod`) so consumers reach for
// the generation-bearing `SlotCell` and are not misled into using the
// epoch-blind cell at a credential-slot boundary.
pub use context::{
    ResourceContext, minimal_scope_for_level, scope_levels_for_acquire, scope_to_level,
};
pub use dedup::{DedupKey, SlotIdentity};
pub use error::{Error, ErrorKind};
pub use events::ResourceEvent;
pub use ext::HasResourcesExt;
pub use guard::ResourceGuard;
pub use manager::{
    DrainTimeoutPolicy, Manager, ManagerConfig, RegisterOptions, RegistrationSpec,
    ResourceHealthSnapshot, RevokeTail, ShutdownConfig, ShutdownError, ShutdownReport, TaintedSlot,
};
pub use metrics::{
    ACQUIRE_WAIT_BUCKET_UPPER_BOUNDS_MICROS, AcquireWaitSnapshot, OutcomeCountersSnapshot,
    ResourceOpsMetrics, ResourceOpsSnapshot,
};
pub use nebula_core::{ExecutionId, ResourceKey, ScopeLevel, WorkflowId, resource_key};
/// Re-export [`Subscriber`] so callers of [`Manager::subscribe_events`] do not
/// need a direct `nebula-eventbus` dependency.
pub use nebula_eventbus::Subscriber;
// Credential surface re-exported so resource consumers don't need a
// direct nebula-credential dep for trait shape.
//
// Per slot model the singular `Resource::Credential` associated type and
// its `NoCredential` opt-out type are gone — credentials are declared
// via `#[credential(key = ...)]` slot fields on the resource struct.
// `NoCredential`/`NoCredentialState` are no longer re-exported.
pub use nebula_credential::{Credential, CredentialContext, CredentialId};
/// Derive macro that generates `From<T> for nebula_resource::Error`.
///
/// See [`nebula_resource_macros::ClassifyError`] for the full attribute
/// reference (supported kinds, `retry_after` forms).
///
/// ```
/// use nebula_resource::{ClassifyError, Error};
///
/// #[derive(Debug, thiserror::Error, ClassifyError)]
/// enum DbError {
///     #[error("connection lost: {0}")]
///     #[classify(transient)]
///     ConnectionLost(String),
///     #[error("rate limited")]
///     #[classify(exhausted, retry_after = "30s")]
///     RateLimited,
/// }
///
/// let err: Error = DbError::ConnectionLost("timeout".into()).into();
/// assert!(err.is_retryable());
///
/// let err: Error = DbError::RateLimited.into();
/// assert_eq!(err.retry_after(), Some(std::time::Duration::from_secs(30)));
/// ```
///
/// For the full derive→register→acquire flow, see the doctest on
/// [`Manager::register`].
pub use nebula_resource_macros::ClassifyError;
/// Derive macro that emits slot plumbing for a resource struct.
///
/// Generates `impl DeclaresDependencies`, slot accessor methods, and
/// `impl HasCredentialSlots`. Used together with a hand-written `impl Provider`.
///
/// See [`nebula_resource_macros::Resource`] for the full field-attribute
/// reference. A slot-less struct is legal and emits the honest zero
/// [`HasCredentialSlots`] impl (same shape as
/// [`no_credential_slots!`](crate::no_credential_slots)):
///
/// ```
/// use nebula_resource::{HasCredentialSlots, Resource};
///
/// #[derive(Resource)]
/// struct HttpClient;
///
/// assert!(!HttpClient::declares_credential_slots());
/// assert_eq!(HttpClient.credential_slot_epoch(), 0);
/// ```
///
/// For the full derive→register→acquire flow (including a `#[credential]`
/// slot field), see the doctest on [`Manager::register`].
pub use nebula_resource_macros::Resource;
/// Derive macro that generates `impl ResourceConfig` with a structural fingerprint
/// and an optional default empty `impl HasSchema`.
///
/// See [`nebula_resource_macros::ResourceConfig`] for the full container-
/// and field-attribute reference (`#[config(validate = path)]`,
/// `#[config(skip_fingerprint)]`).
///
/// ```
/// use nebula_resource::ResourceConfig;
///
/// #[derive(ResourceConfig, Clone)]
/// struct PgConfig {
///     url: String,
///     max_conns: u32,
/// }
///
/// let cfg = PgConfig { url: "postgres://db".to_owned(), max_conns: 8 };
/// let resized = PgConfig { max_conns: 16, ..cfg.clone() };
/// // Fingerprint changes whenever an operationally-significant field changes.
/// assert_ne!(cfg.fingerprint(), resized.fingerprint());
/// ```
pub use nebula_resource_macros::ResourceConfig;
// Schema surface — re-exported so adapter crates don't need a direct
// nebula-schema dep just to satisfy `ResourceConfig`'s `HasSchema`
// super-bound. `Schema` covers both the type and the derive macro
// (separate namespaces sharing the name); `impl_empty_has_schema!` uses
// `$crate::*` paths, so its expansion does not require adapters to keep
// `nebula-schema` in extern_prelude either.
pub use factory::{
    BoxFut, KindActivator, RegisterRequest, RegistrarError, ResourceActivatorRegistry,
    ResourceFactory, ResourceRegistrationOutcome, SlotBinding,
};
pub use nebula_schema::{HasSchema, Schema, ValidSchema, impl_empty_has_schema};
pub use options::AcquireOptions;
pub use recovery::{
    GateState, RecoveryGate, RecoveryGateConfig, RecoveryTicket, RecoveryWaiter, TryBeginError,
};
pub use registry::{LookupOutcome, ManagedHandle, Registry};
pub use release_queue::ReleaseQueue;
pub use reload::ReloadOutcome;
pub use resource::{
    CheckCost, HasCredentialSlots, MetadataCompatibilityError, Provider, ResourceConfig,
    ResourceMetadata, TeardownCx, TeardownReason,
};
pub use resource_ref::ResourceRef;
pub use slot::{CredentialSlot, SlotCell};
// Runtime types — the framework topologies needed for `Manager::register()`.
pub use runtime::managed::ManagedResource;
pub use runtime::{
    bounded::Bounded,
    pool::{PoolStats, Pooled},
    resident::Resident,
};
pub use state::{ResourceErrorSummary, ResourcePhase, ResourceStatus};
// Topology configurations — used at registration time.
pub use topology::{
    AdmissionPhase, AdmissionStatus, CheckedOut, Checkout, InstanceStore, Load,
    MaintenanceSchedule, NoTopology, PoolStrategy, ReturnOutcome, Ticket, Topology, Unavailable,
    bounded::{BoundedMode, BoundedProvider},
    pooled::{
        BrokenCheck, InstanceMetrics, PoolProvider, RecycleDecision, config::Config as PoolConfig,
    },
    resident::{ResidentProvider, config::Config as ResidentConfig},
};
pub use topology_tag::TopologyTag;
// Credential-rotation fan-out — gated on the `rotation` feature so the
// default build of `nebula-resource` does not pay for the eventbus subscriber
// overhead or pull in the extra tokio task. Engine enables this feature when
// it enables its own `rotation` feature.
#[cfg(feature = "rotation")]
pub use credential_fanout::{Bind, ResourceFanoutDriver, ResourceFanoutIndex, RotationOutcome};

/// Prelude — common types for resource authors and engine integrators.
///
/// ```no_run
/// use nebula_resource::prelude::*;
/// use nebula_resource::topology::pooled::PoolProvider;
///
/// #[derive(Clone)]
/// struct MyResource;
///
/// #[async_trait::async_trait]
/// impl Provider for MyResource {
///     type Config = ();
///     type Instance = ();
///     type Topology = Pooled<Self>;
///     fn key() -> ResourceKey { resource_key!("my.resource") }
///     async fn create(&self, _: &(), _: &ResourceContext) -> Result<(), Error> {
///         Ok(())
///     }
/// }
///
/// nebula_resource::no_credential_slots!(MyResource);
///
/// // `Pooled<Self>` requires `PoolProvider`; every method has a default,
/// // so an empty impl opts the resource into pool topology.
/// impl PoolProvider for MyResource {}
/// ```
pub mod prelude {
    pub use crate::{
        AcquireOptions, Error, ErrorKind, HasCredentialSlots, Manager, PoolConfig, Pooled,
        Provider, RegistrationSpec, Resident, ResidentConfig, ResourceConfig, ResourceContext,
        ResourceGuard, ResourceKey, ResourceMetadata, ScopeLevel, ShutdownConfig, SlotCell,
        SlotIdentity, TopologyTag, resource_key,
    };
}
