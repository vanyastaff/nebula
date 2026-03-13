//! Core types for credential management

// TODO: Phase 5 - Re-enable after updating to new error API
// pub mod adapter;

pub mod result;
mod snapshot;
mod state;

mod context;
mod description;
mod error;
mod filter;
mod metadata;
pub mod reference;
mod status;

pub use context::CredentialContext;
pub use description::CredentialDescription;
pub use error::{
    CredentialError, CryptoError, ManagerError, ManagerResult, Result, StorageError,
    ValidationError,
};
pub use filter::CredentialFilter;
pub use snapshot::CredentialSnapshot;

pub use metadata::CredentialMetadata;
/// Re-export for instance identification (UUID-backed, like ResourceId).
pub use nebula_core::CredentialId;
pub use reference::{CredentialProvider, CredentialRef};
pub use state::CredentialState;
pub use status::{CredentialStatus, status_from_metadata};

// Re-exports from utils
pub use crate::utils::{SecretString, from_unix_timestamp, to_unix_timestamp, unix_now};
