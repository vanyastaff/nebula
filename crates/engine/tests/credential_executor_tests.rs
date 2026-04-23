//! Integration tests for engine-owned credential executor wrappers.

use nebula_credential::{CredentialContext, PendingStoreError, credentials::ApiKeyCredential};
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
    let store = InMemoryPendingStore::new();
    let ctx = CredentialContext::for_test("user-1").with_session_id("sess-1");
    let bogus_token = nebula_credential::PendingToken::generate();
    let input = nebula_credential::resolve::UserInput::Poll;

    let result = nebula_engine::credential::execute_continue::<ApiKeyCredential, _>(
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
