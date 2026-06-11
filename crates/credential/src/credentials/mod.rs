//! Built-in credential type implementations.
//!
//! Each type implements [`Credential`](crate::Credential) using
//! the v2 unified trait. Static credentials (API key, basic auth) use
//! [`identity_state!`](crate::identity_state) so that `State = Scheme`.
//!
//! Per Phase 5 of the M6 dependency redesign each credential ships a
//! `<Name>Properties` companion struct (`#[derive(Schema, Deserialize)]`)
//! that owns the setup-form schema; `Credential::Properties` points at
//! it and the engine reads the schema via
//! `nebula_schema::schema_of::<C::Properties>()` (schema-of properties).

pub mod api_key;
pub mod basic_auth;
pub mod bearer_token;
pub mod oauth2;
pub mod oauth2_config;
pub mod shared_key;
pub mod signing_key;

pub use api_key::{ApiKeyCredential, ApiKeyProperties};
pub use basic_auth::{BasicAuthCredential, BasicAuthProperties};
pub use bearer_token::{BearerTokenCredential, BearerTokenProperties};
pub use oauth2::{OAuth2Credential, OAuth2Pending, OAuth2Properties, OAuth2State};
pub use oauth2_config::{
    AuthCodeBuilder, ClientCredentialsBuilder, DeviceCodeBuilder, GrantType, OAuth2Config,
    PkceMethod,
};
pub use shared_key::{SharedKeyCredential, SharedKeyProperties};
pub use signing_key::{SigningKeyCredential, SigningKeyProperties};

// AuthStyle lives in scheme::oauth2 as of Task 8 (M12.3 carve-out prep).
// Re-exported here so `credentials::AuthStyle` still resolves; deprecated.
#[deprecated(
    since = "0.1.0",
    note = "use `nebula_credential::scheme::oauth2::AuthStyle` or the crate-root re-export `nebula_credential::AuthStyle`"
)]
pub use crate::scheme::oauth2::AuthStyle;

/// Register every first-party reference credential into `registry`.
///
/// Fail-closed on duplicate KEY (Tech Spec §15.6): if a KEY is already
/// present the second registration is **rejected** with
/// [`crate::RegisterError::DuplicateKey`], the first registration remains
/// authoritative, and `registry` is left unchanged for the rejected entry.
/// This is not silent "first-wins" — the collision surfaces as an error
/// the caller must handle.
///
/// # Errors
///
/// Returns [`crate::RegisterError::DuplicateKey`] if any reference KEY is
/// already present in `registry` (e.g. a plugin shipped a colliding KEY).
pub fn register_builtins(
    registry: &mut crate::CredentialRegistry,
) -> Result<(), crate::RegisterError> {
    let crate_name = env!("CARGO_CRATE_NAME");
    registry.register(BearerTokenCredential, crate_name)?;
    registry.register(SharedKeyCredential, crate_name)?;
    registry.register(SigningKeyCredential, crate_name)?;
    Ok(())
}
