//! Pre-shared symmetric-key credential — static, non-interactive.
//!
//! Resolves a single secret key into [`SharedKey`]. `State = Scheme`
//! (identity projection). Reference impl mirroring the contract crate's
//! `BasicAuthCredential` shape.

use nebula_schema::{FieldValues, Schema};
use serde::Deserialize;

use crate::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialMetadata,
    ProviderErrorContext, ProviderErrorKind, SecretFreeMessage, SecretString,
    contract::plugin_capability_report, contract::resolve::ResolveResult, scheme::SharedKey,
};

/// Setup-form shape for the `shared_key` credential.
#[derive(Schema, Deserialize, Default)]
pub struct SharedKeyProperties {
    /// The pre-shared symmetric key material.
    #[field(secret, label = "Pre-shared key")]
    #[validate(required)]
    pub key: String,
}

/// Static pre-shared-key credential. Projects stored state (the key)
/// directly as the auth scheme.
pub struct SharedKeyCredential;

impl Credential for SharedKeyCredential {
    type Properties = SharedKeyProperties;
    type Scheme = SharedKey;
    type State = SharedKey;

    const KEY: &'static str = "shared_key";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("shared_key"))
            .name("Pre-shared Key")
            .description("Pre-shared symmetric key (TLS-PSK, WireGuard, IoT).")
            .schema(nebula_schema::schema_of::<Self::Properties>())
            .pattern(AuthPattern::SharedSecret)
            .icon("key")
            .build()
            .expect("shared_key metadata is valid")
    }

    fn project(state: &SharedKey) -> SharedKey {
        state.clone()
    }

    async fn resolve(
        values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SharedKey, ()>, CredentialError> {
        let key = values.get_string_by_str("key").ok_or_else(|| {
            CredentialError::Provider(Box::new(ProviderErrorContext::new(
                ProviderErrorKind::Schema,
                SecretFreeMessage::new("missing required field 'key'"),
            )))
        })?;
        Ok(ResolveResult::Complete(SharedKey::new(SecretString::new(
            key.to_owned(),
        ))))
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
    use crate::CredentialContext;
    use nebula_schema::FieldValues;

    #[test]
    fn key_is_shared_key() {
        assert_eq!(SharedKeyCredential::KEY, "shared_key");
    }

    #[tokio::test]
    async fn resolve_wraps_key_into_shared_key() {
        let mut values = FieldValues::new();
        values
            .try_set_raw("key", serde_json::Value::String("psk-xyz".into()))
            .expect("test-only known-good key");
        let ctx = CredentialContext::for_test("u");
        let r = SharedKeyCredential::resolve(&values, &ctx)
            .await
            .expect("ok");
        match r {
            ResolveResult::Complete(s) => {
                assert_eq!(s.key().expose_secret(), "psk-xyz");
            },
            _ => panic!("expected Complete"),
        }
    }

    #[tokio::test]
    async fn resolve_errors_on_missing_key() {
        let ctx = CredentialContext::for_test("u");
        assert!(
            SharedKeyCredential::resolve(&FieldValues::new(), &ctx)
                .await
                .is_err()
        );
    }
}
