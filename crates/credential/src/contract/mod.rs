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
/// Capability bitflag set + registration-time detection per Tech Spec §15.8.
pub mod capability_report;
mod credential;
mod dynamic;
mod interactive;
mod pending;
mod refreshable;
/// KEY-keyed `CredentialRegistry` + `RegisterError` (Tech Spec §15.6 fatal duplicate-KEY).
pub mod registry;
/// Resolve result types: interaction, refresh, test.
pub mod resolve;
mod revocable;
mod state;
mod static_protocol;
mod testable;

pub use any::AnyCredential;
pub use capability_report::{Capabilities, compute_capabilities, plugin_capability_report};
pub use credential::Credential;
pub use dynamic::Dynamic;
pub use interactive::Interactive;
pub use pending::{NoPendingState, PendingState, PendingToken};
pub use refreshable::Refreshable;
pub use registry::{CredentialRegistry, RegisterError};
pub use resolve::{
    DisplayData, InteractionRequest, RefreshOutcome, RefreshPolicy, ResolveResult,
    StaticResolveResult, TestResult, UserInput,
};
pub use revocable::Revocable;
pub use state::CredentialState;
pub use static_protocol::StaticProtocol;
pub use testable::Testable;
