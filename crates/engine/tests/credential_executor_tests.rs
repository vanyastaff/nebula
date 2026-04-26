//! Integration tests for engine-owned credential executor wrappers.

use nebula_credential::{
    Credential, CredentialContext, CredentialMetadata, PendingStoreError, SecretString,
    credentials::{ApiKeyCredential, OAuth2Credential},
    error::CredentialError,
    resolve::{InteractionRequest, ResolveResult},
    scheme::SecretToken,
};
use nebula_schema::FieldValues;
use nebula_storage::credential::InMemoryPendingStore;

#[tokio::test]
async fn execute_resolve_static_credential_returns_complete() {
    let store = InMemoryPendingStore::new();
    let ctx = CredentialContext::for_test("user-1");

    let mut values = FieldValues::new();
    values.set_raw("api_key", serde_json::Value::String("sk-test-key".into()));

    let result =
        nebula_engine::credential::execute_resolve::<ApiKeyCredential, _>(&values, &ctx, &store)
            .await;

    assert!(
        matches!(
            result,
            Ok(nebula_engine::credential::ResolveResponse::Complete(_))
        ),
        "expected Complete, got: {result:?}"
    );
}

#[tokio::test]
async fn execute_continue_returns_pending_store_error_for_missing_token() {
    // Per Tech Spec §15.4 `execute_continue` is bound on `Interactive`
    // — non-interactive credentials (`ApiKeyCredential`) cannot reach
    // this dispatch path at compile time (Probe 4 cements the
    // `E0277`). This test exercises the runtime "missing token" path
    // against an `Interactive` credential (`OAuth2Credential`).
    let store = InMemoryPendingStore::new();
    let ctx = CredentialContext::for_test("user-1").with_session_id("sess-1");
    let bogus_token = nebula_credential::PendingToken::generate();
    let input = nebula_credential::resolve::UserInput::Poll;

    let result = nebula_engine::credential::execute_continue::<OAuth2Credential, _>(
        &bogus_token,
        &input,
        &ctx,
        &store,
    )
    .await;

    assert!(
        matches!(
            result,
            Err(nebula_engine::credential::ExecutorError::PendingStore(
                PendingStoreError::NotFound
            ))
        ),
        "expected PendingStore NotFound error, got: {result:?}"
    );
}

// ── Test fixture: a credential whose base resolve returns Pending(()) ─────

#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop,
)]
struct DummyState {
    token: String,
}

impl nebula_credential::CredentialState for DummyState {
    const KIND: &'static str = "base_resolve_pending_test_state";
    const VERSION: u32 = 1;
}

/// Credential whose base `resolve` returns `Pending(())` — exactly the
/// shape Tech Spec §15.4 forbids the engine executor from honouring.
struct BaseResolvePendingCredential;

impl Credential for BaseResolvePendingCredential {
    type Input = FieldValues;
    type Scheme = SecretToken;
    type State = DummyState;

    const KEY: &'static str = "base_resolve_pending_test";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::new(
            nebula_core::credential_key!("base_resolve_pending_test"),
            "Base Resolve Pending Test",
            "Fixture credential exercising the §15.4 base-resolve-Pending rejection path",
            Self::schema(),
            nebula_credential::AuthPattern::SecretToken,
        )
    }

    fn project(state: &DummyState) -> SecretToken {
        SecretToken::new(SecretString::new(state.token.clone()))
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<DummyState, ()>, CredentialError> {
        // Deliberately return Pending(()) — Tech Spec §15.4 says this
        // shape MUST be rejected by `execute_resolve`. The base trait
        // cannot carry typed pending state; interactive flows go
        // through credential-specific kickoff helpers.
        Ok(ResolveResult::Pending {
            state: (),
            interaction: InteractionRequest::DisplayInfo {
                title: "test".into(),
                message: "should never reach the user".into(),
                data: nebula_credential::resolve::DisplayData::Text("test".into()),
                expires_in: None,
            },
        })
    }
}

#[tokio::test]
async fn execute_resolve_rejects_base_resolve_pending() {
    // Per Tech Spec §15.4 `execute_resolve` rejects `ResolveResult::Pending`
    // from the base `Credential::resolve` because `state: ()` cannot
    // deserialize into the typed `Interactive::Pending` later in
    // `execute_continue`. The contract is: base resolve returns
    // Complete or Retry; interactive kickoffs use credential-specific
    // helpers that populate the typed Pending directly via
    // `PendingStateStore::put`.
    let store = InMemoryPendingStore::new();
    let ctx = CredentialContext::for_test("user-1").with_session_id("sess-1");
    let values = FieldValues::new();

    let result = nebula_engine::credential::execute_resolve::<BaseResolvePendingCredential, _>(
        &values, &ctx, &store,
    )
    .await;

    assert!(
        matches!(
            result,
            Err(nebula_engine::credential::ExecutorError::BaseResolvePending)
        ),
        "expected BaseResolvePending error, got: {result:?}"
    );
}

#[tokio::test]
async fn execute_continue_rejects_missing_session_id() {
    // Per Tech Spec §15.4 the executor refuses to fall back to a
    // `"default"` session bucket; callers MUST set session_id
    // explicitly to keep concurrent owners in distinct
    // `(KEY, owner, session)` slots inside `PendingStateStore`.
    let store = InMemoryPendingStore::new();
    // Note: no `.with_session_id(...)` — exercises the missing path.
    let ctx = CredentialContext::for_test("user-1");
    let bogus_token = nebula_credential::PendingToken::generate();
    let input = nebula_credential::resolve::UserInput::Poll;

    let result = nebula_engine::credential::execute_continue::<OAuth2Credential, _>(
        &bogus_token,
        &input,
        &ctx,
        &store,
    )
    .await;

    assert!(
        matches!(
            result,
            Err(nebula_engine::credential::ExecutorError::MissingSessionId)
        ),
        "expected MissingSessionId error, got: {result:?}"
    );
}
