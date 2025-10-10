//! # Nebula Resource Management
//!
//! A comprehensive resource management framework for the Nebula workflow engine.
//! Provides lifecycle management, pooling, scoping, and observability for all resources
//! used within workflows and actions.
//!
//! ## Features
//!
//! - **Lifecycle Management**: Automatic initialization, health checks, and cleanup
//! - **Resource Pooling**: Efficient connection pooling with configurable strategies
//! - **Context Awareness**: Automatic context propagation for tracing and multi-tenancy
//! - **Credential Integration**: Seamless integration with `nebula-credential`
//! - **Built-in Observability**: Metrics, logging, and distributed tracing
//! - **Scoped Resources**: Support for Global, Tenant, Workflow, and Action-level scoping
//! - **Dependency Management**: Automatic dependency resolution
//! - **Extensible**: Plugin system and hooks for custom behavior
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use nebula_resource::prelude::*;
//! use async_trait::async_trait;
//!
//! // Define a resource
//! #[derive(Resource)]
//! #[resource(
//!     id = "database",
//!     name = "PostgreSQL Database",
//!     poolable = true,
//!     health_checkable = true
//! )]
//! pub struct DatabaseResource;
//!
//! // Define configuration
//! #[derive(ResourceConfig)]
//! pub struct DatabaseConfig {
//!     pub connection_string: String,
//!     pub max_connections: u32,
//!     pub idle_timeout_seconds: u64,
//! }
//!
//! // Define instance
//! pub struct DatabaseInstance {
//!     // Your database connection here
//! }
//!
//! // Implement resource trait
//! #[async_trait]
//! impl Resource for DatabaseResource {
//!     type Config = DatabaseConfig;
//!     type Instance = DatabaseInstance;
//!
//!     async fn create(
//!         &self,
//!         config: &Self::Config,
//!         context: &ResourceContext,
//!     ) -> Result<Self::Instance, ResourceError> {
//!         // Create database connection
//!         let connection = connect_to_database(config).await?;
//!         Ok(DatabaseInstance::new(connection, context.clone()))
//!     }
//! }
//! ```

#![deny(unsafe_code)]
#![warn(
    missing_docs,
    missing_debug_implementations,
    missing_copy_implementations,
    trivial_casts,
    trivial_numeric_casts,
    unused_import_braces,
    unused_qualifications
)]

// Core modules
pub mod core;
pub mod health;
pub mod manager;
pub mod observability;
pub mod pool;
pub mod stateful;

// Feature-gated modules
#[cfg(feature = "credentials")]
pub mod credentials;

#[cfg(feature = "testing")]
pub mod testing;

// Built-in resources
pub mod resources;

// Re-exports for convenience
pub use crate::core::{
    context::ResourceContext,
    error::{ResourceError, ResourceResult},
    lifecycle::LifecycleState,
    resource::{Resource, ResourceFactory, ResourceInstance},
    scoping::ResourceScope,
    traits::{HealthCheckable, Poolable, Stateful},
};

pub use crate::manager::{ResourceManager, ResourceManagerBuilder};

#[cfg(feature = "pooling")]
pub use crate::pool::{PoolConfig, PoolStrategy, ResourcePool};

/// Prelude module that re-exports the most commonly used types and traits
///
/// This is intended to be glob imported to bring the most important
/// types into scope:
///
/// ```rust
/// use nebula_resource::prelude::*;
/// ```
pub mod prelude {
    pub use crate::core::{
        context::ResourceContext,
        error::{ResourceError, ResourceResult},
        lifecycle::LifecycleState,
        resource::{Resource, ResourceFactory, ResourceInstance},
        scoping::ResourceScope,
        traits::{HealthCheckable, Poolable, Stateful},
    };

    pub use crate::manager::{ResourceManager, ResourceManagerBuilder};

    #[cfg(feature = "pooling")]
    pub use crate::pool::{PoolConfig, PoolStrategy, ResourcePool};

    #[cfg(feature = "serde")]
    pub use serde::{Deserialize, Serialize};

    pub use async_trait::async_trait;
    pub use uuid::Uuid;

    // Re-export derive macros
    // pub use nebula_derive::{Resource, ResourceConfig};
}

// Version information
/// The version of this crate
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
