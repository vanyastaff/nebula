//! # nebula-resource
//!
//! **Role:** Bulkhead Pool — engine-owned resource lifecycle (acquire / health /
//! release). Bulkhead isolation (integration seam step 3). Pattern: Release It! "Bulkhead."
//!
//! The engine is the owner of the resource lifecycle: acquire, health-check,
//! hot-reload via `ReloadOutcome`, and scope-bounded release. Action code
//! receives a `ResourceGuard` that derefs to `R::Instance` and releases on
//! drop. Three built-in topologies cover the integration space: `Pooled`,
//! `Resident`.
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
//! ## Canon note — §11.4
//!
//! Async release is best-effort on crash. Orphaned resources rely on the next
//! process to drain via `ReleaseQueue`. Authors must not assume "release ran"
//! without an explicit checkpoint.
//!
//! See `crates/resource/README.md` for the full contract, topology reference,
//! and drain mechanism details.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub(crate) mod cell;
pub mod context;
#[cfg(feature = "rotation")]
pub mod credential_fanout;
pub mod dedup;
pub mod error;
pub mod events;
pub mod ext;
pub mod guard;
pub(crate) mod hook_guard;
pub mod manager;
pub mod metrics;
pub mod options;
pub mod recovery;
pub mod registry;
pub mod release_queue;
pub mod reload;
pub mod resource;
pub mod resource_ref;
pub mod runtime;
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
pub use error::{Error, ErrorKind, ErrorScope};
pub use events::ResourceEvent;
pub use ext::HasResourcesExt;
pub use guard::ResourceGuard;
pub use manager::{
    DrainTimeoutPolicy, Manager, ManagerConfig, RegisterOptions, RegistrationSpec,
    ResourceHealthSnapshot, RevokeTail, ShutdownConfig, ShutdownError, ShutdownReport, TaintedSlot,
};
pub use metrics::{OutcomeCountersSnapshot, ResourceOpsMetrics, ResourceOpsSnapshot};
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
/// See [`nebula_resource_macros::ClassifyError`] for full documentation.
pub use nebula_resource_macros::ClassifyError;
/// Derive macro that emits slot plumbing for a resource struct.
///
/// Generates `impl DeclaresDependencies`, slot accessor methods, and
/// `impl HasCredentialSlots`. Used together with a hand-written `impl Provider`.
///
/// See [`nebula_resource_macros::Resource`] for full documentation.
pub use nebula_resource_macros::Resource;
/// Derive macro that generates `impl ResourceConfig` with a structural fingerprint
/// and an optional default empty `impl HasSchema`.
///
/// See [`nebula_resource_macros::ResourceConfig`] for full documentation.
pub use nebula_resource_macros::ResourceConfig;
// Schema surface — re-exported so adapter crates don't need a direct
// nebula-schema dep just to satisfy `ResourceConfig`'s `HasSchema`
// super-bound. `Schema` covers both the type and the derive macro
// (separate namespaces sharing the name); `impl_empty_has_schema!` uses
// `$crate::*` paths, so its expansion does not require adapters to keep
// `nebula-schema` in extern_prelude either.
pub use nebula_schema::{HasSchema, Schema, ValidSchema, impl_empty_has_schema};
pub use options::AcquireOptions;
pub use recovery::{GateState, RecoveryGate, RecoveryGateConfig, RecoveryTicket, RecoveryWaiter};
pub use registry::{LookupOutcome, ManagedHandle, Registry};
pub use release_queue::ReleaseQueue;
pub use reload::ReloadOutcome;
pub use resource::{
    CheckCost, HasCredentialSlots, MetadataCompatibilityError, Provider, ResourceConfig,
    ResourceDescriptor, ResourceMetadata, TeardownCx, TeardownReason,
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
pub use state::{ResourcePhase, ResourceStatus};
// Topology configurations — used at registration time.
pub use topology::{
    AdmissionPhase, AdmissionStatus, CheckedOut, Checkout, InstanceStore, Load,
    MaintenanceSchedule, NoTopology, ReturnOutcome, Ticket, Topology, Unavailable,
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
/// impl HasCredentialSlots for MyResource {
///     fn credential_slot_epoch(&self) -> u64 { 0 }
/// }
///
/// // A `Pooled` topology requires `PoolProvider`; every method is defaulted,
/// // so a pool-backed resource with default recycle/prepare needs only this.
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
