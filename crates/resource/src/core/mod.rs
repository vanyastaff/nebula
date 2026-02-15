//! Core types and traits for resource management
pub mod authenticated;
pub mod context;
pub mod dependency;
pub mod error;
pub mod lifecycle;
pub mod resource;
pub mod scoping;
pub mod traits;
pub mod versioning;
// Re-exports
#[cfg(feature = "credentials")]
pub use authenticated::AuthenticatedResource;
pub use context::ResourceContext;
pub use dependency::DependencyGraph;
pub use error::{ResourceError, ResourceResult};
pub use lifecycle::LifecycleState;
pub use resource::{Resource, ResourceFactory, ResourceInstance};
pub use scoping::ResourceScope;
pub use traits::{HealthCheckable, Poolable};
pub use versioning::{Version, VersionChecker};
