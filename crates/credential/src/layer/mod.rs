//! Composable storage layers for [`CredentialStore`](crate::store::CredentialStore).
//!
//! Layers wrap an inner store to add cross-cutting concerns (encryption,
//! caching, auditing) without modifying the store implementation itself.

pub mod audit;
pub mod cache;
pub mod encryption;
pub mod key_provider;
pub mod scope;

pub use audit::{AuditEvent, AuditLayer, AuditOperation, AuditResult, AuditSink};
pub use cache::{CacheConfig, CacheLayer, CacheStats};
pub use encryption::EncryptionLayer;
#[cfg(any(test, feature = "test-util"))]
pub use key_provider::StaticKeyProvider;
pub use key_provider::{EnvKeyProvider, FileKeyProvider, KeyProvider, ProviderError};
pub use scope::{ScopeLayer, ScopeResolver};
