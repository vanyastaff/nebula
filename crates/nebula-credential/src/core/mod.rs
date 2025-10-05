//! Core types for credential management
pub mod adapter;
pub mod result;
mod context;
mod error;
mod metadata;
mod state;
pub use context::CredentialContext;
pub use error::{CredentialError, Result};
pub use metadata::CredentialMetadata;
pub use nebula_core::{CredentialId, CredentialKey};
pub use state::CredentialState;

// Re-exports from utils
pub use crate::utils::{from_unix_timestamp, to_unix_timestamp, unix_now, SecureString};
