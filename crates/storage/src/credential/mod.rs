//! Credential persistence — durable stores (`SqliteCredentialPersistence`,
//! `PgCredentialPersistence`), `KeyProvider`, and composable layers.
//!
//! Two distinct layer families live here:
//!
//! - `CredentialPersistence` wrappers — `EncryptionLayer`, `CacheLayer`,
//!   `AuditLayer`. They compose around a backing store that persists
//!   `StoredCredential` rows. Owner scoping is mandatory in the port itself;
//!   every adapter predicate receives a complete owner-bound selector.
//! - `ExternalProvider` wrappers — `ProviderCacheLayer`. They compose
//!   around an `Arc<dyn ExternalProvider>` that resolves secrets from a
//!   remote system (Vault, AWS SM, env var, …). The `Provider`-prefixed types
//!   are scoped to that trait and do not interact with the `CredentialPersistence`
//!   cache.
//!
//! The `CredentialPersistence` trait + DTOs live in `nebula-storage-port`; concrete
//! implementations and layers live here. See `crates/storage/README.md` and
//! `docs/INTEGRATION_MODEL.md` (Credential) for integration context.

#[cfg(test)]
mod conformance;
#[cfg(test)]
mod conformance_tests;
pub mod key_provider;
pub mod layer;
pub mod provider_cache;
#[cfg(any(test, feature = "sqlite", feature = "postgres"))]
mod schema;

#[cfg(any(test, feature = "credential-in-memory"))]
pub mod pending;
#[cfg(test)]
mod reference;
mod retry_gate;

/// Cross-replica refresh claim repository (CAS + heartbeat).
pub mod refresh_claim;

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(test)]
pub(crate) use conformance::CredentialPersistenceConformance;
pub use key_provider::{EnvKeyProvider, FileKeyProvider, KeyProvider, KeySnapshot, ProviderError};
pub use layer::{
    AuditEvent, AuditLayer, AuditOperation, AuditResult, AuditSink, CacheConfig, CacheLayer,
    CacheStats, EncryptionLayer,
};
#[cfg(any(test, feature = "credential-in-memory"))]
pub use pending::InMemoryPendingStore;
#[cfg(feature = "postgres")]
pub use postgres::PgCredentialPersistence;
pub use provider_cache::{ProviderCacheConfig, ProviderCacheLayer, ProviderCacheStats};
#[cfg(test)]
pub(crate) use reference::ReferenceCredentialPersistence;
#[cfg(feature = "postgres")]
pub use refresh_claim::PgRefreshClaimRepo;
#[cfg(feature = "sqlite")]
pub use refresh_claim::SqliteRefreshClaimRepo;
pub use refresh_claim::{
    ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, InMemoryRefreshClaimRepo, RefreshClaim,
    RefreshClaimRepo, ReplicaId, RepoError, SentinelState,
};
#[cfg(any(test, feature = "sqlite", feature = "postgres"))]
pub use schema::{
    AdmissionReason as CredentialSchemaAdmissionReason, CredentialStoreStartupError,
    UnsupportedSchemaVersion,
};
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteCredentialPersistence;

/// Crate-local helpers for constructing credential lifecycle test commands.
/// Gated on `sqlite` because all callers are `#[cfg(all(test, feature = "sqlite"))]` test modules.
#[cfg(all(test, feature = "sqlite"))]
pub(crate) mod test_support {
    use nebula_storage_port::{
        CredentialCreate, CredentialReplacement, CredentialVersion, SecretBytes,
    };

    pub(crate) fn make_credential(data: &[u8]) -> CredentialCreate {
        CredentialCreate::new(
            "test_credential".to_owned(),
            SecretBytes::new(data.to_vec()),
            "test".to_owned(),
            1,
            None,
            None,
            false,
            Default::default(),
        )
    }

    pub(crate) fn make_replacement(
        expected_version: CredentialVersion,
        data: &[u8],
        refresh_retry_transition: nebula_storage_port::RefreshRetryTransition,
    ) -> CredentialReplacement {
        let material_transition = match refresh_retry_transition {
            nebula_storage_port::RefreshRetryTransition::Clear => {
                nebula_storage_port::CredentialMaterialTransition::advance()
            },
            transition => nebula_storage_port::CredentialMaterialTransition::preserve(transition),
        };
        CredentialReplacement::new(
            expected_version,
            SecretBytes::new(data.to_vec()),
            "test".to_owned(),
            1,
            None,
            None,
            false,
            Default::default(),
            material_transition,
        )
    }
}
