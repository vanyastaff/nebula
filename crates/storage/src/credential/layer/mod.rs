//! Composable storage layers for [`CredentialStore`](super::CredentialStore).

pub mod audit;
pub mod cache;
pub mod encryption;
pub mod scope;

// TODO(P6.4, P6.5): re-export once files are populated.
