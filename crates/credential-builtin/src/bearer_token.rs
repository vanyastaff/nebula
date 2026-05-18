//! Opaque bearer-token credential — static, non-interactive.
//!
//! Resolves a single secret token into [`SecretToken`]. `State = Scheme`
//! (identity projection). Reference impl mirroring the contract crate's
//! `BasicAuthCredential` shape.

use nebula_credential::contract::plugin_capability_report;
use nebula_credential::contract::resolve::ResolveResult;
use nebula_credential::scheme::SecretToken;
use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialMetadata, SecretString,
};
use nebula_schema::{FieldValues, Schema};
use serde::Deserialize;

/// Setup-form shape for the `bearer_token` credential.
#[derive(Schema, Deserialize, Default)]
pub struct BearerTokenProperties {
    /// The opaque bearer token (API key, PAT, session token).
    #[field(secret, label = "Token")]
    #[validate(required)]
    pub token: String,
}

/// Static opaque-token credential. Projects stored state (the token)
/// directly as the auth scheme.
pub struct BearerTokenCredential;

impl Credential for BearerTokenCredential {
    type Properties = BearerTokenProperties;
    type Scheme = SecretToken;
    type State = SecretToken;

    const KEY: &'static str = "bearer_token";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("bearer_token"))
            .name("Bearer Token")
            .description("Opaque bearer token (API key, PAT, session token).")
            .schema(nebula_schema::schema_of::<Self::Properties>())
            .pattern(AuthPattern::SecretToken)
            .icon("key")
            .build()
            .expect("bearer_token metadata is valid")
    }

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SecretToken, ()>, CredentialError> {
        let token = values.get_string_by_str("token").ok_or_else(|| {
            CredentialError::Provider("missing required field 'token'".to_owned())
        })?;
        Ok(ResolveResult::Complete(SecretToken::new(
            SecretString::new(token.to_owned()),
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
    use nebula_credential::CredentialContext;
    use nebula_schema::FieldValues;

    #[test]
    fn key_is_bearer_token() {
        assert_eq!(BearerTokenCredential::KEY, "bearer_token");
    }

    #[tokio::test]
    async fn resolve_wraps_token_into_secret_token() {
        let mut values = FieldValues::new();
        values
            .try_set_raw("token", serde_json::Value::String("sk-abc123".into()))
            .expect("test-only known-good key");
        let ctx = CredentialContext::for_test("test-user");
        let result = BearerTokenCredential::resolve(&values, &ctx)
            .await
            .expect("resolve ok");
        match result {
            ResolveResult::Complete(scheme) => {
                let _: &SecretToken = &scheme;
                assert_eq!(scheme.token().expose_secret(), "sk-abc123");
            },
            _ => panic!("expected Complete"),
        }
    }

    #[tokio::test]
    async fn resolve_errors_on_missing_token() {
        let values = FieldValues::new();
        let ctx = CredentialContext::for_test("test-user");
        assert!(BearerTokenCredential::resolve(&values, &ctx).await.is_err());
    }
}
