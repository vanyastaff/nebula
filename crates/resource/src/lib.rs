//! # Nebula Resource Management
//!
//! Resource lifecycle management for the Nebula workflow engine.
//! Provides lifecycle management, pooling, scoping, and health checks
//! for resources used within workflows and actions.

pub mod context;
pub mod error;
pub mod guard;
pub mod lifecycle;
pub mod resource;
pub mod scope;

// Modules requiring the tokio runtime
#[cfg(feature = "tokio")]
pub mod health;
#[cfg(feature = "tokio")]
pub mod manager;
#[cfg(feature = "tokio")]
pub mod pool;

pub use context::Context;
pub use error::{Error, Result};
pub use guard::Guard;
pub use lifecycle::Lifecycle;
pub use resource::{Config, Resource};
pub use scope::{Scope, Strategy};

#[cfg(feature = "tokio")]
pub use health::{HealthCheckConfig, HealthCheckable, HealthChecker, HealthState, HealthStatus};
#[cfg(feature = "tokio")]
pub use manager::{AnyGuard, AnyGuardTrait, DependencyGraph, Manager};
#[cfg(feature = "tokio")]
pub use pool::{Pool, PoolConfig, PoolStats};
