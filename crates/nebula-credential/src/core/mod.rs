//! Core types for credential management

// TODO: Phase 5 - Re-enable after updating to new error API
// pub mod adapter;
// pub mod result;
// mod state;
// pub use state::CredentialState;
// pub use nebula_core::CredentialKey;

mod context;
mod error;
mod filter;
mod id;
mod metadata;

pub use context::CredentialContext;
pub use error::{CredentialError, CryptoError, Result, StorageError, ValidationError};
pub use filter::CredentialFilter;
pub use id::CredentialId;
pub use metadata::CredentialMetadata;

// Re-exports from utils
pub use crate::utils::{SecretString, from_unix_timestamp, to_unix_timestamp, unix_now};
