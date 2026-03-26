//! # nebula-resource
//!
//! Type-safe, topology-agnostic resource management for the Nebula workflow
//! engine.
//!
//! This crate provides the foundational primitives for managing external
//! resources (databases, HTTP clients, message brokers, etc.) with a unified
//! lifecycle: create, health-check, shutdown, destroy.
//!
//! ## Key types
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`Resource`] | Core trait — 5 associated types, 4 lifecycle methods |
//! | [`ResourceHandle`] | RAII lease handle with Owned/Guarded/Shared modes |
//! | [`Cell`] | Lock-free `ArcSwap`-based cell for resident topologies |
//! | [`ReleaseQueue`] | Background worker pool for async cleanup |
//! | [`Error`] | Unified error with [`ErrorKind`] + [`ErrorScope`] |
//! | [`Ctx`] | Execution context with cancellation and extensions |
//! | [`Manager`] | Central registry with acquire dispatch and shutdown |
//! | [`Registry`] | Type-erased storage for managed resources |
//! | [`ResourceEvent`] | Lifecycle events for observability |
//! | [`ResourceMetrics`](metrics::ResourceMetrics) | Atomic operation counters |

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
pub use ctx::{BasicCtx, Ctx, Extensions, ScopeLevel, ctx_ext};
pub use error::{Error, ErrorKind, ErrorScope};
pub use events::ResourceEvent;
pub use handle::ResourceHandle;
pub use manager::{Manager, ManagerConfig, ShutdownConfig};
pub use metrics::{MetricsSnapshot, ResourceMetrics};
pub use options::{AcquireIntent, AcquireOptions};
pub use registry::{AnyManagedResource, Registry};
pub use release_queue::ReleaseQueue;
pub use resource::{AnyResource, Credential, Resource, ResourceConfig, ResourceMetadata};
pub use state::{ResourcePhase, ResourceStatus};
pub use topology::daemon::{Daemon, RestartPolicy};
pub use topology::event_source::EventSource;
pub use topology::exclusive::Exclusive;
pub use topology::pooled::{BrokenCheck, InstanceMetrics, Pooled, RecycleDecision};
pub use topology::resident::Resident;
pub use topology::service::{Service, TokenMode};
pub use topology::transport::Transport;
pub use topology_tag::TopologyTag;

pub use integration::{AcquireCircuitBreakerPreset, AcquireResilience, AcquireRetryConfig};
pub use recovery::{
    GateState, RecoveryGate, RecoveryGateConfig, RecoveryGroupKey, RecoveryGroupRegistry,
    RecoveryTicket, RecoveryWaiter, WatchdogConfig, WatchdogHandle,
};

pub use nebula_core::{ExecutionId, ResourceKey, WorkflowId, resource_key};

/// Derive macro that generates `From<T> for nebula_resource::Error`.
///
/// See [`nebula_resource_macros::ClassifyError`] for full documentation.
pub use nebula_resource_macros::ClassifyError;

// Runtime types — needed for `Manager::register()`.
pub use runtime::TopologyRuntime;
pub use runtime::daemon::DaemonRuntime;
pub use runtime::event_source::EventSourceRuntime;
pub use runtime::exclusive::ExclusiveRuntime;
pub use runtime::managed::ManagedResource;
pub use runtime::pool::PoolRuntime;
pub use runtime::resident::ResidentRuntime;
pub use runtime::service::ServiceRuntime;
pub use runtime::transport::TransportRuntime;

// Topology configurations — used at registration time.
pub use topology::daemon::config::Config as DaemonConfig;
pub use topology::event_source::config::Config as EventSourceConfig;
pub use topology::exclusive::config::Config as ExclusiveConfig;
pub use topology::pooled::config::Config as PoolConfig;
pub use topology::resident::config::Config as ResidentConfig;
pub use topology::service::config::Config as ServiceConfig;
pub use topology::transport::config::Config as TransportConfig;

// Backward-compatibility re-exports (deprecated, will be removed).
#[allow(deprecated)]
pub use compat::{Context, Scope};
