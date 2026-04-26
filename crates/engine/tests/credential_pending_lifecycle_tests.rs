//! Integration test: pending state lifecycle (interactive credential flow).
//!
//! Validates the full interactive credential resolution flow:
//! `execute_resolve` -> `Pending` -> `execute_continue` -> `Complete`.

use std::time::Duration;

use nebula_credential::{
    Credential, CredentialContext, CredentialMetadata, Interactive, PendingState,
    PendingStateStore, PendingStoreError, PendingToken, Refreshable, SecretString,
    credentials::OAuth2Pending,
    error::CredentialError,
    resolve::{RefreshOutcome, ResolveResult, UserInput},
    scheme::SecretToken,
};
use nebula_schema::FieldValues;
use nebula_storage::credential::InMemoryPendingStore;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TestPending {
    verification_code: String,
}

impl zeroize::Zeroize for TestPending {
    fn zeroize(&mut self) {
        self.verification_code.zeroize();
    }
}

// Per Tech Spec §15.4 — `PendingState: ZeroizeOnDrop`. Hand-rolled
// because the manual `Zeroize` body above conflicts with a derived
// `Drop`; this delegates Drop to the existing zeroize logic.
impl Drop for TestPending {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(self);
    }
}
impl zeroize::ZeroizeOnDrop for TestPending {}

impl PendingState for TestPending {
    const KIND: &'static str = "test_interactive_pending";

    fn expires_in(&self) -> Duration {
        Duration::from_mins(5)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ShortTtl {
    data: String,
}

impl zeroize::Zeroize for ShortTtl {
    fn zeroize(&mut self) {
        self.data.zeroize();
    }
}

// Per Tech Spec §15.4 — `PendingState: ZeroizeOnDrop`. Same hand-roll
// rationale as `TestPending` above.
impl Drop for ShortTtl {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(self);
    }
}
impl zeroize::ZeroizeOnDrop for ShortTtl {}

impl PendingState for ShortTtl {
    const KIND: &'static str = "short_ttl";

    fn expires_in(&self) -> Duration {
        Duration::ZERO
    }
}

#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop,
)]
struct TestInteractiveState {
    token: String,
}

impl nebula_credential::CredentialState for TestInteractiveState {
    const KIND: &'static str = "interactive_test";
    const VERSION: u32 = 1;
}

struct InteractiveTestCredential;

impl Credential for InteractiveTestCredential {
    type Input = FieldValues;
    type Scheme = SecretToken;
    type State = TestInteractiveState;

    const KEY: &'static str = "interactive_test";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::new(
            nebula_core::credential_key!("interactive_test"),
            "Interactive Test",
            "Test credential for pending lifecycle",
            Self::schema(),
            nebula_credential::AuthPattern::SecretToken,
        )
    }

    fn project(state: &TestInteractiveState) -> SecretToken {
        SecretToken::new(SecretString::new(state.token.clone()))
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<TestInteractiveState, ()>, CredentialError> {
        // Per Tech Spec §15.4 the base `Credential::resolve` cannot
        // carry typed pending state. The interactive entry point goes
        // through credential-specific kickoff helpers + direct
        // PendingStateStore::put — see the test bodies below for the
        // pattern.
        Err(CredentialError::Provider(
            "interactive_test must be initiated via PendingStateStore::put + execute_continue"
                .into(),
        ))
    }
}

impl Interactive for InteractiveTestCredential {
    type Pending = TestPending;

    async fn continue_resolve(
        pending: &TestPending,
        input: &UserInput,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<TestInteractiveState, TestPending>, CredentialError> {
        match input {
            UserInput::Code { code } if code == &pending.verification_code => {
                Ok(ResolveResult::Complete(TestInteractiveState {
                    token: "final-token".into(),
                }))
            },
            _ => Err(CredentialError::InvalidInput(
                "incorrect verification code".into(),
            )),
        }
    }
}

impl Refreshable for InteractiveTestCredential {
    async fn refresh(
        _state: &mut TestInteractiveState,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        Ok(RefreshOutcome::NotSupported)
    }
}

struct RetryAwareCredential;

impl Credential for RetryAwareCredential {
    type Input = FieldValues;
    type Scheme = SecretToken;
    type State = TestInteractiveState;

    const KEY: &'static str = "retry_aware";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::new(
            nebula_core::credential_key!("retry_aware"),
            "Retry Aware",
            "Test credential for retry-poll pending lifecycle",
            Self::schema(),
            nebula_credential::AuthPattern::SecretToken,
        )
    }

    fn project(state: &TestInteractiveState) -> SecretToken {
        SecretToken::new(SecretString::new(state.token.clone()))
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<TestInteractiveState, ()>, CredentialError> {
        Err(CredentialError::Provider(
            "retry_aware must be initiated via PendingStateStore::put + execute_continue".into(),
        ))
    }
}

