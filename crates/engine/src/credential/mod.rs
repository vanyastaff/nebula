//! Engine-owned credential orchestration primitives (**Plane B**).
//!
//! This module hosts runtime credential resolution and type-erased registry
//! logic used by the execution engine for **integration credentials** — workflow
//! access to external systems per [`Credential`](nebula_credential::Credential)
//! and ADR-0033 (`docs/adr/0033-integration-credentials-plane-b.md`). It does
//! not implement platform/operator authentication (**Plane A**).

pub mod executor;
pub mod refresh;
pub mod registry;
pub mod resolver;
#[cfg(feature = "rotation")]
pub mod rotation;

pub use executor::{ExecutorError, ResolveResponse, execute_continue, execute_resolve};
pub use refresh::{RefreshAttempt, RefreshCoordinator};
pub use registry::{CredentialRegistry, RegistryError};
pub use resolver::{CredentialResolver, ResolveError};
