//! API Key credential -- static, non-interactive.
//!
//! The simplest credential type: a single secret token resolved from user
//! input. State and Scheme are the same type ([`SecretToken`]) via
//! [`identity_state!`](crate::identity_state).

use nebula_schema::Schema;
use serde::Deserialize;

use crate::{
    CredentialContext, SecretString, error::CredentialError, metadata::CredentialMetadataDraft,
    resolve::StaticResolveResult, scheme::SecretToken,
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
/// The `#[field(secret)]` declaration requires the field type to implement
/// `nebula_schema::SecretInput`; credential [`SecretString`]
/// satisfies that contract and keeps plaintext in zeroizing memory after the
/// trusted typed decode.
#[derive(Schema, Deserialize)]
pub struct ApiKeyProperties {
    /// Optional base URL of the service (e.g. `https://api.example.com`).
    #[field(label = "Server URL", placeholder = "https://api.example.com")]
    pub server: Option<String>,
    /// Secret API token or personal access token.
    #[field(secret, label = "API Key")]
    #[validate(required)]
    pub api_key: SecretString,
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
/// ```
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

    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("api_key"),
            crate::metadata_name!("API Key"),
            "Static API key or bearer token for HTTP APIs.",
        )
        .with_icon(nebula_metadata::Icon::inline("key"))
    }

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        properties: &ApiKeyProperties,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
        Ok(StaticResolveResult::Complete(SecretToken::new(
            properties.api_key.clone(),
        )))
    }
}

#[cfg(test)]
mod tests {
    // `Credential` (for `KEY` / `Properties`) and `CredentialLifecycle` (for
    // `policy`) are only referenced by the tests now that `#[credential]`
    // generates the trait impls via absolute paths.
    use crate::{Credential, CredentialLifecycle, credentials::resolve_properties};

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
        let properties = resolve_properties::<ApiKeyCredential>(serde_json::json!({
            "api_key": "sk-secret-123"
        }))
        .unwrap();
        assert_eq!(properties.api_key.expose_secret(), "sk-secret-123");
        let ctx = CredentialContext::for_owner("test-user");
        let result = ApiKeyCredential::resolve(&properties, &ctx).await.unwrap();
        match result {
            StaticResolveResult::Complete(token) => {
                let exposed = token.token().expose_secret().to_owned();
                assert_eq!(exposed, "sk-secret-123");
            },
            _ => panic!("expected Complete variant"),
        }
    }

    #[test]
    fn schema_rejects_missing_api_key_before_resolve() {
        let Err(report) = resolve_properties::<ApiKeyCredential>(serde_json::json!({})) else {
            panic!("missing API key must fail schema validation");
        };
        assert!(
            report.errors().any(|error| {
                error.code() == "required" && error.path().to_string() == "/api_key"
            })
        );
    }

    #[test]
    fn parameters_contains_server_and_api_key() {
        let params = nebula_schema::schema_of::<<ApiKeyCredential as Credential>::Properties>()
            .expect("valid API key schema");
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
        let params = nebula_schema::schema_of::<<ApiKeyCredential as Credential>::Properties>()
            .expect("valid API key schema");
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
