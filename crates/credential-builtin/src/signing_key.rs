//! Request-signing-key credential — static, non-interactive.
//!
//! Resolves a secret key + algorithm id into [`SigningKey`]. `State =
//! Scheme` (identity projection). Reference impl mirroring the contract
//! crate's `BasicAuthCredential` shape.

use nebula_credential::contract::plugin_capability_report;
use nebula_credential::contract::resolve::ResolveResult;
use nebula_credential::scheme::SigningKey;
use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialMetadata, SecretString,
};
use nebula_schema::{FieldValues, Schema};
use serde::Deserialize;

/// Setup-form shape for the `signing_key` credential.
#[derive(Schema, Deserialize, Default)]
pub struct SigningKeyProperties {
    /// The signing secret (HMAC key, webhook signing secret).
    #[field(secret, label = "Signing key")]
    #[validate(required)]
    pub key: String,
    /// Algorithm identifier (e.g. `hmac-sha256`, `sigv4`).
    #[field(label = "Algorithm")]
    #[validate(required)]
    pub algorithm: String,
}

/// Static request-signing-key credential. Projects stored state (the key
/// + algorithm) directly as the auth scheme.
pub struct SigningKeyCredential;

impl Credential for SigningKeyCredential {
    type Properties = SigningKeyProperties;
    type Scheme = SigningKey;
    type State = SigningKey;

    const KEY: &'static str = "signing_key";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("signing_key"))
            .name("Signing Key")
            .description("Request-signing secret (HMAC, SigV4, webhook signatures).")
            .schema(Self::properties_schema())
            .pattern(AuthPattern::RequestSigning)
            .icon("key")
            .build()
            .expect("signing_key metadata is valid")
    }

    fn project(state: &SigningKey) -> SigningKey {
        state.clone()
    }

    async fn resolve(
        values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SigningKey, ()>, CredentialError> {
        let key = values
            .get_string_by_str("key")
            .ok_or_else(|| CredentialError::Provider("missing required field 'key'".to_owned()))?;
        let algorithm = values.get_string_by_str("algorithm").ok_or_else(|| {
            CredentialError::Provider("missing required field 'algorithm'".to_owned())
        })?;
        Ok(ResolveResult::Complete(SigningKey::new(
            SecretString::new(key.to_owned()),
            algorithm.to_owned(),
        )))
    }
}

impl plugin_capability_report::IsInteractive for SigningKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRefreshable for SigningKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRevocable for SigningKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsTestable for SigningKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsDynamic for SigningKeyCredential {
    const VALUE: bool = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_credential::CredentialContext;
    use nebula_schema::FieldValues;

    #[test]
    fn key_is_signing_key() {
        assert_eq!(SigningKeyCredential::KEY, "signing_key");
    }

    #[tokio::test]
    async fn resolve_wraps_key_and_algorithm() {
        let mut values = FieldValues::new();
        values
            .try_set_raw("key", serde_json::Value::String("whsec_1".into()))
            .expect("test-only known-good key");
        values
            .try_set_raw("algorithm", serde_json::Value::String("hmac-sha256".into()))
            .expect("test-only known-good key");
        let ctx = CredentialContext::for_test("u");
        let r = SigningKeyCredential::resolve(&values, &ctx)
            .await
            .expect("ok");
        match r {
            ResolveResult::Complete(s) => {
                assert_eq!(s.key().expose_secret(), "whsec_1");
                assert_eq!(s.algorithm(), "hmac-sha256");
            },
            _ => panic!("expected Complete"),
        }
    }

    #[tokio::test]
    async fn resolve_errors_on_missing_key() {
        let mut values = FieldValues::new();
        values
            .try_set_raw("algorithm", serde_json::Value::String("hmac-sha256".into()))
            .expect("test-only known-good key");
        let ctx = CredentialContext::for_test("u");
        assert!(SigningKeyCredential::resolve(&values, &ctx).await.is_err());
    }
}