impl Interactive for RetryAwareCredential {
    type Pending = TestPending;

    async fn continue_resolve(
        pending: &TestPending,
        input: &UserInput,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<TestInteractiveState, TestPending>, CredentialError> {
        match input {
            UserInput::Poll => Ok(ResolveResult::Retry {
                after: Duration::from_secs(1),
            }),
            UserInput::Code { code } if code == &pending.verification_code => {
                Ok(ResolveResult::Complete(TestInteractiveState {
                    token: "final-token".into(),
                }))
            },
            _ => Err(CredentialError::InvalidInput(
                "incorrect verification code".into(),
            )),
        }
    }
}

impl Refreshable for RetryAwareCredential {
    async fn refresh(
        _state: &mut TestInteractiveState,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        Ok(RefreshOutcome::NotSupported)
    }
}

/// Helper: kickoff an interactive test credential by storing the typed
/// `TestPending` directly in the pending store and returning the issued
/// token. Per Tech Spec §15.4 the base `Credential::resolve` cannot
/// carry typed pending state — the kickoff happens at the API
/// orchestration layer (or in tests, here).
async fn kickoff_test_pending(
    pending_store: &InMemoryPendingStore,
    ctx: &CredentialContext,
    credential_key: &str,
) -> PendingToken {
    let session_id = ctx.session_id().unwrap_or("default");
    let pending = TestPending {
        verification_code: "secret-code-123".into(),
    };
    pending_store
        .put(credential_key, ctx.owner_id(), session_id, pending)
        .await
        .expect("pending_store::put should succeed")
}

#[tokio::test]
async fn pending_lifecycle_resolve_then_continue() {
    let pending_store = InMemoryPendingStore::new();
    let ctx = CredentialContext::for_test("test-user").with_session_id("sess-1");

    let token = kickoff_test_pending(&pending_store, &ctx, InteractiveTestCredential::KEY).await;

    let input = UserInput::Code {
        code: "secret-code-123".into(),
    };
    let response = nebula_engine::credential::execute_continue::<InteractiveTestCredential, _>(
        &token,
        &input,
        &ctx,
        &pending_store,
    )
    .await
    .expect("execute_continue should succeed");

    match response {
        nebula_engine::credential::ResolveResponse::Complete(state) => {
            assert_eq!(state.token, "final-token");
        },
        other => panic!("expected Complete response, got: {other:?}"),
    }
}

#[tokio::test]
async fn pending_token_is_single_use() {
    let pending_store = InMemoryPendingStore::new();
    let ctx = CredentialContext::for_test("test-user").with_session_id("sess-1");

    let token = kickoff_test_pending(&pending_store, &ctx, InteractiveTestCredential::KEY).await;

    let input = UserInput::Code {
        code: "secret-code-123".into(),
    };
    let result = nebula_engine::credential::execute_continue::<InteractiveTestCredential, _>(
        &token,
        &input,
        &ctx,
        &pending_store,
    )
    .await;
    assert!(result.is_ok(), "first continue should succeed");

    let result = nebula_engine::credential::execute_continue::<InteractiveTestCredential, _>(
        &token,
        &input,
        &ctx,
        &pending_store,
    )
    .await;
    assert!(result.is_err(), "second continue should fail (single-use)");
}

#[tokio::test]
async fn continue_with_wrong_code_returns_error() {
    let pending_store = InMemoryPendingStore::new();
    let ctx = CredentialContext::for_test("test-user").with_session_id("sess-1");

    let token = kickoff_test_pending(&pending_store, &ctx, InteractiveTestCredential::KEY).await;

    let input = UserInput::Code {
        code: "wrong-code".into(),
    };
    let result = nebula_engine::credential::execute_continue::<InteractiveTestCredential, _>(
        &token,
        &input,
        &ctx,
        &pending_store,
    )
    .await;
    assert!(
        result.is_err(),
        "continue with wrong code should fail: {result:?}"
    );
}

#[tokio::test]
async fn retry_does_not_consume_pending_token() {
    let pending_store = InMemoryPendingStore::new();
    let ctx = CredentialContext::for_test("test-user").with_session_id("sess-1");

    let token = kickoff_test_pending(&pending_store, &ctx, RetryAwareCredential::KEY).await;

    let retry = nebula_engine::credential::execute_continue::<RetryAwareCredential, _>(
        &token,
        &UserInput::Poll,
        &ctx,
        &pending_store,
    )
    .await
    .expect("poll should return Retry");
    let retry_token = match retry {
        nebula_engine::credential::ResolveResponse::Retry {
            after,
            token: Some(token),
        } => {
            assert_eq!(after, Duration::from_secs(1));
            token
        },
        other => panic!("expected Retry(after=1s, token), got: {other:?}"),
    };

    let completed = nebula_engine::credential::execute_continue::<RetryAwareCredential, _>(
        &retry_token,
        &UserInput::Code {
            code: "secret-code-123".into(),
        },
        &ctx,
        &pending_store,
    )
    .await
    .expect("pending token should still be valid after Retry");
    assert!(
        matches!(
            completed,
            nebula_engine::credential::ResolveResponse::Complete(TestInteractiveState { ref token })
                if token == "final-token"
        ),
        "expected Complete after retry flow, got: {completed:?}"
    );
}

#[tokio::test]
async fn retry_path_rejects_mismatched_session() {
    let pending_store = InMemoryPendingStore::new();
    let owner_ctx = CredentialContext::for_test("test-user").with_session_id("sess-owner");
    let attacker_ctx = CredentialContext::for_test("test-user").with_session_id("sess-attacker");

    let token = kickoff_test_pending(&pending_store, &owner_ctx, RetryAwareCredential::KEY).await;

    let result = nebula_engine::credential::execute_continue::<RetryAwareCredential, _>(
        &token,
        &UserInput::Poll,
        &attacker_ctx,
        &pending_store,
    )
    .await;

    assert!(
        matches!(
            result,
            Err(nebula_engine::credential::ExecutorError::PendingStore(
                PendingStoreError::ValidationFailed { .. }
            ))
        ),
        "expected session-binding validation error, got: {result:?}"
    );
}

#[tokio::test]
async fn invariant_2_ttl_expiry_is_surfaced_and_evicted() {
    let store = InMemoryPendingStore::new();
    let pending = ShortTtl {
        data: "ephemeral".into(),
    };
    let token = store
        .put("oauth2", "user_1", "sess_1", pending)
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(5)).await;

