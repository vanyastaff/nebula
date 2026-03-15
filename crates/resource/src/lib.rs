//! # Nebula Resource Management
//!
//! Resource lifecycle management for the Nebula workflow engine.
//! Provides lifecycle management, pooling, scoping, health checks,
//! and provider abstractions for resources used within workflows and actions.
//!
//! ## Core Concepts
//!
//! - **Resource**: Trait defining a reusable resource type (database connection, HTTP client, etc.)
//! - **Manager**: Central registry managing multiple resource pools with dependency ordering
//! - **Pool**: Efficient resource pooling with configurable strategies (FIFO/LIFO)
//! - **Context**: Execution context carrying scope, tenant, workflow, and cancellation info
//! - **ResourceProvider**: Trait for decoupled resource acquisition (type-safe + dynamic)
//! - **ResourceRef**: Type-safe wrapper around `TypeId` for resource identification
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use nebula_resource::{Manager, Resource, PoolConfig, Context, Scope};
//!
//! // Define a resource
//! struct DbResource;
//! impl Resource for DbResource {
//!     type Config = DbConfig;
//!     type Instance = DbConnection;
//!     fn id() -> &'static str { "postgres" }
//! }
//!
//! // Register and acquire
//! let manager = Manager::new();
//! manager.register(DbResource, config, PoolConfig::default())?;
//!
//! let ctx = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new());
//! let conn = manager.acquire("postgres", &ctx).await?;
//! ```
//!
//! ## ResourceProvider Pattern
//!
//! For decoupled resource access in actions/triggers:
//!
//! ```rust,ignore
//! use nebula_resource::{ResourceProvider, Resource};
//!
//! // Type-safe acquisition
//! let conn = provider.resource::<DbResource>(&ctx).await?;
//!
//! // Dynamic acquisition
//! let any = provider.acquire("postgres", &ctx).await?;
//! ```
//!
//! See [`mod@reference`] module for details on `ResourceRef` and `ResourceProvider`.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod context;
pub mod error;
pub mod guard;
pub mod lifecycle;
pub mod metadata;
pub mod reference;
pub mod resource;
pub mod scope;

pub mod any;
pub mod dependency;
pub mod handler;
pub(crate) mod dependency_graph;
pub mod instrumented;
pub(crate) mod manager_guard;
pub(crate) mod manager_pool;

pub mod autoscale;
pub mod events;
pub mod health;
pub mod hooks;
pub mod manager;
pub mod metrics;
pub mod poison;
pub mod pool;
pub mod quarantine;

pub use any::AnyResource;
pub use handler::TypedCredentialHandler;
pub use dependency::ResourceDependencies;
pub use context::{Context, ResourcePoolHandle};
pub use error::{Error, ErrorCategory, FieldViolation, Result};
pub use guard::Guard;
pub use instrumented::InstrumentedGuard;
pub use lifecycle::Lifecycle;
pub use metadata::{ResourceMetadata, ResourceMetadataBuilder};
pub use reference::{ErasedResourceRef, ResourceProvider, ResourceRef};
pub use resource::{Config, Resource};
pub use scope::{Scope, Strategy};
// Re-export execution trace types from telemetry for compatibility.
pub use nebula_telemetry::{
    CallBody, CallPayload, CallRecord, CallStatus, DropReason, NoopRecorder, Recorder,
    ResourceUsageRecord,
};

pub use autoscale::{AutoScalePolicy, AutoScaler};
pub use events::{
    BackPressurePolicy, CleanupReason, EventBus, EventBusStats, EventFilter, EventSubscriber,
    QuarantineTrigger, ResourceEvent, ScopedEvent, ScopedSubscriber, SubscriptionScope,
};
pub use health::{
    HealthCheckConfig, HealthCheckable, HealthChecker, HealthRecord, HealthStage, HealthState,
    HealthStatus, ResourceHealthAdapter, ThresholdCallback,
};
pub use hooks::{
    AuditHook, HookEvent, HookFilter, HookRegistry, HookResult, ResourceHook, SlowAcquireHook,
};
pub use manager::{
    AnyGuard, AnyGuardTrait, DependencyGraph, Manager, ManagerBuilder, ResourceHandle,
    ResourcePoolStatus, ResourceStatus, ShutdownConfig, TypedPool, TypedResourceGuard,
};
pub use metrics::MetricsCollector;
pub use poison::{Poison, PoisonError, PoisonGuard};
pub use pool::{
    AdaptiveBackpressurePolicy, LatencyPercentiles, Pool, PoolBackpressurePolicy, PoolConfig,
    PoolStats, PoolStrategy,
};
pub use quarantine::{
    QuarantineConfig, QuarantineEntry, QuarantineManager, QuarantineReason, RecoveryStrategy,
};

/// Re-export id and key types from [`nebula_core`] for convenience.
pub use nebula_core::{ExecutionId, PluginKey, ResourceId, ResourceKey, WorkflowId};

/// Convenience re-exports of the most commonly used types.
///
/// ```rust
/// use nebula_resource::prelude::*;
/// ```
pub mod prelude {
    pub use crate::any::AnyResource;
    pub use crate::dependency::ResourceDependencies;
    pub use crate::context::Context;
    pub use crate::error::{Error, ErrorCategory, Result};
    pub use crate::guard::Guard;
    pub use crate::lifecycle::Lifecycle;
    pub use crate::metadata::ResourceMetadata;
    pub use crate::reference::{ErasedResourceRef, ResourceProvider, ResourceRef};
    pub use crate::resource::{Config, Resource};
    pub use crate::scope::{Scope, Strategy};

    pub use crate::autoscale::AutoScalePolicy;
    pub use crate::events::{
        BackPressurePolicy, EventBus, EventBusStats, EventFilter, EventSubscriber,
        QuarantineTrigger, ResourceEvent, ScopedEvent, ScopedSubscriber, SubscriptionScope,
    };
    pub use crate::health::{HealthCheckable, HealthState, HealthStatus, ResourceHealthAdapter};
    pub use crate::hooks::{HookEvent, HookFilter, HookRegistry, HookResult, ResourceHook};
    pub use crate::manager::{
        Manager, ManagerBuilder, ResourceHandle, ResourcePoolStatus, ResourceStatus,
        TypedResourceGuard,
    };
    pub use crate::pool::{
        AdaptiveBackpressurePolicy, LatencyPercentiles, Pool, PoolBackpressurePolicy, PoolConfig,
        PoolStats, PoolStrategy,
    };

    pub use nebula_core::{ExecutionId, ResourceId, ResourceKey, WorkflowId};
}
