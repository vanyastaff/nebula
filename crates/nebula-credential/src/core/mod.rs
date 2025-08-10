//! Core types for credential management

mod id;
mod key;
mod token;
mod secure;
mod error;
mod state;
mod context;
mod metadata;
mod ephemeral;
mod time;

pub use id::CredentialId;
pub use key::{CredentialKey};
pub use token::{AccessToken, TokenType};
pub use secure::SecureString;
pub use error::{CredentialError, Result};
pub use state::CredentialState;
pub use context::CredentialContext;
pub use metadata::CredentialMetadata;
pub use ephemeral::Ephemeral;
pub use time::{to_unix_timestamp, from_unix_timestamp, unix_now};