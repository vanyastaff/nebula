//! Composable storage layers for [`CredentialStoreV2`](crate::store_v2::CredentialStoreV2).
//!
//! Layers wrap an inner store to add cross-cutting concerns (encryption,
//! caching, auditing) without modifying the store implementation itself.

pub mod encryption;

pub use encryption::EncryptionLayer;
