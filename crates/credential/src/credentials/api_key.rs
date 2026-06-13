//! API Key credential -- static, non-interactive.
//!
//! The simplest credential type: a single secret token resolved from user
//! input. State and Scheme are the same type ([`SecretToken`]) via
//! [`identity_state!`](crate::identity_state).

use nebula_schema::{FieldValues, Schema};
use serde::Deserialize;

use crate::{
    CredentialContext, SecretString,
    error::{CredentialError, ProviderErrorContext, ProviderErrorKind, SecretFreeMessage},
    metadata::CredentialMetadata,
    resolve::ResolveResult,
    scheme::SecretToken,
};

/// Typed shape of the `api_key` credential setup form (Phase 5 — replaces
/// the legacy `ApiKeyInput`).
///
/// The struct is purely the schema-bearing companion: `#[derive(Schema)]`
/// emits the `HasSchema` impl read via
/// `nebula_schema::schema_of::<Self::Properties>()` (schema-of properties). The
/// actual auth material conversion to
/// [`SecretToken`] happens in [`Credential::resolve`](crate::Credential::resolve).
///
/// The plaintext lives in a `String` here rather than `SecretString` so
/// that the universal `#[derive(Schema)]` field-type inference applies
/// (`SecretString` would land in the `UserDefined` bucket and require a
/// hand-rolled `HasSchema` impl); the `#[field(secret)]` flag tells the
/// schema layer to render this as a redacted/secret form field, while
/// `resolve` immediately wraps the value in `SecretString` for storage.
#[derive(Schema, Deserialize, Default)]
pub struct ApiKeyProperties {
    /// Optional base URL of the service (e.g. `https://api.example.com`).
    #[field(label = "Server URL", placeholder = "https://api.example.com")]
    pub server: Option<String>,
    /// Secret API token or personal access token.
    #[field(secret, label = "API Key")]
    #[validate(required)]
    pub api_key: String,
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

// ADR-0088 D1: the whole credential surface is declared in one `impl` block.
// `#[credential]` reads which methods are present — here only `project` +
// `resolve`, with no capability methods — and emits the `Credential` impl, the
// five all-`false` capability-report consts, and a `CredentialLifecycle` whose
// synthesized policy is static (no refresh, no provider-side revoke), matching
// the absent capability sub-traits.
#[nebula_credential::credential(key = "api_key")]
impl ApiKeyCredential {
    type Properties = ApiKeyProperties;
    type Scheme = SecretToken;
    type State = SecretToken;

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("api_key"))
            .name("API Key")
            .description("Static API key or bearer token for HTTP APIs.")
            .schema(nebula_schema::schema_of::<Self::Properties>())
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
            CredentialError::Provider(Box::new(ProviderErrorContext::new(
                ProviderErrorKind::Schema,
                SecretFreeMessage::new("missing required field 'api_key'"),
            )))
        })?;
        let secret = SecretString::new(token.to_owned());
        Ok(ResolveResult::Complete(SecretToken::new(secret)))
    }
}

#[cfg(test)]
mod tests {
    // `Credential` (for `KEY` / `Properties`) and `CredentialLifecycle` (for
    // `policy`) are only referenced by the tests now that `#[credential]`
    // generates the trait impls via absolute paths.
    use crate::{Credential, CredentialLifecycle};

    use super::*;

    #[test]
    fn key_is_api_key() {
        assert_eq!(ApiKeyCredential::KEY, "api_key");
    }

    #[test]
    fn lifecycle_policy_is_static() {
        let token = SecretToken::new(SecretString::new("x"));
        let p = ApiKeyCredential::policy(&token);
        assert!(!p.is_expiring());
        assert!(!p.is_auto_renewable());
        assert_eq!(p.refresh, crate::RefreshStrategy::Static);
        assert_eq!(p.revoke, crate::RevokeStrategy::None);
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
        let params = nebula_schema::schema_of::<<ApiKeyCredential as Credential>::Properties>();
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
        let params = nebula_schema::schema_of::<<ApiKeyCredential as Credential>::Properties>();
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
