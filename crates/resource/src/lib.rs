#![allow(clippy::excessive_nesting)]

//! # Nebula Resource Management
//!
//! Resource lifecycle management for the Nebula workflow engine.
//! Provides lifecycle management, pooling, scoping, and health checks
//! for resources used within workflows and actions.

pub mod context;
pub mod error;
pub mod health;
pub mod lifecycle;
pub mod manager;
pub mod pool;
pub mod resource;
pub mod scope;

// Re-exports
pub use context::ResourceContext;
pub use error::{ResourceError, ResourceResult};
pub use health::{HealthCheckConfig, HealthCheckable, HealthChecker, HealthState, HealthStatus};
pub use lifecycle::LifecycleState;
pub use pool::{Pool, PoolConfig, PoolStats};
pub use resource::{Resource, ResourceConfig, ResourceGuard};
pub use scope::{ResourceScope, ScopingStrategy};

pub use manager::ResourceManager;
pub use manager::dependency::DependencyGraph;
