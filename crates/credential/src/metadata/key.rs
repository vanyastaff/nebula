//! Newtype for credential type keys.
//!
//! `CredentialKey` is the validated, normalized string key for a credential type
//! (e.g., `"github_oauth2"`, `"api_key"`). Defined in [`nebula_core::CredentialKey`]
//! via `domain_key::key_type!` with compile-time validation.
//! Re-exported here for discoverability.

pub use nebula_core::CredentialKey;
// Re-export the compile-time `credential_key!` macro from core.
pub use nebula_core::credential_key;
