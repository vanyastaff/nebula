//! Composable storage layers for [`CredentialStore`](crate::credential_store::CredentialStore).
//!
//! Layers wrap an inner store to add cross-cutting concerns (encryption,
//! caching, auditing) without modifying the store implementation itself.

pub mod encryption;

pub use encryption::EncryptionLayer;
