//! Closed, secret-free failure vocabulary for Plane-A OAuth.

use std::fmt;

use thiserror::Error;

/// Stable low-cardinality failure codes for Plane-A OAuth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum OAuthFailureCode {
    /// A configured or discovered endpoint violated the fixed URL policy.
    EndpointRejected,
    /// OIDC discovery could not produce a currently usable document.
    DiscoveryUnavailable,
    /// The provider key is not present in the runtime-owned configuration.
    ProviderNotConfigured,
    /// The authorization-code token exchange failed.
    TokenExchangeFailed,
    /// The primary userinfo request or response failed.
    UserinfoFailed,
    /// The verified-email request or response failed.
    VerifiedEmailFailed,
    /// The provider identity is valid but has no email evidence that meets
    /// Nebula's account-provisioning policy.
    VerifiedEmailUnavailable,
    /// A successful provider response omitted or malformed a required field.
    ProviderResponseInvalid,
    /// The callback redirect URI no longer matches the value stored at start.
    RedirectUriMismatch,
    /// The end-to-end OAuth completion deadline elapsed.
    CompletionTimeout,
}

impl OAuthFailureCode {
    /// Stable diagnostic code suitable for low-cardinality logs and metrics.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::EndpointRejected => "endpoint_rejected",
            Self::DiscoveryUnavailable => "discovery_unavailable",
            Self::ProviderNotConfigured => "provider_not_configured",
            Self::TokenExchangeFailed => "token_exchange_failed",
            Self::UserinfoFailed => "userinfo_failed",
            Self::VerifiedEmailFailed => "verified_email_failed",
            Self::VerifiedEmailUnavailable => "verified_email_unavailable",
            Self::ProviderResponseInvalid => "provider_response_invalid",
            Self::RedirectUriMismatch => "redirect_uri_mismatch",
            Self::CompletionTimeout => "completion_timeout",
        }
    }
}

impl fmt::Display for OAuthFailureCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Failure to construct the fixed Plane-A OAuth runtime policy.
///
/// The underlying HTTP-builder error is deliberately discarded because it
/// may retain endpoint or proxy material in its source chain.
#[derive(Debug, Error)]
#[error("OAuth identity runtime initialization failed")]
pub struct OAuthRuntimeBuildError {
    _private: (),
}

impl OAuthRuntimeBuildError {
    pub(crate) const fn new() -> Self {
        Self { _private: () }
    }
}
