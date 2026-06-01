//! HTTP Basic Auth credential -- static, non-interactive.
//!
//! Resolves a username + password pair into [`IdentityPassword`]. State and
//! Scheme are the same type via [`identity_state!`](crate::identity_state).

use nebula_schema::{FieldValues, Schema};
use serde::Deserialize;

use crate::{
    CredentialContext, SecretString,
    error::{CredentialError, ProviderErrorContext, ProviderErrorKind, SecretFreeMessage},
    metadata::CredentialMetadata,
    resolve::ResolveResult,
    scheme::IdentityPassword,
};

/// Typed shape of the `basic_auth` credential setup form (Phase 5 — replaces
/// the legacy `BasicAuthInput`).
///
/// `#[derive(Schema)]` provides the `HasSchema` impl read via
/// `nebula_schema::schema_of::<Self::Properties>()` (schema-of properties). The
/// plaintext password lives
/// in a `String` here for schema derivation and is wrapped into
/// [`SecretString`] inside [`Credential::resolve`](crate::Credential::resolve)
/// before it leaves the resolver.
#[derive(Schema, Deserialize, Default)]
pub struct BasicAuthProperties {
    /// Username for HTTP Basic authentication.
    #[field(label = "Username")]
    #[validate(required)]
    pub username: String,
    /// Password for HTTP Basic authentication.
    #[field(secret, label = "Password")]
    #[validate(required)]
    pub password: String,
}

/// HTTP Basic Auth credential -- resolves username + password into
/// [`IdentityPassword`].
///
/// - **Non-interactive:** resolves in one step from user input. Per §15.4 sub-trait split, this
///   credential does *not* implement [`Interactive`](crate::Interactive).
/// - **Non-refreshable:** static credentials have no expiry. Does not implement
///   [`Refreshable`](crate::Refreshable).
/// - **Identity projection:** stored state is the scheme itself.
pub struct BasicAuthCredential;

// ADR-0088 D1: one `impl` block declares the whole credential. `#[credential]`
// sees only `project` + `resolve` (no capability methods) and emits the
// `Credential` impl, five all-`false` capability-report consts, and a
// `StaticSecret` `CredentialLifecycle` policy — matching the absent capability
// sub-traits.
#[nebula_credential::credential(key = "basic_auth", category = StaticSecret)]
impl BasicAuthCredential {
    type Properties = BasicAuthProperties;
    type Scheme = IdentityPassword;
    type State = IdentityPassword;

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("basic_auth"))
            .name("Basic Auth")
            .description("HTTP Basic authentication (username + password).")
            .schema(nebula_schema::schema_of::<Self::Properties>())
            .pattern(crate::AuthPattern::IdentityPassword)
            .icon("lock")
            .build()
            .expect("basic_auth metadata is valid")
    }

    fn project(state: &IdentityPassword) -> IdentityPassword {
        state.clone()
    }

    async fn resolve(
        values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<IdentityPassword, ()>, CredentialError> {
        let username = values.get_string_by_str("username").ok_or_else(|| {
            CredentialError::Provider(Box::new(ProviderErrorContext::new(
                ProviderErrorKind::Schema,
                SecretFreeMessage::new("missing required field 'username'"),
            )))
        })?;
        let password = values.get_string_by_str("password").ok_or_else(|| {
            CredentialError::Provider(Box::new(ProviderErrorContext::new(
                ProviderErrorKind::Schema,
                SecretFreeMessage::new("missing required field 'password'"),
            )))
        })?;
        let secret = SecretString::new(password.to_owned());
        Ok(ResolveResult::Complete(IdentityPassword::new(
            username, secret,
        )))
    }
}

#[cfg(test)]
mod tests {
    // `Credential` (for `KEY`) and `CredentialLifecycle` (for `policy`) are
    // only referenced by the tests now that `#[credential]` generates the
    // trait impls via absolute paths.
    use crate::{Credential, CredentialLifecycle};

    use super::*;

    #[test]
    fn key_is_basic_auth() {
        assert_eq!(BasicAuthCredential::KEY, "basic_auth");
    }

    #[test]
    fn lifecycle_policy_is_static() {
        let auth = IdentityPassword::new("u", SecretString::new("p"));
        let p = BasicAuthCredential::policy(&auth);
        assert_eq!(p.category, crate::CredentialCategory::StaticSecret);
        assert!(!p.is_expiring());
        assert!(!p.is_auto_renewable());
    }

    // Capability membership checks moved to compile-time: the absence
    // of `impl Interactive | Refreshable | Revocable | Testable | Dynamic`
    // for `BasicAuthCredential` is the type-level statement that this
    // credential is static. Probe 4 (compile_fail_engine_dispatch_capability)
    // pins this guarantee at the engine dispatch site.

    #[test]
    fn project_returns_clone_of_state() {
        let auth = IdentityPassword::new("admin", SecretString::new("s3cret"));
        let projected = BasicAuthCredential::project(&auth);
        assert_eq!(projected.identity(), "admin");
        let original = auth.password().expose_secret().to_owned();
        let cloned = projected.password().expose_secret().to_owned();
        assert_eq!(original, cloned);
    }

    #[tokio::test]
    async fn resolve_extracts_username_and_password() {
        let mut values = FieldValues::new();
        values
            .try_set_raw("username", serde_json::Value::String("alice".into()))
            .expect("test-only known-good key");
        values
            .try_set_raw("password", serde_json::Value::String("p@ssw0rd".into()))
            .expect("test-only known-good key");
        let ctx = CredentialContext::for_test("test-user");
        let result = BasicAuthCredential::resolve(&values, &ctx).await.unwrap();
        match result {
            ResolveResult::Complete(auth) => {
                assert_eq!(auth.identity(), "alice");
                let pw = auth.password().expose_secret().to_owned();
                assert_eq!(pw, "p@ssw0rd");
            },
            _ => panic!("expected Complete variant"),
        }
    }

    #[tokio::test]
    async fn resolve_returns_error_on_missing_username() {
        let mut values = FieldValues::new();
        values
            .try_set_raw("password", serde_json::Value::String("secret".into()))
            .expect("test-only known-good key");
        let ctx = CredentialContext::for_test("test-user");
        let result = BasicAuthCredential::resolve(&values, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn resolve_returns_error_on_missing_password() {
        let mut values = FieldValues::new();
        values
            .try_set_raw("username", serde_json::Value::String("alice".into()))
            .expect("test-only known-good key");
        let ctx = CredentialContext::for_test("test-user");
        let result = BasicAuthCredential::resolve(&values, &ctx).await;
        assert!(result.is_err());
    }
}
