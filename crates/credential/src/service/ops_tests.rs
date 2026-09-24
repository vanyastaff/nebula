use std::{
    assert_matches,
    sync::atomic::{AtomicUsize, Ordering},
};

use nebula_schema::{HasSchema, Property, Schema, Transformer, field_key};
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;
use crate::{
    CredentialError, CredentialMetadataDraft, DisplayData, InteractionRequest, Interactive,
    NoPendingState, PendingState, PendingStoreError, ResolveResult, SecretString, SecretToken,
    StaticResolveResult, UserInput,
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
            .property(
                Property::secret(field_key!("token"))
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

struct HangingCredential;

impl Credential for HangingCredential {
    type Properties = NormalizedProperties;
    type State = SecretToken;
    type Scheme = SecretToken;
    const KEY: &'static str = "hanging_provider_test";

    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("hanging_provider_test"),
            crate::metadata_name!("Hanging provider"),
            "Provider timeout classification fixture",
        )
    }

    fn project(state: &Self::State) -> Self::Scheme {
        state.clone()
    }

    async fn resolve(
        _: &Self::Properties,
        _: &CredentialContext,
    ) -> Result<StaticResolveResult<Self::State>, CredentialError> {
        std::future::pending().await
    }
}

impl Interactive for HangingCredential {
    type Pending = NoPendingState;

    async fn begin(
        _: &Self::Properties,
        _: &CredentialContext,
    ) -> Result<ResolveResult<Self::State, Self::Pending>, CredentialError> {
        std::future::pending().await
    }

    async fn continue_resolve(
        _: &Self::Pending,
        _: &UserInput,
        _: &CredentialContext,
    ) -> Result<ResolveResult<Self::State, Self::Pending>, CredentialError> {
        std::future::pending().await
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
    let decoded: SecretToken = {
        // The resolve path produces the envelope-wrapped payload; decode
        // through the choke point with the row axes it reports.
        let body = decode_state_payload::<SecretToken>(
            &state.data,
            &state.state_kind,
            state.state_version,
        )
        .unwrap();
        body.into_state().unwrap()
    };
    assert_eq!(decoded.token().expose_secret(), "aa");
    assert_eq!(state.state_kind, SecretToken::KIND);
    assert_eq!(state.state_version, SecretToken::VERSION);
}

#[tokio::test]
async fn initial_acquisition_carries_conservative_provider_boundary_evidence() {
    let outcome = dispatch()
        .acquire(
            NormalizedCredential::KEY,
            json!({"token": "value"}),
            &CredentialContext::for_owner("owner"),
            &UnusedPendingStore,
        )
        .await
        .expect("fixture acquisition completes");

    assert!(matches!(
        outcome,
        AcquireOutcome::Complete(ResolvedState {
            completion_evidence: AcquisitionCompletionEvidence::ProviderBoundaryUnproven,
            ..
        })
    ));
}

#[tokio::test(start_paused = true)]
async fn provider_capable_initial_timeouts_are_outcome_unknown_for_all_entry_paths() {
    let mut ops = DispatchOps::new();
    register_runtime_ops::<HangingCredential, UnusedPendingStore>(&mut ops)
        .expect("base fixture registration succeeds");
    register_interactive_ops::<HangingCredential, UnusedPendingStore>(&mut ops)
        .expect("interactive fixture registration succeeds");
    let ctx = CredentialContext::for_owner("owner").with_session_id("session");
    let props = || json!({"token": "value"});

    let create_or_update = ops
        .resolve(HangingCredential::KEY, props(), &ctx)
        .await
        .err()
        .expect("provider-capable resolve times out ambiguously");
    assert_matches!(create_or_update, CredentialServiceError::OutcomeUnknown);

    let initial_acquire = ops
        .acquire(HangingCredential::KEY, props(), &ctx, &UnusedPendingStore)
        .await
        .err()
        .expect("provider-capable begin times out ambiguously");
    assert_matches!(initial_acquire, CredentialServiceError::OutcomeUnknown);

    let reauthorization = ops
        .acquire_with_intent(
            HangingCredential::KEY,
            props(),
            &ctx,
            &UnusedPendingStore,
            AcquisitionIntent::ReauthorizeExisting {
                credential_id: "cred-1".to_owned(),
                observed_version: 1,
                observed_material_epoch: 1,
                credential_key: HangingCredential::KEY.to_owned(),
            },
        )
        .await
        .err()
        .expect("provider-capable reauthorization begin times out ambiguously");
    assert_matches!(reauthorization, CredentialServiceError::OutcomeUnknown);
}

struct InteractiveEvidenceCredential;

impl Credential for InteractiveEvidenceCredential {
    type Properties = NormalizedProperties;
    type State = SecretToken;
    type Scheme = SecretToken;
    const KEY: &'static str = "interactive_evidence_test";

    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("interactive_evidence_test"),
            crate::metadata_name!("Interactive evidence"),
            "Interactive completion evidence fixture",
        )
    }

    fn project(state: &Self::State) -> Self::Scheme {
        state.clone()
    }

    async fn resolve(
        properties: &Self::Properties,
        _: &CredentialContext,
    ) -> Result<StaticResolveResult<Self::State>, CredentialError> {
        Ok(StaticResolveResult::Complete(SecretToken::new(
            properties.token.clone(),
        )))
    }
}

