use std::assert_matches;

use nebula_schema::{Field, HasSchema, Schema, Transformer, field_key};
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;
use crate::{
    AuthPattern, CredentialError, CredentialMetadataDraft, PendingState, PendingStoreError,
    SecretString, SecretToken, StaticResolveResult,
};

struct UnusedPendingStore;

impl PendingStateStore for UnusedPendingStore {
    async fn put<P: PendingState>(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: P,
    ) -> Result<PendingToken, PendingStoreError> {
        panic!("static credential resolution must not write pending state");
    }

    async fn get<P: PendingState>(&self, _: &PendingToken) -> Result<P, PendingStoreError> {
        panic!("static credential resolution must not read pending state");
    }

    async fn get_bound<P: PendingState>(
        &self,
        _: &str,
        _: &PendingToken,
        _: &str,
        _: &str,
    ) -> Result<P, PendingStoreError> {
        panic!("static credential resolution must not read pending state");
    }

    async fn consume<P: PendingState>(
        &self,
        _: &str,
        _: &PendingToken,
        _: &str,
        _: &str,
    ) -> Result<P, PendingStoreError> {
        panic!("static credential resolution must not consume pending state");
    }

    async fn delete(&self, _: &PendingToken) -> Result<(), PendingStoreError> {
        panic!("static credential resolution must not delete pending state");
    }
}

#[derive(Debug, Deserialize)]
struct NormalizedProperties {
    token: SecretString,
}

impl HasSchema for NormalizedProperties {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Schema::builder()
            .add(
                Field::secret(field_key!("token"))
                    .required()
                    .read_alias("old_token")?
                    .with_transformer(Transformer::Replace {
                        from: "a".into(),
                        to: "aa".into(),
                    }),
            )
            .build()
    }
}

struct NormalizedCredential;

impl Credential for NormalizedCredential {
    type Properties = NormalizedProperties;
    type State = SecretToken;
    type Scheme = SecretToken;
    const KEY: &'static str = "normalized_test";

    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("normalized_test"),
            crate::metadata_name!("Normalized"),
            "Normalized credential fixture",
            AuthPattern::SecretToken,
        )
    }

    fn project(state: &Self::State) -> Self::Scheme {
        state.clone()
    }

    async fn resolve(
        properties: &NormalizedProperties,
        _: &CredentialContext,
    ) -> Result<StaticResolveResult<Self::State>, CredentialError> {
        Ok(StaticResolveResult::Complete(SecretToken::new(
            properties.token.clone(),
        )))
    }
}

fn dispatch() -> DispatchOps<UnusedPendingStore> {
    let mut ops = DispatchOps::new();
    register_runtime_ops::<NormalizedCredential, _>(&mut ops).unwrap();
    ops
}

#[tokio::test]
async fn registered_ops_pass_the_once_normalized_secret_to_the_provider() {
    let ops = dispatch();
    let state = ops
        .resolve(
            NormalizedCredential::KEY,
            json!({"old_token": "a"}),
            &CredentialContext::for_owner("owner"),
        )
        .await
        .unwrap();
    let decoded: SecretToken = serde_json::from_slice(&state.data).unwrap();
    assert_eq!(decoded.token().expose_secret(), "aa");
    assert_eq!(state.state_kind, SecretToken::KIND);
    assert_eq!(state.state_version, SecretToken::VERSION);
}

#[test]
fn preparation_preserves_template_like_secret_strings_as_data() {
    let schema = NormalizedProperties::schema().unwrap();
    let properties =
        prepare_properties::<NormalizedCredential>(&schema, json!({"token": "{{ 1 / 0 }}"}))
            .unwrap();
    assert_eq!(properties.token.expose_secret(), "{{ 1 / 0 }}");
}

#[tokio::test]
async fn preparation_reports_structural_errors_without_input_material() {
    let ops = dispatch();
    for (input, code) in [
        (json!({}), "required"),
        (
            json!({"old_token": {"$expr": "private-expression"}}),
            "type_mismatch",
        ),
    ] {
        let result = ops
            .resolve(
                NormalizedCredential::KEY,
                input,
                &CredentialContext::for_owner("owner"),
            )
            .await;
        let Err(error) = result else {
            panic!("invalid properties must fail before provider dispatch");
        };
        assert!(!format!("{error:?} {error}").contains("private-expression"));
        let CredentialServiceError::ValidationFailed { report } = error else {
            panic!("expected structural validation failure");
        };
        assert_eq!(
            report
                .issues()
                .map(|issue| (issue.code(), issue.path()))
                .collect::<Vec<_>>(),
            [(code, "/token")]
        );
    }
}

