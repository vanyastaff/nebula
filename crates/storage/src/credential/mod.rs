//! Credential persistence — durable stores (`SqliteCredentialStore`,
//! `PgCredentialStore`), `KeyProvider`, and composable layers.
//!
//! Two distinct layer families live here:
//!
//! - `CredentialStore` wrappers — `EncryptionLayer`, `CacheLayer`,
//!   `AuditLayer`. They compose around a backing store that persists
//!   `StoredCredential` rows. The multi-tenant scope wrapper was
//!   re-homed to `nebula_tenancy::CredentialScopeLayer` (spec §8) —
//!   scope policy belongs in the tenancy security boundary; it
//!   re-composes outermost at the composition root.
//! - `ExternalProvider` wrappers — `ProviderCacheLayer`. They compose
//!   around an `Arc<dyn ExternalProvider>` that resolves secrets from a
//!   remote system (Vault, AWS SM, env var, …). The `Provider`-prefixed types
//!   are scoped to that trait and do not interact with the `CredentialStore`
//!   cache.
//!
//! The `CredentialStore` trait + DTOs live in `nebula_credential`; concrete
//! implementations and layers live here. See `crates/storage/README.md` and
//! `docs/INTEGRATION_MODEL.md` (Credential) for integration context.

pub mod key_provider;
pub mod layer;
pub mod provider_cache;

#[cfg(any(test, feature = "credential-in-memory"))]
pub mod pending;

#[cfg(feature = "rotation")]
pub mod backup;

/// Cross-replica refresh claim repository (CAS + heartbeat).
pub mod refresh_claim;

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "rotation")]
pub use backup::RotationBackup;
pub use key_provider::{EnvKeyProvider, FileKeyProvider, KeyProvider, ProviderError};
pub use layer::{
    AuditEvent, AuditLayer, AuditOperation, AuditResult, AuditSink, CacheConfig, CacheLayer,
    CacheStats, EncryptionLayer,
};
#[cfg(any(test, feature = "credential-in-memory"))]
pub use pending::InMemoryPendingStore;
#[cfg(feature = "postgres")]
pub use postgres::PgCredentialStore;
pub use provider_cache::{ProviderCacheConfig, ProviderCacheLayer, ProviderCacheStats};
#[cfg(feature = "postgres")]
pub use refresh_claim::PgRefreshClaimRepo;
#[cfg(feature = "sqlite")]
pub use refresh_claim::SqliteRefreshClaimRepo;
pub use refresh_claim::{
    ClaimAttempt, ClaimToken, HeartbeatError, InMemoryRefreshClaimRepo, ReclaimedClaim,
    RefreshClaim, RefreshClaimRepo, ReplicaId, RepoError, SentinelState,
};
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteCredentialStore;

/// Crate-local test helpers for constructing [`nebula_credential::StoredCredential`] instances.
/// Gated on `sqlite` because all callers are `#[cfg(all(test, feature = "sqlite"))]` test modules.
#[cfg(all(test, feature = "sqlite"))]
pub(crate) mod test_support {
    use nebula_credential::StoredCredential;

    pub(crate) fn make_credential(id: &str, data: &[u8]) -> StoredCredential {
        StoredCredential {
            id: id.into(),
            name: None,
            credential_key: "test_credential".into(),
            data: data.to_vec(),
            state_kind: "test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            reauth_required: false,
            metadata: Default::default(),
        }
    }
}
