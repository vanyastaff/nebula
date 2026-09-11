//! Integration tests for engine-owned credential executor wrappers.

use nebula_credential::{
    Credential, CredentialContext, PendingStoreError,
    credentials::{ApiKeyCredential, OAuth2Credential},
};
use nebula_schema::{AuthoredValue, schema_of};
use nebula_storage::credential::InMemoryPendingStore;

fn resolve_properties<C: Credential>(data: serde_json::Value) -> C::Properties {
    schema_of::<C::Properties>()
        .expect("valid credential properties schema")
        .validate(AuthoredValue::from_data(data).expect("bounded credential properties data"))
        .expect("valid credential properties")
        .resolve_data()
        .expect("fully resolved credential properties")
        .into_typed_exposing_secrets()
        .expect("typed credential properties")
}

#[tokio::test]
async fn execute_resolve_static_credential_returns_complete() {
    let ctx = CredentialContext::for_owner("user-1");

    let properties = resolve_properties::<ApiKeyCredential>(serde_json::json!({
        "api_key": "sk-test-key"
    }));
    assert_eq!(properties.api_key.expose_secret(), "sk-test-key");

    let result =
        nebula_credential::runtime::execute_resolve::<ApiKeyCredential>(&properties, &ctx).await;

    match result {
        Ok(nebula_credential::runtime::ResolveResponse::Complete(state)) => {
            assert_eq!(state.token().expose_secret(), "sk-test-key");
        },
        other => panic!("expected Complete, got: {other:?}"),
    }
}

#[tokio::test]
async fn execute_continue_returns_pending_store_error_for_missing_token() {
    // Per Tech Spec `execute_continue` is bound on `Interactive`
    // — non-interactive credentials (`ApiKeyCredential`) cannot reach
    // this dispatch path at compile time (Probe 4 cements the
    // `E0277`). This test exercises the runtime "missing token" path
    // against an `Interactive` credential (`OAuth2Credential`).
    let store = InMemoryPendingStore::new();
    let ctx = CredentialContext::for_owner("user-1").with_session_id("sess-1");
    let bogus_token = nebula_credential::PendingToken::generate();
    let input = nebula_credential::resolve::UserInput::Poll;

    let result = nebula_credential::runtime::execute_continue::<OAuth2Credential, _>(
        &bogus_token,
        &input,
        &ctx,
        &store,
    )
    .await;

    assert!(
        matches!(
            result,
            Err(nebula_credential::runtime::ExecutorError::PendingStore(
                PendingStoreError::NotFound
            ))
        ),
        "expected PendingStore NotFound error, got: {result:?}"
    );
}

#[tokio::test]
async fn execute_continue_rejects_missing_session_id() {
    // Per Tech Spec the executor refuses to fall back to a
    // `"default"` session bucket; callers MUST set session_id
    // explicitly to keep concurrent owners in distinct
    // `(KEY, owner, session)` slots inside `PendingStateStore`.
    let store = InMemoryPendingStore::new();
    // Note: no `.with_session_id(...)` — exercises the missing path.
    let ctx = CredentialContext::for_owner("user-1");
    let bogus_token = nebula_credential::PendingToken::generate();
    let input = nebula_credential::resolve::UserInput::Poll;

    let result = nebula_credential::runtime::execute_continue::<OAuth2Credential, _>(
        &bogus_token,
        &input,
        &ctx,
        &store,
    )
    .await;

    assert!(
        matches!(
            result,
            Err(nebula_credential::runtime::ExecutorError::MissingSessionId)
        ),
        "expected MissingSessionId error, got: {result:?}"
    );
}