    let err = store
        .consume::<ShortTtl>("oauth2", &token, "user_1", "sess_1")
        .await
        .unwrap_err();
    assert!(matches!(err, PendingStoreError::Expired));

    let err2 = store
        .consume::<ShortTtl>("oauth2", &token, "user_1", "sess_1")
        .await
        .unwrap_err();
    assert!(matches!(err2, PendingStoreError::NotFound));
}

#[tokio::test]
async fn invariant_3_consume_is_single_use() {
    let store = InMemoryPendingStore::new();
    let pending = TestPending {
        verification_code: "x".into(),
    };

    let token = store
        .put("interactive_test", "user_1", "sess_1", pending)
        .await
        .unwrap();

    let _: TestPending = store
        .consume("interactive_test", &token, "user_1", "sess_1")
        .await
        .expect("first consume should succeed");

    let err = store
        .consume::<TestPending>("interactive_test", &token, "user_1", "sess_1")
        .await
        .unwrap_err();
    assert!(matches!(err, PendingStoreError::NotFound));
}

#[tokio::test]
async fn invariant_4_consume_rejects_mismatched_session() {
    let store = InMemoryPendingStore::new();
    let pending = TestPending {
        verification_code: "x".into(),
    };

    let token = store
        .put("interactive_test", "user_1", "sess_1", pending)
        .await
        .unwrap();

    let err = store
        .consume::<TestPending>("interactive_test", &token, "user_1", "sess_2")
        .await
        .unwrap_err();
    assert!(matches!(err, PendingStoreError::ValidationFailed { .. }));

    let ok: TestPending = store
        .consume("interactive_test", &token, "user_1", "sess_1")
        .await
        .expect("legitimate consume should still succeed after bad probe");
    assert_eq!(ok.verification_code, "x");
}

#[test]
fn invariant_5_oauth2_pending_secrets_are_secret_string() {
    fn _compile_time_witness(p: &OAuth2Pending) {
        let _: &SecretString = &p.client_secret;
        let _: &Option<SecretString> = &p.pkce_verifier;
    }
    let _ = _compile_time_witness;
}

#[test]
fn invariant_6_pending_state_requires_zeroize() {
    fn assert_zeroize<P: zeroize::Zeroize>() {}
    assert_zeroize::<TestPending>();
    assert_zeroize::<ShortTtl>();
    assert_zeroize::<OAuth2Pending>();
}

#[tokio::test]
async fn delete_is_idempotent() {
    let store = InMemoryPendingStore::new();
    let token = PendingToken::generate();

    store.delete(&token).await.unwrap();
    store.delete(&token).await.unwrap();
}
