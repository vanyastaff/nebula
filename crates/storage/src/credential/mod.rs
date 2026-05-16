//! Credential persistence — `InMemoryStore`, `KeyProvider`, and composable
//! layers.
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
//!   remote system (Vault, AWS SM, env var, …) per ADR-0051. The
//!   `Provider`-prefixed types are scoped to that trait and do not interact
//!   with the `CredentialStore` cache.
//!
//! The `CredentialStore` trait + DTOs live in `nebula_credential` per
//! [ADR-0032](../../../../docs/adr/0032-credential-store-canonical-home.md);
//! only the concrete implementations are owned by storage
//! ([ADR-0029](../../../../docs/adr/0029-storage-owns-credential-persistence.md)).
//!
//! See also [ADR-0028](../../../../docs/adr/0028-cross-crate-credential-invariants.md)
//! for the umbrella cross-crate invariants, and
//! [ADR-0051](../../../../docs/adr/0051-external-provider-redesign.md) for
//! the `ExternalProvider` design that motivates `ProviderCacheLayer`.

pub mod key_provider;
pub mod layer;
pub mod provider_cache;

#[cfg(any(test, feature = "credential-in-memory"))]
pub mod memory;

#[cfg(any(test, feature = "credential-in-memory"))]
pub mod pending;

#[cfg(feature = "rotation")]
pub mod backup;

/// Cross-replica refresh claim repository — see ADR-0041.
pub mod refresh_claim;

#[cfg(feature = "rotation")]
pub use backup::RotationBackup;
#[cfg(any(test, feature = "test-util"))]
pub use key_provider::StaticKeyProvider;
pub use key_provider::{EnvKeyProvider, FileKeyProvider, KeyProvider, ProviderError};
pub use layer::{
    AuditEvent, AuditLayer, AuditOperation, AuditResult, AuditSink, CacheConfig, CacheLayer,
    CacheStats, EncryptionLayer,
};
#[cfg(any(test, feature = "credential-in-memory"))]
pub use memory::InMemoryStore;
#[cfg(any(test, feature = "credential-in-memory"))]
pub use pending::InMemoryPendingStore;
pub use provider_cache::{ProviderCacheConfig, ProviderCacheLayer, ProviderCacheStats};
#[cfg(feature = "postgres")]
pub use refresh_claim::PgRefreshClaimRepo;
#[cfg(feature = "sqlite")]
pub use refresh_claim::SqliteRefreshClaimRepo;
pub use refresh_claim::{
    ClaimAttempt, ClaimToken, HeartbeatError, InMemoryRefreshClaimRepo, ReclaimedClaim,
    RefreshClaim, RefreshClaimRepo, ReplicaId, RepoError, SentinelState,
};
