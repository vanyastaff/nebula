//! OAuth/OIDC HTTP infrastructure for Plane-A identity sign-in.
//!
//! Plane-B integration credentials use the universal credential acquisition
//! protocol; no raw provider authorization/callback ceremony is mounted by the
//! API. This module therefore serves only [`crate::domain::auth`].
//!
mod egress;
mod error;
mod runtime;
#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use error::OAuthFailureCode;
pub use error::OAuthRuntimeBuildError;
pub use runtime::OAuthIdentityRuntime;
#[cfg(test)]
pub(crate) use runtime::OAuthTestProviderProfile;
