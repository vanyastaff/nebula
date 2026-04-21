//! Credential persistence — `CredentialStore` trait, `InMemoryStore`,
//! `KeyProvider`, and composable layers (`EncryptionLayer`, `CacheLayer`,
//! `AuditLayer`, `ScopeLayer`).
//!
//! See [ADR-0028](../../../../docs/adr/0028-cross-crate-credential-invariants.md)
//! (umbrella invariants) and
//! [ADR-0029](../../../../docs/adr/0029-storage-owns-credential-persistence.md).

pub mod key_provider;
pub mod layer;
pub mod store;

#[cfg(any(test, feature = "credential-in-memory"))]
pub mod memory;

// TODO(P6.2-P6.5): re-export once files are populated.
// pub use key_provider::{EnvKeyProvider, FileKeyProvider, KeyProvider, ProviderError};
// #[cfg(any(test, feature = "test-util"))]
// pub use key_provider::StaticKeyProvider;
// pub use layer::{ ... };
// pub use store::{ ... };
// #[cfg(any(test, feature = "credential-in-memory"))]
// pub use memory::InMemoryStore;
