//! # Nebula Resource Management
//!
//! Resource lifecycle management for the Nebula workflow engine.
//! Provides lifecycle management, pooling, scoping, and health checks
//! for resources used within workflows and actions.

#![warn(missing_docs)]

pub mod context;
#[cfg(feature = "credentials")]
pub mod credentials;
pub mod error;
pub mod guard;
pub mod lifecycle;
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
    pub use crate::manager::{Manager, ResourceHandle};
    #[cfg(feature = "tokio")]
    pub use crate::pool::{Pool, PoolConfig, PoolStats, PoolStrategy};
}
