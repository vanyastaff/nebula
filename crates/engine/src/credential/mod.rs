//! Engine-owned credential orchestration primitives.
//!
//! This module hosts runtime credential resolution and type-erased registry
//! logic used by the execution engine.

pub mod executor;
pub mod registry;
pub mod resolver;
#[cfg(feature = "rotation")]
pub mod rotation;

pub use executor::{ExecutorError, ResolveResponse, execute_continue, execute_resolve};
pub use registry::{CredentialRegistry, RegistryError};
pub use resolver::{CredentialResolver, ResolveError};
