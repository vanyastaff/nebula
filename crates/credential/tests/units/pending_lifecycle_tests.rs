//! Integration test: pending state lifecycle (interactive credential flow).
//!
//! Validates the full interactive credential resolution flow:
//! `execute_resolve` → `Pending` → `execute_continue` → `Complete`.

use std::time::Duration;

use nebula_credential::{
    SecretString,
    context::CredentialContext,
    credential::Credential,
    description::CredentialDescription,
    error::CredentialError,
    executor::{ResolveResponse, execute_continue, execute_resolve},
    pending::PendingState,
    pending_store_memory::InMemoryPendingStore,
    resolve::{DisplayData, InteractionRequest, RefreshOutcome, ResolveResult, UserInput},
    scheme::SecretToken,
};
use nebula_parameter::{ParameterCollection, values::ParameterValues};

// ── Test pending state ───────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TestPending {
    verification_code: String,
}

impl zeroize::Zeroize for TestPending {
    fn zeroize(&mut self) {
        self.verification_code.zeroize();
    }
}

impl PendingState for TestPending {
    const KIND: &'static str = "test_interactive_pending";

    fn expires_in(&self) -> Duration {
        Duration::from_secs(300)
    }
}

// ── Test credential state ────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TestInteractiveState {
    token: String,
}

impl nebula_credential::state::CredentialState for TestInteractiveState {
    const KIND: &'static str = "interactive_test";
    const VERSION: u32 = 1;
}

// ── Test credential type ─────────────────────────────────────────────

struct InteractiveTestCredential;

impl Credential for InteractiveTestCredential {
    type Scheme = SecretToken;
    type State = TestInteractiveState;
    type Pending = TestPending;

    const KEY: &'static str = "interactive_test";
    const INTERACTIVE: bool = true;

    fn description() -> CredentialDescription {
        CredentialDescription {
            key: Self::KEY.to_owned(),
            name: "Interactive Test".to_owned(),
            description: "Test credential for pending lifecycle".to_owned(),
            icon: None,
            icon_url: None,
            documentation_url: None,
            properties: Self::parameters(),
            pattern: nebula_core::AuthPattern::SecretToken,
        }
    }

    fn parameters() -> ParameterCollection {
        ParameterCollection::new()
    }

    fn project(state: &TestInteractiveState) -> SecretToken {
        SecretToken::new(SecretString::new(state.token.clone()))
    }

    async fn resolve(
        _values: &ParameterValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<TestInteractiveState, TestPending>, CredentialError> {
        Ok(ResolveResult::Pending {
            state: TestPending {
                verification_code: "secret-code-123".into(),
            },
            interaction: InteractionRequest::DisplayInfo {
                title: "Enter Code".into(),
                message: "Please enter the verification code".into(),
                data: DisplayData::Text("Check your email for the code".into()),
                expires_in: Some(300),
            },
        })
    }

    async fn continue_resolve(
        pending: &TestPending,
        input: &UserInput,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<TestInteractiveState, TestPending>, CredentialError> {
        // Verify the user provided the correct code
        match input {
            UserInput::Code { code } if code == &pending.verification_code => {
                Ok(ResolveResult::Complete(TestInteractiveState {
                    token: "final-token".into(),
                }))
            }
            _ => Err(CredentialError::InvalidInput(
                "incorrect verification code".into(),
            )),
        }
    }

    async fn refresh(
        _state: &mut TestInteractiveState,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        Ok(RefreshOutcome::NotSupported)
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn pending_lifecycle_resolve_then_continue() {
    let pending_store = InMemoryPendingStore::new();
    let ctx = CredentialContext::new("test-user").with_session_id("sess-1");
    let values = ParameterValues::new();

    // Step 1: initial resolve → should return Pending
    let response = execute_resolve::<InteractiveTestCredential, _>(&values, &ctx, &pending_store)
        .await
        .expect("execute_resolve should succeed");

    let token = match response {
        ResolveResponse::Pending { token, interaction } => {
            // Verify the interaction request matches what we returned
            assert!(
                matches!(interaction, InteractionRequest::DisplayInfo { ref title, .. } if title == "Enter Code"),
                "expected DisplayInfo interaction, got: {interaction:?}"
            );
            token
        }
        other => panic!("expected Pending response, got: {other:?}"),
    };

    // Step 2: continue with correct code → should return Complete
    let input = UserInput::Code {
        code: "secret-code-123".into(),
    };
    let response =
        execute_continue::<InteractiveTestCredential, _>(&token, &input, &ctx, &pending_store)
            .await
            .expect("execute_continue should succeed");

    match response {
        ResolveResponse::Complete(state) => {
            assert_eq!(state.token, "final-token");
        }
        other => panic!("expected Complete response, got: {other:?}"),
    }
}

#[tokio::test]
async fn pending_token_is_single_use() {
    let pending_store = InMemoryPendingStore::new();
    let ctx = CredentialContext::new("test-user").with_session_id("sess-1");
    let values = ParameterValues::new();

    // Resolve → Pending
    let response = execute_resolve::<InteractiveTestCredential, _>(&values, &ctx, &pending_store)
        .await
        .unwrap();
    let token = match response {
        ResolveResponse::Pending { token, .. } => token,
        other => panic!("expected Pending, got: {other:?}"),
    };

    // First continue → should succeed
    let input = UserInput::Code {
        code: "secret-code-123".into(),
    };
    let result =
        execute_continue::<InteractiveTestCredential, _>(&token, &input, &ctx, &pending_store)
            .await;
    assert!(result.is_ok(), "first continue should succeed");

    // Second continue with same token → should fail (single-use)
    let result =
        execute_continue::<InteractiveTestCredential, _>(&token, &input, &ctx, &pending_store)
            .await;
    assert!(result.is_err(), "second continue should fail (single-use)");
}

#[tokio::test]
async fn continue_with_wrong_code_returns_error() {
    let pending_store = InMemoryPendingStore::new();
    let ctx = CredentialContext::new("test-user").with_session_id("sess-1");
    let values = ParameterValues::new();

    // Resolve → Pending
    let response = execute_resolve::<InteractiveTestCredential, _>(&values, &ctx, &pending_store)
        .await
        .unwrap();
    let token = match response {
        ResolveResponse::Pending { token, .. } => token,
        other => panic!("expected Pending, got: {other:?}"),
    };

    // Continue with wrong code → should return credential error
    let input = UserInput::Code {
        code: "wrong-code".into(),
    };
    let result =
        execute_continue::<InteractiveTestCredential, _>(&token, &input, &ctx, &pending_store)
            .await;
    assert!(
        result.is_err(),
        "continue with wrong code should fail: {result:?}"
    );
}
