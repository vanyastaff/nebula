//! Composable storage layers for
//! [`CredentialPersistence`](nebula_storage_port::CredentialPersistence).
//!
//! Credential owner scope is part of every port operation through mandatory
//! [`CredentialSelector`](nebula_storage_port::CredentialSelector) /
//! [`CredentialOwner`](nebula_storage_port::CredentialOwner) values. There is
//! no metadata-keyed scope decorator or optional admin owner. These layers
//! preserve the selector while composing audit, encryption, and caching over
//! the backend; each cache key and forwarded operation remains owner-bound.

pub mod audit;
pub mod cache;
pub mod encryption;

pub use audit::AuditLayer;
pub use cache::{CacheConfig, CacheLayer, CacheStats};
pub use encryption::EncryptionLayer;
pub use nebula_credential::{AuditEvent, AuditOperation, AuditResult, AuditSink};
