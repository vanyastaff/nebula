//! Credential contract surface — the Credential trait and its associated types.
//!
//! Action / resource / plugin authors bind to these types; they never touch
//! persistence, orchestration, or transport concerns.
//!
//! # Canonical import paths
//!
//! This submodule is `pub` for escape hatches. Prefer flat root re-exports:
//! `use nebula_credential::Credential;` (not `nebula_credential::contract::Credential`).

mod any;
mod credential;
mod pending;
mod state;
mod static_protocol;

pub use any::AnyCredential;
pub use credential::Credential;
pub use pending::{NoPendingState, PendingState, PendingToken};
pub use state::CredentialState;
pub use static_protocol::StaticProtocol;
