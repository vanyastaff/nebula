//! # nebula-resource
//!
//! **Role:** Bulkhead Pool — engine-owned resource lifecycle (acquire / health /
//! release). Bulkhead isolation (integration seam step 3). Pattern: Release It! "Bulkhead."
//!
//! The engine is the owner of the resource lifecycle: acquire, health-check,
//! hot-reload via `ReloadOutcome`, and scope-bounded release. Action code
//! receives a `ResourceGuard` that derefs to the lease type and releases on
//! drop. Two topology traits cover the integration space: `Pooled` and
//! `Resident`.
//!
//! ## Key types
//!
//! | Type | Purpose |
//! |------|---------|
//! | `Resource` | Core trait — 4 associated types + lifecycle + slot-rotation hooks |
//! | `ResourceGuard` | RAII lease guard with Owned/Guarded/Shared modes |
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
pub mod dedup;
pub mod error;
pub mod events;
pub mod ext;
pub mod guard;
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
    DrainTimeoutPolicy, ErasedAcquireFn, Manager, ManagerConfig, RegisterOptions, RegistrationSpec,
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
pub use nebula_resource_macros::Resource;
// Schema surface — re-exported so adapter crates don't need a direct
// nebula-schema dep just to satisfy `ResourceConfig`'s `HasSchema`
// super-bound. `Schema` covers both the type and the derive macro
// (separate namespaces sharing the name); `impl_empty_has_schema!` uses
// `$crate::*` paths, so its expansion does not require adapters to keep
// `nebula-schema` in extern_prelude either.
pub use nebula_schema::{HasSchema, Schema, ValidSchema, impl_empty_has_schema};
pub use options::AcquireOptions;
pub use recovery::{GateState, RecoveryGate, RecoveryGateConfig, RecoveryTicket, RecoveryWaiter};
pub use registry::{AnyManagedResource, LookupOutcome, Registry};
pub use release_queue::ReleaseQueue;
pub use reload::ReloadOutcome;
pub use resource::{
    AnyResource, HasCredentialSlots, MetadataCompatibilityError, Resource, ResourceConfig,
    ResourceMetadata,
};
pub use resource_ref::ResourceRef;
pub use slot::{CredentialSlot, SlotCell};
// Runtime types — needed for `Manager::register()`.
pub use runtime::TopologyRuntime;
pub use runtime::{
    managed::ManagedResource,
    pool::{PoolRuntime, PoolStats},
    resident::ResidentRuntime,
};
pub use state::{ResourcePhase, ResourceStatus};
// Topology configurations — used at registration time.
pub use topology::{
    pooled::{BrokenCheck, InstanceMetrics, Pooled, RecycleDecision, config::Config as PoolConfig},
    resident::{Resident, config::Config as ResidentConfig},
};
pub use topology_tag::TopologyTag;
