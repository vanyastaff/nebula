//! SAML protocol stub — planned for Phase 7.
//!
//! Full implementation requires `samael` or a similar SAML library.
//! This module provides configuration types for forward-compatibility
//! with `#[credential(extends = SamlProtocol)]` macro usage.

use serde::{Deserialize, Serialize};

/// SAML request/response binding type (SAMLBind spec section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SamlBinding {
    /// HTTP POST binding — assertion delivered via HTML form POST.
    #[default]
    HttpPost,
    /// HTTP Redirect binding — assertion carried in query string.
    HttpRedirect,
}

/// SAML identity provider configuration.
///
/// Used with `#[saml(...)]` macro attribute once `SamlProtocol` is implemented.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SamlConfig {
    /// SSO URL of the Identity Provider.
    pub idp_sso_url: String,
    /// Entity ID of the Identity Provider.
    pub idp_entity_id: String,
    /// PEM-encoded X.509 certificate of the Identity Provider.
    pub idp_certificate: String,
    /// Preferred binding for AuthnRequest.
    pub binding: SamlBinding,
    /// Whether to sign outgoing AuthnRequests.
    pub sign_requests: bool,
}
