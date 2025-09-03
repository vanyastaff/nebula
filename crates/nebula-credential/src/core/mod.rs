//! Core types for credential management

mod context;
mod ephemeral;
mod error;
mod id;
mod key;
mod metadata;
mod secure;
mod state;
mod time;
mod token;

pub use context::CredentialContext;
pub use ephemeral::Ephemeral;
pub use error::{CredentialError, Result};
pub use id::CredentialId;
pub use key::CredentialKey;
pub use metadata::CredentialMetadata;
pub use secure::SecureString;
pub use state::CredentialState;
pub use time::{from_unix_timestamp, to_unix_timestamp, unix_now};
pub use token::{AccessToken, TokenType};
