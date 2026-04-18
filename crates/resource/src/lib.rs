//! # nebula-resource
//!
//! **Role:** Bulkhead Pool — engine-owned resource lifecycle (acquire / health /
//! release). Canon §11.4, §13.3. Pattern: Release It! "Bulkhead."
//!
//! The engine is the owner of the resource lifecycle: acquire, health-check,
//! hot-reload via `ReloadOutcome`, and scope-bounded release. Action code
//! receives a `ResourceHandle` that derefs to the lease type and releases on
//! drop. Seven topology traits cover the full integration space.
//!
//! ## Key types
//!
//! | Type | Purpose |
//! |------|---------|
//! | `Resource` | Core trait — 5 associated types, 4 lifecycle methods |
//! | `ResourceHandle` | RAII lease handle with Owned/Guarded/Shared modes |
//! | `Manager` | Central registry with acquire dispatch and shutdown |
//! | `ReleaseQueue` | Background worker pool for async cleanup (best-effort on crash) |
//! | `DrainTimeoutPolicy` | Drain operation timeout policy |
//! | `Cell` | Lock-free `ArcSwap`-based cell for resident topologies |
//! | `Error`, `ErrorKind` | Unified typed error with retry classification |
//! | `Ctx` | Execution context with cancellation and extensions |
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
#[allow(deprecated)]
pub mod compat;
pub mod ctx;
pub mod error;
pub mod events;
pub mod handle;
pub mod integration;
pub mod manager;
pub mod metrics;
pub mod options;
pub mod recovery;
pub mod registry;
pub mod release_queue;
pub mod resource;
pub mod runtime;
pub mod state;
pub mod topology;
pub mod topology_tag;

pub use cell::Cell;
// Backward-compatibility re-exports (deprecated, will be removed).
#[allow(deprecated)]
pub use compat::{Context, Scope};
pub use ctx::{BasicCtx, Ctx, Extensions, ScopeLevel, ctx_ext};
pub use error::{Error, ErrorKind, ErrorScope};
pub use events::ResourceEvent;
pub use handle::ResourceHandle;
pub use integration::{AcquireResilience, AcquireRetryConfig};
pub use manager::{
    DrainTimeoutPolicy, Manager, ManagerConfig, RegisterOptions, ResourceHealthSnapshot,
    ShutdownConfig, ShutdownError, ShutdownReport,
};
pub use metrics::{ResourceOpsMetrics, ResourceOpsSnapshot};
pub use nebula_core::{ExecutionId, ResourceKey, WorkflowId, resource_key};
/// Derive macro that generates `From<T> for nebula_resource::Error`.
///
/// See [`nebula_resource_macros::ClassifyError`] for full documentation.
pub use nebula_resource_macros::ClassifyError;
pub use nebula_resource_macros::Resource;
pub use options::{AcquireIntent, AcquireOptions};
pub use recovery::{
    GateState, RecoveryGate, RecoveryGateConfig, RecoveryGroupKey, RecoveryGroupRegistry,
    RecoveryTicket, RecoveryWaiter, WatchdogConfig, WatchdogHandle,
};
pub use registry::{AnyManagedResource, Registry};
pub use release_queue::ReleaseQueue;
pub use resource::{AnyResource, Resource, ResourceConfig, ResourceMetadata};
// Runtime types — needed for `Manager::register()`.
pub use runtime::TopologyRuntime;
pub use runtime::{
    daemon::DaemonRuntime,
    event_source::EventSourceRuntime,
    exclusive::ExclusiveRuntime,
    managed::ManagedResource,
    pool::{PoolRuntime, PoolStats},
    resident::ResidentRuntime,
    service::ServiceRuntime,
    transport::TransportRuntime,
};
pub use state::{ResourcePhase, ResourceStatus};
// Topology configurations — used at registration time.
pub use topology::daemon::config::Config as DaemonConfig;
pub use topology::{
    daemon::{Daemon, RestartPolicy},
    event_source::{EventSource, config::Config as EventSourceConfig},
    exclusive::{Exclusive, config::Config as ExclusiveConfig},
    pooled::{BrokenCheck, InstanceMetrics, Pooled, RecycleDecision, config::Config as PoolConfig},
    resident::{Resident, config::Config as ResidentConfig},
    service::{Service, TokenMode, config::Config as ServiceConfig},
    transport::{Transport, config::Config as TransportConfig},
};
pub use topology_tag::TopologyTag;
