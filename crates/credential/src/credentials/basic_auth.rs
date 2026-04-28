//! HTTP Basic Auth credential -- static, non-interactive.
//!
//! Resolves a username + password pair into [`IdentityPassword`]. State and
//! Scheme are the same type via [`identity_state!`](crate::identity_state).

use nebula_schema::{Field, FieldValues, HasSchema, Schema, ValidSchema, field_key};

use crate::{
    Credential, CredentialContext, SecretString, contract::plugin_capability_report,
    error::CredentialError, metadata::CredentialMetadata, resolve::ResolveResult,
    scheme::IdentityPassword,
};

/// Typed shape of the `basic_auth` credential setup form.
pub struct BasicAuthInput;

impl HasSchema for BasicAuthInput {
    fn schema() -> ValidSchema {
        Schema::builder()
            .add(
                Field::string(field_key!("username"))
                    .label("Username")
                    .description("Username for HTTP Basic authentication")
                    .required(),
            )
            .add(
                Field::secret(field_key!("password"))
                    .label("Password")
                    .description("Password for HTTP Basic authentication")
                    .required(),
            )
            .build()
            .expect("basic_auth schema is always valid")
    }
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

impl Credential for BasicAuthCredential {
    type Input = BasicAuthInput;
    type Scheme = IdentityPassword;
    type State = IdentityPassword;

    const KEY: &'static str = "basic_auth";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("basic_auth"))
            .name("Basic Auth")
            .description("HTTP Basic authentication (username + password).")
            .schema(Self::schema())
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
            CredentialError::Provider("missing required field 'username'".to_owned())
        })?;
        let password = values.get_string_by_str("password").ok_or_else(|| {
            CredentialError::Provider("missing required field 'password'".to_owned())
        })?;
        let secret = SecretString::new(password.to_owned());
        Ok(ResolveResult::Complete(IdentityPassword::new(
            username, secret,
        )))
    }
}

// Per Tech Spec §15.8 every credential reports its sub-trait surface
// via `plugin_capability_report::Is*`. `BasicAuthCredential` is fully
// static — no capability sub-trait impls — so all five constants are
// `false`.
impl plugin_capability_report::IsInteractive for BasicAuthCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRefreshable for BasicAuthCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRevocable for BasicAuthCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsTestable for BasicAuthCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsDynamic for BasicAuthCredential {
    const VALUE: bool = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_basic_auth() {
        assert_eq!(BasicAuthCredential::KEY, "basic_auth");
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
