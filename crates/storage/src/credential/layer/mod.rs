//! Composable storage layers for [`CredentialStore`](nebula_credential::CredentialStore).
//!
//! The multi-tenant **scope** layer was re-homed to the tenancy security
//! boundary (`nebula_tenancy::CredentialScopeLayer` /
//! `CredentialScopeResolver`, spec §8) — scope *policy* does not belong in
//! the storage adapter. These layers (`EncryptionLayer`, `CacheLayer`,
//! `AuditLayer`) stay here and re-compose **on top** of the tenancy scope
//! layer at the composition root; the ADR-0029 fail-closed audit +
//! zeroize-on-drop invariants are unaffected by the move (layer order
//! `ScopeLayer → AuditLayer → EncryptionLayer → CacheLayer → Backend` is
//! preserved by the composition root, not by this module).

pub mod audit;
pub mod cache;
pub mod encryption;

pub use audit::{AuditEvent, AuditLayer, AuditOperation, AuditResult, AuditSink};
pub use cache::{CacheConfig, CacheLayer, CacheStats};
pub use encryption::EncryptionLayer;
