//! # nebula-resource
//!
//! **Role:** Bulkhead Pool — engine-owned resource lifecycle (acquire / health /
//! release). Canon §11.4, §13.3. Pattern: Release It! "Bulkhead."
//!
//! The engine is the owner of the resource lifecycle: acquire, health-check,
//! hot-reload via `ReloadOutcome`, and scope-bounded release. Action code
//! receives a `ResourceGuard` that derefs to the lease type and releases on
//! drop. Five topology traits cover the full integration space.
//!
//! ## Key types
//!
//! | Type | Purpose |
//! |------|---------|
//! | `Resource` | Core trait — 5 associated types, 5 core methods |
//! | `ResourceGuard` | RAII lease guard with Owned/Guarded/Shared modes |
//! | `Manager` | Central registry with acquire dispatch and shutdown |
//! | `ReleaseQueue` | Background worker pool for async cleanup (best-effort on crash) |
//! | `DrainTimeoutPolicy` | Drain operation timeout policy |
//! | `Cell` | Lock-free `ArcSwap`-based cell for resident topologies |
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

pub mod cell;
pub mod context;
pub mod error;
pub mod events;
pub mod ext;
pub mod guard;
pub mod integration;
pub mod manager;
pub mod metrics;
pub mod options;
pub mod recovery;
pub mod registry;
pub mod release_queue;
pub mod reload;
pub mod resource;
pub mod runtime;
pub mod state;
pub mod topology;
pub mod topology_tag;

pub use cell::Cell;
pub use context::ResourceContext;
pub use error::{Error, ErrorKind, ErrorScope, RefreshOutcome, RevokeOutcome, RotationOutcome};
pub use events::ResourceEvent;
pub use ext::HasResourcesExt;
pub use guard::ResourceGuard;
pub use integration::{AcquireResilience, AcquireRetryConfig};
pub use manager::{
    DrainTimeoutPolicy, Manager, ManagerConfig, RegisterOptions, ResourceHealthSnapshot,
    ShutdownConfig, ShutdownError, ShutdownReport,
};
pub use metrics::{OutcomeCountersSnapshot, ResourceOpsMetrics, ResourceOpsSnapshot};
pub use nebula_core::{ExecutionId, ResourceKey, ScopeLevel, WorkflowId, resource_key};
// Credential adoption surface per ADR-0036 — re-exported so resource
// consumers don't need a direct nebula-credential dep for trait shape.
pub use nebula_credential::{
    Credential, CredentialContext, CredentialId, NoCredential, NoCredentialState, SchemeGuard,
};
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
pub use recovery::{
    GateState, RecoveryGate, RecoveryGateConfig, RecoveryGroupKey, RecoveryGroupRegistry,
    RecoveryTicket, RecoveryWaiter, WatchdogConfig, WatchdogHandle,
};
pub use registry::{AnyManagedResource, Registry};
pub use release_queue::ReleaseQueue;
pub use reload::ReloadOutcome;
pub use resource::{
    AnyResource, MetadataCompatibilityError, Resource, ResourceConfig, ResourceMetadata,
};
// Runtime types — needed for `Manager::register()`.
pub use runtime::TopologyRuntime;
pub use runtime::{
    exclusive::ExclusiveRuntime,
    managed::ManagedResource,
    pool::{PoolRuntime, PoolStats},
    resident::ResidentRuntime,
    service::ServiceRuntime,
    transport::TransportRuntime,
};
pub use state::{ResourcePhase, ResourceStatus};
// Topology configurations — used at registration time.
pub use topology::{
    exclusive::{Exclusive, config::Config as ExclusiveConfig},
    pooled::{BrokenCheck, InstanceMetrics, Pooled, RecycleDecision, config::Config as PoolConfig},
    resident::{Resident, config::Config as ResidentConfig},
    service::{Service, TokenMode, config::Config as ServiceConfig},
    transport::{Transport, config::Config as TransportConfig},
};
pub use topology_tag::TopologyTag;
