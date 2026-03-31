//! Core types for credential management

mod context;
mod description;
mod error;
mod metadata;
mod snapshot;

pub use context::CredentialContext;
pub use description::CredentialDescription;
pub use error::{
    CredentialError, CryptoError, ManagerError, ManagerResult, RefreshErrorKind, ResolutionStage,
    Result, RetryAdvice, StorageError, ValidationError,
};
pub use metadata::CredentialMetadata;
pub use snapshot::CredentialSnapshot;

/// Re-export for instance identification (UUID-backed, like ResourceId).
pub use nebula_core::CredentialId;

// Re-exports from utils
pub use crate::utils::SecretString;
