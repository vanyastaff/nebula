//! Engine-owned credential orchestration primitives (**Plane B**).
//!
//! This module hosts runtime credential resolution used by the execution
//! engine for **integration credentials** — workflow access to external
//! systems per [`Credential`](nebula_credential::Credential). It does not
//! implement platform/operator authentication (**Plane A**).
//!
//! `CredentialResolver` and `ResolveError` are re-exported from
//! `nebula_credential::runtime` (ADR-0092). `default_in_memory_coordinator`
//! remains here because it constructs `InMemoryRefreshClaimRepo` from
//! `nebula-storage`, which may not be linked by credential consumers.

pub mod reqwest_transport;
#[cfg(feature = "rotation")]
pub mod rotation;

pub use reqwest_transport::ReqwestRefreshTransport;

// `dispatchers` / `executor` / `scoped_accessor` / `lease` / `refresh` /
// `resolver` were relocated to `nebula_credential::runtime` (ADR-0092);
// re-exported here so `nebula_engine::credential::*` consumers keep resolving.
pub use nebula_credential::runtime::{
    ConfigError, CredentialResolver, ExecutorError, LeaseLifecycle, LeaseLifecycleConfig,
    LeaseLifecycleError, LeaseToken, ReclaimSweepHandle, RefreshAttempt, RefreshConfigError,
    RefreshCoordConfig, RefreshCoordMetrics, RefreshCoordinator, RefreshError, RefreshTransport,
    RefreshTransportError, RenewalPolicy, ResolveError, ResolveResponse, ScopedCredentialAccessor,
    SentinelDecision, SentinelThresholdConfig, SentinelTrigger, TokenPostRequest,
    TokenPostResponse, dispatch_release, dispatch_revoke, dispatch_test, execute_continue,
    execute_resolve,
};
// Re-export TestResult for the dispatchers module to reference, and to give
// downstream callers a single import surface for the capability dispatch path.
pub use nebula_credential::resolve::TestResult;

/// Build a default in-memory [`RefreshCoordinator`] backed by
/// `InMemoryRefreshClaimRepo` for tests and single-replica desktop mode.
///
/// Production composition should supply a durable
/// [`nebula_storage_port::store::RefreshClaimStore`] (Postgres / SQLite) via
/// [`RefreshCoordinator::new_with`] instead.
///
/// # Errors
///
/// Returns [`ConfigError`] if metric handle registration fails. In practice
/// this cannot occur for the static default config.
pub fn default_in_memory_coordinator() -> Result<RefreshCoordinator, ConfigError> {
    use std::sync::Arc;

    use nebula_storage_port::store::RefreshClaimStore;
    use nebula_storage_port::store::ReplicaId;

    let repo: Arc<dyn RefreshClaimStore> =
        Arc::new(nebula_storage::credential::InMemoryRefreshClaimRepo::new());
    RefreshCoordinator::new_with(
        repo,
        ReplicaId::new("nebula-engine-default"),
        RefreshCoordConfig::default(),
    )
}
