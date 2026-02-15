#![allow(clippy::excessive_nesting)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::type_complexity)]
#![allow(clippy::if_same_then_else)]
#![allow(clippy::collapsible_if)]

//! # Nebula Resource Management
//!
//! Resource lifecycle management for the Nebula workflow engine.
//! Provides lifecycle management, pooling, scoping, and health checks
//! for resources used within workflows and actions.
//!
//! ## Features
//!
//! - **Lifecycle Management**: Automatic initialization, health checks, and cleanup
//! - **Resource Pooling**: Efficient connection pooling with configurable strategies
//! - **Scoped Resources**: Support for Global, Tenant, Workflow, and Action-level scoping
//! - **Credential Integration**: Seamless integration with `nebula-credential`
//! - **Dependency Management**: Automatic dependency resolution

// Core modules
pub mod core;
pub mod health;
pub mod manager;
pub mod pool;

#[cfg(feature = "testing")]
pub mod testing;

// Re-exports for convenience
pub use crate::core::{
    context::ResourceContext,
    error::{ResourceError, ResourceResult},
    lifecycle::LifecycleState,
    resource::{Resource, ResourceFactory, ResourceInstance},
    scoping::ResourceScope,
    traits::{HealthCheckable, Poolable},
};

pub use crate::manager::{ResourceManager, ResourceManagerBuilder};

#[cfg(feature = "pooling")]
pub use crate::pool::{PoolConfig, PoolStrategy, ResourcePool};

/// Prelude module that re-exports the most commonly used types and traits
pub mod prelude {
    pub use crate::core::{
        context::ResourceContext,
        error::{ResourceError, ResourceResult},
        lifecycle::LifecycleState,
        resource::{Resource, ResourceFactory, ResourceInstance},
        scoping::ResourceScope,
        traits::{HealthCheckable, Poolable},
    };

    pub use crate::manager::{ResourceManager, ResourceManagerBuilder};

    #[cfg(feature = "pooling")]
    pub use crate::pool::{PoolConfig, PoolStrategy, ResourcePool};

    #[cfg(feature = "serde")]
    pub use serde::{Deserialize, Serialize};

    pub use async_trait::async_trait;
    pub use uuid::Uuid;
}

/// The version of this crate
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
