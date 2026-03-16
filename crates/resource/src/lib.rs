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
//! - **TypeId index**: Type-safe acquisition via `acquire_typed<R>()` — no string key required
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
//! See [`mod@reference`] module for details on `ResourceProvider`.

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

pub mod instrumented;

pub mod autoscale;
pub mod events;
pub mod health;
pub mod hooks;
pub mod manager;
pub mod metrics;
pub mod poison;
pub mod pool;
pub mod quarantine;

pub use context::{Context, ResourcePoolHandle, TraceContext};
pub use error::{Error, ErrorCategory, FieldViolation, Result};
pub use guard::Guard;
pub use instrumented::InstrumentedGuard;
pub use lifecycle::Lifecycle;
pub use metadata::{ResourceMetadata, ResourceMetadataBuilder};
pub use reference::{ErasedResourceRef, ResourceProvider};
pub use resource::{AnyResource, Config, Resource, ResourceDependencies};
pub use scope::{Scope, Strategy};
// Re-export execution trace types from telemetry for compatibility.
pub use nebula_telemetry::{
    CallBody, CallPayload, CallRecord, CallStatus, DropReason, NoopRecorder, Recorder,
    ResourceUsageRecord,
};

pub use autoscale::{AutoScalePolicy, AutoScaler, AutoScalerHandle};
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
    AdaptiveBackpressurePolicy, InstanceMetadata, LatencyPercentiles, Pool, PoolAcquire,
    PoolBackpressurePolicy, PoolConfig, PoolLifetime, PoolResiliencePolicy, PoolSharingMode,
    PoolSizing, PoolStats, PoolStrategy, RetryConfig,
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
    pub use crate::context::Context;
    pub use crate::error::{Error, ErrorCategory, Result};
    pub use crate::guard::Guard;
    pub use crate::lifecycle::Lifecycle;
    pub use crate::metadata::ResourceMetadata;
    pub use crate::reference::{ErasedResourceRef, ResourceProvider};
    pub use crate::resource::{AnyResource, Config, Resource, ResourceDependencies};
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
