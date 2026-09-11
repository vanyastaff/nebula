//! Request-signing-key credential — static, non-interactive.
//!
//! Resolves a secret key + algorithm id into [`SigningKey`]. `State =
//! Scheme` (identity projection). Reference impl mirroring the contract
//! crate's `BasicAuthCredential` shape.

use nebula_schema::Schema;
use serde::Deserialize;

use crate::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialMetadataDraft,
    SecretString, contract::plugin_capability_report, contract::resolve::StaticResolveResult,
    scheme::SigningKey,
};

/// Setup-form shape for the `signing_key` credential.
#[derive(Schema, Deserialize)]
pub struct SigningKeyProperties {
    /// The signing secret (HMAC key, webhook signing secret).
    #[field(secret, label = "Signing key")]
    #[validate(required)]
    pub key: SecretString,
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

    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("signing_key"),
            crate::metadata_name!("Signing Key"),
            "Request-signing secret (HMAC, SigV4, webhook signatures).",
            AuthPattern::RequestSigning,
        )
        .with_icon(nebula_metadata::Icon::inline("key"))
    }

    fn project(state: &SigningKey) -> SigningKey {
        state.clone()
    }

    async fn resolve(
        properties: &SigningKeyProperties,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<SigningKey>, CredentialError> {
        Ok(StaticResolveResult::Complete(SigningKey::new(
            properties.key.clone(),
            properties.algorithm.clone(),
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
    use crate::{CredentialContext, credentials::resolve_properties};

    #[test]
    fn key_is_signing_key() {
        assert_eq!(SigningKeyCredential::KEY, "signing_key");
    }

    #[tokio::test]
    async fn resolve_wraps_key_and_algorithm() {
        let properties = resolve_properties::<SigningKeyCredential>(serde_json::json!({
            "key": "whsec_1", "algorithm": "hmac-sha256"
        }))
        .unwrap();
        assert_eq!(properties.key.expose_secret(), "whsec_1");
        let ctx = CredentialContext::for_owner("u");
        let r = SigningKeyCredential::resolve(&properties, &ctx)
            .await
            .expect("ok");
        match r {
            StaticResolveResult::Complete(s) => {
                assert_eq!(s.key().expose_secret(), "whsec_1");
                assert_eq!(s.algorithm(), "hmac-sha256");
            },
            _ => panic!("expected Complete"),
        }
    }

    #[test]
    fn schema_rejects_missing_key_before_resolve() {
        let Err(report) = resolve_properties::<SigningKeyCredential>(serde_json::json!({
            "algorithm": "hmac-sha256"
        })) else {
            panic!("missing signing key must fail schema validation");
        };
        assert!(
            report
                .errors()
                .any(|error| { error.code() == "required" && error.path().to_string() == "/key" })
        );
    }
}
