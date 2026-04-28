//! API Key credential -- static, non-interactive.
//!
//! The simplest credential type: a single secret token resolved from user
//! input. State and Scheme are the same type ([`SecretToken`]) via
//! [`identity_state!`](crate::identity_state).

use nebula_schema::{Field, FieldValues, HasSchema, Schema, ValidSchema, field_key};

use crate::{
    Credential, CredentialContext, SecretString, contract::plugin_capability_report,
    error::CredentialError, metadata::CredentialMetadata, resolve::ResolveResult,
    scheme::SecretToken,
};

/// Typed shape of the `api_key` credential setup form.
///
/// Used solely as the [`Credential::Input`] — the value is carried through
/// the resolver as [`FieldValues`] for now; this struct exists to advertise
/// the canonical schema via [`HasSchema`].
pub struct ApiKeyInput;

impl HasSchema for ApiKeyInput {
    fn schema() -> ValidSchema {
        Schema::builder()
            .add(
                Field::string(field_key!("server"))
                    .label("Server URL")
                    .description("Base URL of the service (e.g. https://api.example.com)")
                    .placeholder("https://api.example.com"),
            )
            .add(
                Field::secret(field_key!("api_key"))
                    .label("API Key")
                    .description("Secret API token or personal access token")
                    .required(),
            )
            .build()
            .expect("api_key schema is always valid")
    }
}

/// API Key credential -- resolves a single token into a [`SecretToken`].
///
/// - **Non-interactive:** resolves in one step from user input. Per §15.4 sub-trait split, this
///   credential does *not* implement [`Interactive`](crate::Interactive) — the absence of the
///   sub-trait impl is the type-level declaration of non-interactive.
/// - **Non-refreshable:** static tokens have no expiry. Does not implement
///   [`Refreshable`](crate::Refreshable).
/// - **Identity projection:** stored state is the scheme itself.
///
/// # Examples
///
/// ```ignore
/// use nebula_credential::credentials::ApiKeyCredential;
/// use nebula_credential::Credential;
///
/// assert_eq!(ApiKeyCredential::KEY, "api_key");
/// ```
pub struct ApiKeyCredential;

impl Credential for ApiKeyCredential {
    type Input = ApiKeyInput;
    type Scheme = SecretToken;
    type State = SecretToken;

    const KEY: &'static str = "api_key";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("api_key"))
            .name("API Key")
            .description("Static API key or bearer token for HTTP APIs.")
            .schema(Self::schema())
            .pattern(crate::AuthPattern::SecretToken)
            .icon("key")
            .build()
            .expect("api_key metadata is valid")
    }

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SecretToken, ()>, CredentialError> {
        let token = values.get_string_by_str("api_key").ok_or_else(|| {
            CredentialError::Provider("missing required field 'api_key'".to_owned())
        })?;
        let secret = SecretString::new(token.to_owned());
        Ok(ResolveResult::Complete(SecretToken::new(secret)))
    }
}

// Per Tech Spec §15.8 every credential reports its sub-trait surface
// via `plugin_capability_report::Is*`. `ApiKeyCredential` is fully
// static — no capability sub-trait impls — so all five constants are
// `false`. `CredentialRegistry::register` reads these to compute the
// `Capabilities` bitflag set.
impl plugin_capability_report::IsInteractive for ApiKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRefreshable for ApiKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRevocable for ApiKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsTestable for ApiKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsDynamic for ApiKeyCredential {
    const VALUE: bool = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_api_key() {
        assert_eq!(ApiKeyCredential::KEY, "api_key");
    }

    // Capability membership checks moved to compile-time: the absence
    // of `impl Interactive | Refreshable | Revocable | Testable | Dynamic`
    // for `ApiKeyCredential` is the type-level statement that this
    // credential is static. Probe 4 (compile_fail_engine_dispatch_capability)
    // pins this guarantee at the engine dispatch site.

    #[test]
    fn project_returns_clone_of_state() {
        let token = SecretToken::new(SecretString::new("test-token"));
        let projected = ApiKeyCredential::project(&token);
        let original = token.token().expose_secret().to_owned();
        let cloned = projected.token().expose_secret().to_owned();
        assert_eq!(original, cloned);
    }

    #[tokio::test]
    async fn resolve_extracts_api_key_field() {
        let mut values = FieldValues::new();
        values
            .try_set_raw("api_key", serde_json::Value::String("sk-secret-123".into()))
            .expect("test-only known-good key");
        let ctx = CredentialContext::for_test("test-user");
        let result = ApiKeyCredential::resolve(&values, &ctx).await.unwrap();
        match result {
            ResolveResult::Complete(token) => {
                let exposed = token.token().expose_secret().to_owned();
                assert_eq!(exposed, "sk-secret-123");
            },
            _ => panic!("expected Complete variant"),
        }
    }

    #[tokio::test]
    async fn resolve_returns_error_on_missing_field() {
        let values = FieldValues::new();
        let ctx = CredentialContext::for_test("test-user");
        let result = ApiKeyCredential::resolve(&values, &ctx).await;
        assert!(result.is_err());
    }

    #[test]
    fn parameters_contains_server_and_api_key() {
        let params = ApiKeyCredential::schema();
        assert!(params.fields().iter().any(|f| f.key().as_str() == "server"));
        assert!(
            params
                .fields()
                .iter()
                .any(|f| f.key().as_str() == "api_key")
        );
        assert_eq!(params.fields().len(), 2);
    }

    #[test]
    fn server_is_optional() {
        let params = ApiKeyCredential::schema();
        let server = params
            .fields()
            .iter()
            .find(|f| f.key().as_str() == "server")
            .unwrap();
        assert!(!matches!(
            server.required(),
            nebula_schema::RequiredMode::Always
        ));
    }
}
