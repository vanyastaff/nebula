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
mod dynamic;
mod interactive;
mod pending;
mod refreshable;
/// Resolve result types: interaction, refresh, test.
pub mod resolve;
mod revocable;
mod state;
mod static_protocol;
mod testable;

pub use any::AnyCredential;
pub use credential::Credential;
pub use dynamic::Dynamic;
pub use interactive::Interactive;
pub use pending::{NoPendingState, PendingState, PendingToken};
pub use refreshable::Refreshable;
pub use resolve::{
    DisplayData, InteractionRequest, RefreshOutcome, RefreshPolicy, ResolveResult,
    StaticResolveResult, TestResult, UserInput,
};
pub use revocable::Revocable;
pub use state::CredentialState;
pub use static_protocol::StaticProtocol;
pub use testable::Testable;
