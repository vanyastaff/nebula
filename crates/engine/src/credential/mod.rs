//! Engine-owned credential composition helper (**Plane B**).
//!
//! Runtime credential resolution for **integration credentials** — workflow
//! access to external systems per [`Credential`](nebula_credential::Credential)
//! — lives in `nebula_credential::runtime` (ADR-0092); callers import those
//! types from their canonical home. This module retains only
//! [`default_in_memory_coordinator`], which constructs `InMemoryRefreshClaimRepo`
//! from `nebula-storage` — a dependency credential consumers may not link — and
//! therefore cannot fold into the credential crate.
//!
//! It does not implement platform/operator authentication (**Plane A**).

use nebula_credential::runtime::{ConfigError, RefreshCoordConfig, RefreshCoordinator};

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

    use nebula_storage_port::store::{RefreshClaimStore, ReplicaId};

    let repo: Arc<dyn RefreshClaimStore> =
        Arc::new(nebula_storage::credential::InMemoryRefreshClaimRepo::new());
    RefreshCoordinator::new_with(
        repo,
        ReplicaId::new("nebula-engine-default"),
        RefreshCoordConfig::default(),
    )
}
