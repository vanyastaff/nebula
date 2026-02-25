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
//! let ctx = Context::new(Scope::Global, "wf-1", "ex-1");
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

pub mod context;
#[cfg(feature = "credentials")]
pub mod credentials;
pub mod error;
pub mod guard;
pub mod lifecycle;
pub mod metadata;
pub mod reference;
pub mod resource;
pub mod scope;

// Modules requiring the tokio runtime
#[cfg(feature = "tokio")]
pub mod autoscale;
#[cfg(feature = "tokio")]
pub mod events;
#[cfg(feature = "tokio")]
pub mod health;
#[cfg(feature = "tokio")]
pub mod hooks;
#[cfg(feature = "tokio")]
pub mod manager;
#[cfg(feature = "tokio")]
#[cfg(feature = "metrics")]
pub mod metrics;
#[cfg(feature = "tokio")]
pub mod pool;
#[cfg(feature = "tokio")]
pub mod quarantine;

pub use context::Context;
pub use error::{Error, FieldViolation, Result};
pub use guard::Guard;
pub use lifecycle::Lifecycle;
pub use metadata::ResourceMetadata;
pub use reference::{ResourceProvider, ResourceRef};
pub use resource::{Config, Resource};
pub use scope::{Scope, Strategy};

#[cfg(feature = "tokio")]
pub use autoscale::{AutoScalePolicy, AutoScaler};
#[cfg(feature = "tokio")]
pub use events::{CleanupReason, EventBus, ResourceEvent};
#[cfg(feature = "tokio")]
pub use health::{
    ConnectivityStage, HealthCheckConfig, HealthCheckable, HealthChecker, HealthPipeline,
    HealthRecord, HealthStage, HealthState, HealthStatus, PerformanceStage,
};
#[cfg(feature = "tokio")]
pub use hooks::{
    AuditHook, HookEvent, HookFilter, HookRegistry, HookResult, ResourceHook, SlowAcquireHook,
};
#[cfg(feature = "tokio")]
pub use manager::{
    AnyGuard, AnyGuardTrait, DependencyGraph, Manager, ResourceHandle, ShutdownConfig,
    TypedResourceGuard,
};
#[cfg(feature = "tokio")]
#[cfg(feature = "metrics")]
pub use metrics::MetricsCollector;
#[cfg(feature = "tokio")]
pub use pool::{Pool, PoolConfig, PoolStats, PoolStrategy};
#[cfg(feature = "tokio")]
pub use quarantine::{
    QuarantineConfig, QuarantineEntry, QuarantineManager, QuarantineReason, RecoveryStrategy,
};

/// Convenience re-exports of the most commonly used types.
///
/// ```rust
/// use nebula_resource::prelude::*;
/// ```
pub mod prelude {
    pub use crate::context::Context;
    pub use crate::error::{Error, Result};
    pub use crate::guard::Guard;
    pub use crate::lifecycle::Lifecycle;
    pub use crate::metadata::ResourceMetadata;
    pub use crate::reference::{ResourceProvider, ResourceRef};
    pub use crate::resource::{Config, Resource};
    pub use crate::scope::{Scope, Strategy};

    #[cfg(feature = "tokio")]
    pub use crate::autoscale::AutoScalePolicy;
    #[cfg(feature = "tokio")]
    pub use crate::events::{EventBus, ResourceEvent};
    #[cfg(feature = "tokio")]
    pub use crate::health::{HealthCheckable, HealthState, HealthStatus};
    #[cfg(feature = "tokio")]
    pub use crate::hooks::{HookEvent, HookFilter, HookRegistry, HookResult, ResourceHook};
    #[cfg(feature = "tokio")]
    pub use crate::manager::{Manager, ResourceHandle, TypedResourceGuard};
    #[cfg(feature = "tokio")]
    pub use crate::pool::{Pool, PoolConfig, PoolStats, PoolStrategy};
}
