//! Core types and traits for resource management

pub mod context;
pub mod error;
pub mod lifecycle;
pub mod resource;
pub mod scoping;
pub mod traits;

// Re-exports
pub use context::ResourceContext;
pub use error::{ResourceError, ResourceResult};
pub use lifecycle::LifecycleState;
pub use resource::{Resource, ResourceFactory, ResourceInstance};
pub use scoping::ResourceScope;
pub use traits::{HealthCheckable, Poolable, Stateful};