impl Interactive for InteractiveEvidenceCredential {
    type Pending = NoPendingState;

    async fn begin(
        _: &Self::Properties,
        _: &CredentialContext,
    ) -> Result<ResolveResult<Self::State, Self::Pending>, CredentialError> {
        Ok(ResolveResult::Pending {
            state: NoPendingState,
            interaction: InteractionRequest::DisplayInfo {
                title: "Continue".to_owned(),
                message: "Continue acquisition".to_owned(),
                data: DisplayData::Text("continue".to_owned()),
                expires_in: Some(60),
            },
        })
    }

    async fn continue_resolve(
        _: &Self::Pending,
        _: &UserInput,
        _: &CredentialContext,
    ) -> Result<ResolveResult<Self::State, Self::Pending>, CredentialError> {
        Ok(ResolveResult::Complete(SecretToken::new(
            SecretString::new("provider-result"),
        )))
    }
}

struct ConsumedPendingStore {
    pending: Vec<u8>,
    consume_calls: AtomicUsize,
    consume_failure: ConsumeFailure,
    put_fails: bool,
}

#[derive(Clone, Copy)]
enum ConsumeFailure {
    None,
    NotFound,
    Backend,
}

impl PendingStateStore for ConsumedPendingStore {
    async fn put<P: PendingState>(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: P,
    ) -> Result<PendingToken, PendingStoreError> {
        if self.put_fails {
            return Err(PendingStoreError::Backend(Box::new(std::io::Error::other(
                "test pending backend failure",
            ))));
        }
        panic!("completed continuation must not write pending state")
    }

    async fn get<P: PendingState>(&self, _: &PendingToken) -> Result<P, PendingStoreError> {
        panic!("bound continuation must not use unscoped get")
    }

    async fn get_bound<P: PendingState>(
        &self,
        _: &str,
        _: &PendingToken,
        _: &str,
        _: &str,
    ) -> Result<P, PendingStoreError> {
        serde_json::from_slice(&self.pending)
            .map_err(|error| PendingStoreError::Backend(Box::new(error)))
    }

    async fn consume<P: PendingState>(
        &self,
        _: &str,
        _: &PendingToken,
        _: &str,
        _: &str,
    ) -> Result<P, PendingStoreError> {
        self.consume_calls.fetch_add(1, Ordering::Relaxed);
        match self.consume_failure {
            ConsumeFailure::None => {},
            ConsumeFailure::NotFound => return Err(PendingStoreError::NotFound),
            ConsumeFailure::Backend => {
                return Err(PendingStoreError::Backend(Box::new(std::io::Error::other(
                    "test pending backend failure",
                ))));
            },
        }
        serde_json::from_slice(&self.pending)
            .map_err(|error| PendingStoreError::Backend(Box::new(error)))
    }

    async fn delete(&self, _: &PendingToken) -> Result<(), PendingStoreError> {
        panic!("completed continuation must not delete pending state")
    }
}

#[tokio::test]
async fn reauthorization_continuation_marks_completion_after_consuming_authority() {
    let intent = AcquisitionIntent::ReauthorizeExisting {
        credential_id: "cred-1".to_owned(),
        observed_version: 4,
        observed_material_epoch: 2,
        credential_key: InteractiveEvidenceCredential::KEY.to_owned(),
    };
    let expectation = intent.expectation();
    let pending = AcquisitionPending::new(intent, NoPendingState);
    let pending = crate::serde_secret::expose_for_serialization(|| serde_json::to_vec(&pending))
        .expect("serialize pending fixture");
    let store = ConsumedPendingStore {
        pending,
        consume_calls: AtomicUsize::new(0),
        consume_failure: ConsumeFailure::None,
        put_fails: false,
    };
    let mut ops = DispatchOps::new();
    register_runtime_ops::<InteractiveEvidenceCredential, _>(&mut ops)
        .expect("base fixture registration succeeds");
    register_interactive_ops::<InteractiveEvidenceCredential, _>(&mut ops)
        .expect("interactive fixture registration succeeds");

    let outcome = ops
        .continue_with_intent(
            &PendingToken::generate(),
            &UserInput::Code {
                code: "one-time-code".to_owned(),
            },
            &CredentialContext::for_owner("owner").with_session_id("session"),
            &store,
            expectation,
        )
        .await
        .expect("provider continuation completes");

    assert_eq!(store.consume_calls.load(Ordering::Relaxed), 1);
    assert!(matches!(
        outcome,
        AcquireOutcome::Complete(ResolvedState {
            completion_evidence: AcquisitionCompletionEvidence::InteractiveContinuationComplete,
            ..
        })
    ));
}

