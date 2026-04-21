//! Consumer-facing accessor surface.
//!
//! Action / resource code imports from here to obtain credentials via
//! [`CredentialAccessor`]. [`CredentialHandle`] is the typed handle returned
//! by resolution. [`CredentialContext`] carries execution context during
//! resolve.
//!
//! # Canonical import paths
//!
//! This submodule is `pub` for escape hatches. Prefer flat root re-exports:
//! `use nebula_credential::CredentialAccessor;`
//! (not `nebula_credential::accessor::CredentialAccessor`).

mod access_error;
#[allow(clippy::module_inception)]
mod accessor;
mod context;
mod handle;

pub use access_error::CredentialAccessError;
pub use accessor::{
    CredentialAccessor, NoopCredentialAccessor, ScopedCredentialAccessor,
    default_credential_accessor,
};
pub use context::{CredentialContext, CredentialResolverRef};
pub use handle::CredentialHandle;
