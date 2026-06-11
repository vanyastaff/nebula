//! Engine-owned credential orchestration primitives (**Plane B**).
//!
//! This module hosts runtime credential resolution used by the execution
//! engine for **integration credentials** — workflow
//! access to external systems per [`Credential`](nebula_credential::Credential)
//! and (`crates/engine/README.md`). It does
//! not implement platform/operator authentication (**Plane A**).

pub mod lease;
pub mod refresh;
pub mod resolver;
#[cfg(feature = "rotation")]
pub mod rotation;

// `dispatchers` / `executor` / `scoped_accessor` were relocated to
// `nebula_credential::runtime` (ADR-0092); re-exported here so
// `nebula_engine::credential::*` consumers keep resolving.
pub use lease::{
    LeaseLifecycle, LeaseLifecycleConfig, LeaseLifecycleError, LeaseToken, RenewalPolicy,
};
pub use nebula_credential::runtime::{
    ExecutorError, ResolveResponse, ScopedCredentialAccessor, dispatch_release, dispatch_revoke,
    dispatch_test, execute_continue, execute_resolve,
};
// Re-export TestResult for the dispatchers module to reference, and to
// give downstream callers a single import surface for the capability
// dispatch path.
pub use nebula_credential::resolve::TestResult;
pub use refresh::{RefreshAttempt, RefreshCoordinator};
pub use resolver::{CredentialResolver, ResolveError};