#[tokio::test]
async fn post_provider_poll_finalization_preserves_exact_vs_unknown_store_failure() {
    let intent = AcquisitionIntent::create_for_key(InteractiveEvidenceCredential::KEY);
    let expectation = intent.expectation();
    let pending = AcquisitionPending::new(intent, NoPendingState);
    let pending = crate::serde_secret::expose_for_serialization(|| serde_json::to_vec(&pending))
        .expect("serialize pending fixture");
    let mut ops = DispatchOps::new();
    register_runtime_ops::<InteractiveEvidenceCredential, _>(&mut ops)
        .expect("base fixture registration succeeds");
    register_interactive_ops::<InteractiveEvidenceCredential, _>(&mut ops)
        .expect("interactive fixture registration succeeds");
    let ctx = CredentialContext::for_owner("owner").with_session_id("session");

    for (consume_failure, expected_unknown) in [
        (ConsumeFailure::Backend, true),
        (ConsumeFailure::NotFound, false),
    ] {
        let store = ConsumedPendingStore {
            pending: pending.clone(),
            consume_calls: AtomicUsize::new(0),
            consume_failure,
            put_fails: false,
        };
        let error = ops
            .continue_with_intent(
                &PendingToken::generate(),
                &UserInput::Poll,
                &ctx,
                &store,
                expectation.clone(),
            )
            .await
            .err()
            .expect("post-provider pending finalization must fail closed");
        if expected_unknown {
            assert_matches!(error, CredentialServiceError::OutcomeUnknown);
        } else {
            assert_matches!(
                error,
                CredentialServiceError::AcquisitionFinalizationRequired
            );
        }
        assert_eq!(store.consume_calls.load(Ordering::Relaxed), 1);
    }
}

#[tokio::test]
async fn post_provider_begin_pending_write_failure_is_outcome_unknown() {
    let store = ConsumedPendingStore {
        pending: Vec::new(),
        consume_calls: AtomicUsize::new(0),
        consume_failure: ConsumeFailure::None,
        put_fails: true,
    };
    let mut ops = DispatchOps::new();
    register_runtime_ops::<InteractiveEvidenceCredential, _>(&mut ops)
        .expect("base fixture registration succeeds");
    register_interactive_ops::<InteractiveEvidenceCredential, _>(&mut ops)
        .expect("interactive fixture registration succeeds");

    let error = ops
        .acquire(
            InteractiveEvidenceCredential::KEY,
            json!({"token": "value"}),
            &CredentialContext::for_owner("owner").with_session_id("session"),
            &store,
        )
        .await
        .err()
        .expect("post-provider pending write must fail closed");

    assert_matches!(error, CredentialServiceError::OutcomeUnknown);
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

#[test]
fn acquisition_preserves_unknown_provider_outcome() {
    assert_matches!(
        credential_error_to_service_error(CredentialError::OutcomeUnknown),
        CredentialServiceError::OutcomeUnknown
    );
}

#[test]
fn post_provider_protocol_finalization_failures_require_reconciliation() {
    for error in [
        crate::runtime::ExecutorError::InvalidContinuationOutcome,
        crate::runtime::ExecutorError::PostProviderPendingFinalization(
            PendingStoreError::ValidationFailed {
                reason: "closed test reason".to_owned(),
            },
        ),
    ] {
        assert_matches!(
            executor_error_to_service_error(error),
            CredentialServiceError::AcquisitionFinalizationRequired
        );
    }
}

#[derive(zeroize::ZeroizeOnDrop, crate::StateWireFingerprint)]
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
        ops.test(
            UnserializableCredential::KEY,
            data,
            <UnserializableState as CredentialState>::KIND,
            <UnserializableState as CredentialState>::VERSION,
            &context,
        )
        .await
        .unwrap_err(),
        ops.revoke(
            UnserializableCredential::KEY,
            data,
            <UnserializableState as CredentialState>::KIND,
            <UnserializableState as CredentialState>::VERSION,
            &context,
        )
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
        assert_matches!(
            error,
            CredentialServiceError::AcquisitionFinalizationRequired
        );
    }
}
