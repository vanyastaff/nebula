//! Core types for credential management

// TODO: Phase 5 - Re-enable after updating to new error API
// pub mod adapter;

pub mod result;
mod state;

mod context;
mod description;
mod error;
mod filter;
mod id;
mod metadata;

pub use context::CredentialContext;
pub use description::CredentialDescription;
pub use error::{
    CredentialError, CryptoError, ManagerError, ManagerResult, Result, StorageError,
    ValidationError,
};
pub use filter::CredentialFilter;
pub use id::{CredentialId, ScopeId};
pub use metadata::CredentialMetadata;
pub use state::CredentialState;

// Re-exports from utils
pub use crate::utils::{SecretString, from_unix_timestamp, to_unix_timestamp, unix_now};
