//! Consumer-facing accessor surface.
//!
//! Action / resource code imports from here to obtain credentials via
//! [`CredentialAccessor`]. The surface is deliberately split between two
//! concerns:
//!
//! - [`CredentialHandle`] is the typed RAII lease returned by resolution — it owns the projected
//!   scheme for the duration of a use and drops cleanly (zeroize-on-drop applies to any secret
//!   material it borrows).
//! - [`CredentialContext`] carries execution-scope state during `resolve` — cancellation, logger
//!   references, and the resolver handle itself — so that the accessor can honor cooperative
//!   cancellation and emit structured events tied to the caller's span.
//!
//! [`CredentialAccessError`] lives alongside the trait rather than in
//! [`crate::error`] because it is the consumer-facing failure type for a
//! consumer-facing capability — keeping the trait and its error adjacent
//! means action/resource authors read one module instead of jumping to
//! `crate::error` for every `?` on a resolve call.
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
