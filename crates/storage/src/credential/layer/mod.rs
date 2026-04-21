//! Composable storage layers for [`CredentialStore`](nebula_credential::CredentialStore).

pub mod audit;
pub mod cache;
pub mod encryption;
pub mod scope;

pub use audit::{AuditEvent, AuditLayer, AuditOperation, AuditResult, AuditSink};
pub use cache::{CacheConfig, CacheLayer, CacheStats};
pub use encryption::EncryptionLayer;
pub use scope::{ScopeLayer, ScopeResolver};
