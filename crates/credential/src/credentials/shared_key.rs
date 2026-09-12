//! Pre-shared symmetric-key credential — static, non-interactive.
//!
//! Resolves a single secret key into [`SharedKey`]. `State = Scheme`
//! (identity projection). Reference impl mirroring the contract crate's
//! `BasicAuthCredential` shape.

use nebula_schema::Schema;
use serde::Deserialize;

use crate::{
    Credential, CredentialContext, CredentialError, CredentialMetadataDraft, SecretString,
    contract::plugin_capability_report, contract::resolve::StaticResolveResult, scheme::SharedKey,
};

/// Setup-form shape for the `shared_key` credential.
#[derive(Schema, Deserialize)]
pub struct SharedKeyProperties {
    /// The pre-shared symmetric key material.
    #[field(secret, label = "Pre-shared key")]
    #[validate(required)]
    pub key: SecretString,
}

/// Static pre-shared-key credential. Projects stored state (the key)
/// directly as the auth scheme.
pub struct SharedKeyCredential;

impl Credential for SharedKeyCredential {
    type Properties = SharedKeyProperties;
    type Scheme = SharedKey;
    type State = SharedKey;

    const KEY: &'static str = "shared_key";

    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("shared_key"),
            crate::metadata_name!("Pre-shared Key"),
            "Pre-shared symmetric key (TLS-PSK, WireGuard, IoT).",
        )
        .with_icon(nebula_metadata::Icon::inline("key"))
    }

    fn project(state: &SharedKey) -> SharedKey {
        state.clone()
    }

    async fn resolve(
        properties: &SharedKeyProperties,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<SharedKey>, CredentialError> {
        Ok(StaticResolveResult::Complete(SharedKey::new(
            properties.key.clone(),
        )))
    }
}

impl plugin_capability_report::IsInteractive for SharedKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRefreshable for SharedKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRevocable for SharedKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsTestable for SharedKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsDynamic for SharedKeyCredential {
    const VALUE: bool = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CredentialContext, credentials::resolve_properties};

    #[test]
    fn key_is_shared_key() {
        assert_eq!(SharedKeyCredential::KEY, "shared_key");
    }

    #[tokio::test]
    async fn resolve_wraps_key_into_shared_key() {
        let properties = resolve_properties::<SharedKeyCredential>(serde_json::json!({
            "key": "psk-xyz"
        }))
        .unwrap();
        assert_eq!(properties.key.expose_secret(), "psk-xyz");
        let ctx = CredentialContext::for_owner("u");
        let r = SharedKeyCredential::resolve(&properties, &ctx)
            .await
            .expect("ok");
        match r {
            StaticResolveResult::Complete(s) => {
                assert_eq!(s.key().expose_secret(), "psk-xyz");
            },
            _ => panic!("expected Complete"),
        }
    }

    #[test]
    fn schema_rejects_missing_key_before_resolve() {
        let Err(report) = resolve_properties::<SharedKeyCredential>(serde_json::json!({})) else {
            panic!("missing shared key must fail schema validation");
        };
        assert!(
            report
                .errors()
                .any(|error| { error.code() == "required" && error.path().to_string() == "/key" })
        );
    }
}
