//! Credential persistence — `InMemoryStore`, `KeyProvider`, and composable
//! layers (`EncryptionLayer`, `CacheLayer`, `AuditLayer`, `ScopeLayer`).
//!
//! The `CredentialStore` trait + DTOs live in `nebula_credential` per
//! [ADR-0032](../../../../docs/adr/0032-credential-store-canonical-home.md);
//! only the concrete implementations are owned by storage
//! ([ADR-0029](../../../../docs/adr/0029-storage-owns-credential-persistence.md)).
//!
//! See also [ADR-0028](../../../../docs/adr/0028-cross-crate-credential-invariants.md)
//! for the umbrella cross-crate invariants.

pub mod key_provider;
pub mod layer;

#[cfg(any(test, feature = "credential-in-memory"))]
pub mod memory;

#[cfg(any(test, feature = "credential-in-memory"))]
pub mod pending;

#[cfg(feature = "rotation")]
pub mod backup;

#[cfg(feature = "rotation")]
pub use backup::RotationBackup;
#[cfg(any(test, feature = "test-util"))]
pub use key_provider::StaticKeyProvider;
pub use key_provider::{EnvKeyProvider, FileKeyProvider, KeyProvider, ProviderError};
pub use layer::{
    AuditEvent, AuditLayer, AuditOperation, AuditResult, AuditSink, CacheConfig, CacheLayer,
    CacheStats, EncryptionLayer, ScopeLayer, ScopeResolver,
};
#[cfg(any(test, feature = "credential-in-memory"))]
pub use memory::InMemoryStore;
#[cfg(any(test, feature = "credential-in-memory"))]
pub use pending::InMemoryPendingStore;
