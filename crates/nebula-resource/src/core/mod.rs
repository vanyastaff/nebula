//! Core types and traits for resource management
pub mod versioning;
pub mod dependency;
pub mod context;
pub mod error;
pub mod lifecycle;
pub mod resource;
pub mod scoping;
pub mod traits;

// Re-exports
pub use context::ResourceContext;
pub use dependency::DependencyGraph;
pub use error::{ResourceError, ResourceResult};
pub use lifecycle::LifecycleState;
pub use resource::{Resource, ResourceFactory, ResourceInstance};
pub use scoping::ResourceScope;
pub use traits::{HealthCheckable, Poolable, Stateful};
pub use versioning::{Version, VersionChecker};

