//! Opaque bearer-token credential — static, non-interactive.
//!
//! Resolves a single secret token into [`SecretToken`]. `State = Scheme`
//! (identity projection). Reference impl mirroring the contract crate's
//! `BasicAuthCredential` shape.

use nebula_schema::Schema;
use serde::Deserialize;

use crate::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialMetadataDraft,
    SecretString, contract::plugin_capability_report, contract::resolve::StaticResolveResult,
    scheme::SecretToken,
};

/// Setup-form shape for the `bearer_token` credential.
#[derive(Schema, Deserialize)]
pub struct BearerTokenProperties {
    /// The opaque bearer token (API key, PAT, session token).
    #[field(secret, label = "Token")]
    #[validate(required)]
    pub token: SecretString,
}

/// Static opaque-token credential. Projects stored state (the token)
/// directly as the auth scheme.
pub struct BearerTokenCredential;

impl Credential for BearerTokenCredential {
    type Properties = BearerTokenProperties;
    type Scheme = SecretToken;
    type State = SecretToken;

    const KEY: &'static str = "bearer_token";

    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("bearer_token"),
            crate::metadata_name!("Bearer Token"),
            "Opaque bearer token (API key, PAT, session token).",
            AuthPattern::SecretToken,
        )
        .with_icon(nebula_metadata::Icon::inline("key"))
    }

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        properties: &BearerTokenProperties,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
        Ok(StaticResolveResult::Complete(SecretToken::new(
            properties.token.clone(),
        )))
    }
}

impl plugin_capability_report::IsInteractive for BearerTokenCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRefreshable for BearerTokenCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRevocable for BearerTokenCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsTestable for BearerTokenCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsDynamic for BearerTokenCredential {
    const VALUE: bool = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CredentialContext, credentials::resolve_properties};

    #[test]
    fn key_is_bearer_token() {
        assert_eq!(BearerTokenCredential::KEY, "bearer_token");
    }

    #[tokio::test]
    async fn resolve_wraps_token_into_secret_token() {
        let properties = resolve_properties::<BearerTokenCredential>(serde_json::json!({
            "token": "sk-abc123"
        }))
        .unwrap();
        assert_eq!(properties.token.expose_secret(), "sk-abc123");
        let ctx = CredentialContext::for_owner("test-user");
        let result = BearerTokenCredential::resolve(&properties, &ctx)
            .await
            .expect("resolve ok");
        match result {
            StaticResolveResult::Complete(scheme) => {
                let _: &SecretToken = &scheme;
                assert_eq!(scheme.token().expose_secret(), "sk-abc123");
            },
            _ => panic!("expected Complete"),
        }
    }

    #[test]
    fn schema_rejects_missing_token_before_resolve() {
        let Err(report) = resolve_properties::<BearerTokenCredential>(serde_json::json!({})) else {
            panic!("missing bearer token must fail schema validation");
        };
        assert!(
            report.errors().any(|error| {
                error.code() == "required" && error.path().to_string() == "/token"
            })
        );
    }
}