#[test]
fn property_preparation_decodes_secret_into_the_typed_owner() {
    let properties = prepare_properties::<NormalizedCredential>(
        &NormalizedProperties::schema().unwrap(),
        json!({"token": "SECRET_CANARY"}),
    )
    .unwrap();
    assert_eq!(properties.token.expose_secret(), "SECRET_CANARY");
    assert!(!format!("{properties:?}").contains("SECRET_CANARY"));
}

#[derive(zeroize::ZeroizeOnDrop)]
struct UnserializableState {
    token: String,
}

impl<'de> Deserialize<'de> for UnserializableState {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let input = Value::deserialize(deserializer)?;
        Err(serde::de::Error::custom(input))
    }
}

impl serde::Serialize for UnserializableState {
    fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom(&self.token))
    }
}

crate::identity_state!(UnserializableState, "unserializable_fixture", 1);

struct UnserializableCredential;

impl Credential for UnserializableCredential {
    type Properties = Value;
    type State = UnserializableState;
    type Scheme = ();
    const KEY: &'static str = "unserializable_fixture";

    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("unserializable_fixture"),
            crate::metadata_name!("Unserializable"),
            "Failing serializer fixture",
            AuthPattern::NoAuth,
        )
    }

    fn project(_: &Self::State) {}

    async fn resolve(
        _: &Value,
        _: &CredentialContext,
    ) -> Result<StaticResolveResult<Self::State>, CredentialError> {
        Ok(StaticResolveResult::Complete(UnserializableState {
            token: "STATE_SECRET_CANARY".to_owned(),
        }))
    }
}

impl Testable for UnserializableCredential {
    async fn test((): &(), _: &CredentialContext) -> Result<TestResult, CredentialError> {
        panic!("invalid stored state must not reach the provider probe");
    }
}

impl Revocable for UnserializableCredential {
    async fn revoke(
        _: &mut UnserializableState,
        _: &CredentialContext,
    ) -> Result<(), CredentialError> {
        panic!("invalid stored state must not reach provider revocation");
    }
}

#[tokio::test]
async fn stored_state_decoding_errors_never_publish_stored_material() {
    let mut ops = DispatchOps::new();
    register_runtime_ops::<UnserializableCredential, UnusedPendingStore>(&mut ops).unwrap();
    register_testable_ops::<UnserializableCredential, _>(&mut ops).unwrap();
    register_revocable_ops::<UnserializableCredential, _>(&mut ops).unwrap();
    let context = CredentialContext::for_owner("owner");
    let data = br#"{"token":"STATE_SECRET_CANARY"}"#;
    for error in [
        ops.test(UnserializableCredential::KEY, data, &context)
            .await
            .unwrap_err(),
        ops.revoke(UnserializableCredential::KEY, data, &context)
            .await
            .unwrap_err(),
    ] {
        let mut cause: Option<&dyn std::error::Error> = Some(&error);
        while let Some(current) = cause {
            assert!(!format!("{current:?} {current}").contains("STATE_SECRET_CANARY"));
            cause = current.source();
        }
        assert_matches!(error, CredentialServiceError::Internal(_));
    }
}

#[tokio::test]
async fn stored_state_serialization_errors_never_publish_provider_material() {
    let mut ops = DispatchOps::new();
    register_runtime_ops::<UnserializableCredential, UnusedPendingStore>(&mut ops).unwrap();
    let context = CredentialContext::for_owner("owner");
    for error in [
        ops.resolve(UnserializableCredential::KEY, json!({}), &context)
            .await
            .err()
            .expect("failing state serializer rejects resolution"),
        ops.acquire(
            UnserializableCredential::KEY,
            json!({}),
            &context,
            &UnusedPendingStore,
        )
        .await
        .err()
        .expect("failing state serializer rejects acquisition"),
    ] {
        assert!(!format!("{error:?} {error}").contains("STATE_SECRET_CANARY"));
        assert_matches!(error, CredentialServiceError::Internal(_));
    }
}
