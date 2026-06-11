//! ADR-0092 drain shim — deleted in step 8.
//!
//! The three decorator types and their supporting types were relocated to
//! `nebula_credential::store_layer` (Core tier) in ADR-0092 step 3.
//! These re-exports keep the previous `nebula_storage::credential` public
//! surface resolving without changes at every call site.

/// Drain shim — see [`nebula_credential::store_layer::audit`].
pub mod audit {
    pub use nebula_credential::store_layer::audit::*;
}

/// Drain shim — see [`nebula_credential::store_layer::cache`].
pub mod cache {
    pub use nebula_credential::store_layer::cache::*;
}

/// Drain shim — see [`nebula_credential::store_layer::encryption`].
pub mod encryption {
    pub use nebula_credential::store_layer::encryption::*;
}

pub use nebula_credential::store_layer::{
    AuditEvent, AuditLayer, AuditOperation, AuditResult, AuditSink, CacheConfig, CacheLayer,
    CacheStats, EncryptionLayer,
};
