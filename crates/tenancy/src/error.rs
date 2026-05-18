//! Tenancy resolution errors.
//!
//! These surface only at the composition root, when a [`Principal`] is
//! turned into a [`Scope`]. They are deliberately coarse: a caller must
//! never learn *why* resolution failed in a way that lets it probe the
//! tenant graph (the same existence-non-disclosure rule the scoped
//! decorators enforce for row access — spec §6.1).
//!
//! [`Principal`]: crate::Principal
//! [`Scope`]: nebula_storage_port::Scope

/// Failure resolving a [`Principal`] to a [`Scope`].
///
/// [`Principal`]: crate::Principal
/// [`Scope`]: nebula_storage_port::Scope
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TenancyError {
    /// The principal carries no workspace binding, but the requested
    /// operation is workspace-scoped. Returned instead of silently
    /// widening to an org-only scope (fail-closed).
    #[error("principal has no workspace binding for a workspace-scoped operation")]
    MissingWorkspace,

    /// The principal is not authorized for the tenant it presented
    /// (org/workspace mismatch against its grants). Coarse on purpose —
    /// it never reveals which half mismatched.
    #[error("principal is not authorized for the requested tenant")]
    Unauthorized,
}
