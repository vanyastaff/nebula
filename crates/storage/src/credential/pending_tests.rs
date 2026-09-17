use std::time::Duration;

// The typed `put`/`get`/`consume::<T>` surface comes from the blanket
// `PendingStateStore` impl on every `DynPendingStateStore`; pull both
// traits into scope so the tests exercise the typed API.
use nebula_credential::{PendingState, PendingStateStore};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct TestPending {
    data: String,
}

impl Zeroize for TestPending {
    fn zeroize(&mut self) {
        self.data.zeroize();
    }
}

// Per Tech Spec §15.4 — `PendingState: ZeroizeOnDrop`. Hand-rolled
// because the manual `Zeroize` body above would conflict with a
// derived `Drop`; this delegates Drop to the existing zeroize logic.
impl Drop for TestPending {
    fn drop(&mut self) {
        self.zeroize();
    }
}
impl ZeroizeOnDrop for TestPending {}

impl PendingState for TestPending {
    const KIND: &'static str = "test_pending";

    fn expires_in(&self) -> Duration {
        Duration::from_mins(5)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct ShortLivedPending {
    data: String,
}

impl Zeroize for ShortLivedPending {
    fn zeroize(&mut self) {
        self.data.zeroize();
    }
}

// Per Tech Spec §15.4 — `PendingState: ZeroizeOnDrop`. Same hand-roll
// rationale as `TestPending` above.
impl Drop for ShortLivedPending {
    fn drop(&mut self) {
        self.zeroize();
    }
}
impl ZeroizeOnDrop for ShortLivedPending {}

impl PendingState for ShortLivedPending {
    const KIND: &'static str = "short_lived";

    fn expires_in(&self) -> Duration {
        Duration::ZERO
    }
}

fn test_pending(data: &str) -> TestPending {
    TestPending {
        data: data.to_owned(),
    }
}

#[tokio::test]
async fn serialized_byte_core_keeps_zeroizing_buffers_end_to_end() {
    let store = InMemoryPendingStore::new();
    let serialized = Zeroizing::new(br#"{"data":"pending-secret-canary"}"#.to_vec());
    let token = DynPendingStateStore::put_serialized(
        &store,
        "oauth2",
        "user_1",
        "sess_1",
        serialized,
        Duration::from_mins(5),
    )
    .await
    .expect("serialized pending state stores");

    {
        let entries = store.entries.read().await;
        let entry = entries
            .get(token.as_str())
            .expect("pending entry remains present");
        let _: &Zeroizing<Vec<u8>> = &entry.data;
    }

    let restored: Zeroizing<Vec<u8>> = DynPendingStateStore::get_serialized(&store, &token)
        .await
        .expect("serialized pending state reads");
    assert!(restored.windows(6).any(|window| window == b"canary"));
}

#[tokio::test]
async fn put_and_consume_roundtrip() {
    let store = InMemoryPendingStore::new();
    let pending = test_pending("hello");

    let token = store
        .put("oauth2", "user_1", "sess_1", pending.clone())
        .await
        .unwrap();

    let result: TestPending = store
        .consume("oauth2", &token, "user_1", "sess_1")
        .await
        .unwrap();

    assert_eq!(result, pending);
}

#[tokio::test]
async fn consume_rejects_wrong_credential_kind_and_preserves_entry() {
    let store = InMemoryPendingStore::new();
    let token = store
        .put("oauth2", "user_1", "sess_1", test_pending("x"))
        .await
        .unwrap();

    let err = store
        .consume::<TestPending>("api_key", &token, "user_1", "sess_1")
        .await
        .unwrap_err();
    assert!(matches!(err, PendingStoreError::ValidationFailed { .. }));

    // Entry must still be consumable by the legitimate caller — a
    // wrong-kind probe must not destroy pending state.
    let ok: TestPending = store
        .consume("oauth2", &token, "user_1", "sess_1")
        .await
        .expect("legitimate consume should still succeed after bad probe");
    assert_eq!(ok.data, "x");
}

#[tokio::test]
async fn consume_rejects_wrong_owner_and_preserves_entry() {
    let store = InMemoryPendingStore::new();
    let token = store
        .put("oauth2", "user_1", "sess_1", test_pending("x"))
        .await
        .unwrap();

    let err = store
        .consume::<TestPending>("oauth2", &token, "user_2", "sess_1")
        .await
        .unwrap_err();
    assert!(matches!(err, PendingStoreError::ValidationFailed { .. }));

    let ok: TestPending = store
        .consume("oauth2", &token, "user_1", "sess_1")
        .await
        .expect("legitimate consume should still succeed after bad probe");
    assert_eq!(ok.data, "x");
}

#[tokio::test]
async fn consume_rejects_wrong_session_and_preserves_entry() {
    let store = InMemoryPendingStore::new();
    let token = store
        .put("oauth2", "user_1", "sess_1", test_pending("x"))
        .await
        .unwrap();

    let err = store
        .consume::<TestPending>("oauth2", &token, "user_1", "sess_2")
        .await
        .unwrap_err();
    assert!(matches!(err, PendingStoreError::ValidationFailed { .. }));

    let ok: TestPending = store
        .consume("oauth2", &token, "user_1", "sess_1")
        .await
        .expect("legitimate consume should still succeed after bad probe");
    assert_eq!(ok.data, "x");
}

#[tokio::test]
async fn consume_deletes_entry() {
    let store = InMemoryPendingStore::new();
    let token = store
        .put("oauth2", "user_1", "sess_1", test_pending("x"))
        .await
        .unwrap();

    let _: TestPending = store
        .consume("oauth2", &token, "user_1", "sess_1")
        .await
        .unwrap();

    let err = store
        .consume::<TestPending>("oauth2", &token, "user_1", "sess_1")
        .await
        .unwrap_err();

    assert!(matches!(err, PendingStoreError::NotFound));
}

#[tokio::test]
async fn get_does_not_delete_entry() {
    let store = InMemoryPendingStore::new();
    let pending = test_pending("repeatable");
    let token = store
        .put("oauth2", "user_1", "sess_1", pending.clone())
        .await
        .unwrap();

    let first: TestPending = store.get(&token).await.unwrap();
    let second: TestPending = store.get(&token).await.unwrap();

    assert_eq!(first, pending);
    assert_eq!(second, pending);
}

#[tokio::test]
async fn expired_entry_returns_error_on_get() {
    let store = InMemoryPendingStore::new();
    let pending = ShortLivedPending {
        data: "ephemeral".to_owned(),
    };
    let token = store
        .put("oauth2", "user_1", "sess_1", pending)
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(5)).await;

    let err = store.get::<ShortLivedPending>(&token).await.unwrap_err();
    assert!(matches!(err, PendingStoreError::Expired));
    assert_eq!(store.entries.read().await.len(), 0);

    // Once evicted, later reads are a clean miss instead of repeatedly
    // returning Expired while retaining memory.
    let second = store.get::<ShortLivedPending>(&token).await.unwrap_err();
    assert!(matches!(second, PendingStoreError::NotFound));
}

#[tokio::test]
async fn expired_entry_returns_error_on_consume() {
    let store = InMemoryPendingStore::new();
    let pending = ShortLivedPending {
        data: "ephemeral".to_owned(),
    };
    let token = store
        .put("oauth2", "user_1", "sess_1", pending)
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(5)).await;

    let err = store
        .consume::<ShortLivedPending>("oauth2", &token, "user_1", "sess_1")
        .await
        .unwrap_err();

    assert!(matches!(err, PendingStoreError::Expired));
}

#[tokio::test]
async fn delete_is_idempotent() {
    let store = InMemoryPendingStore::new();
    let token = PendingToken::generate();

    // Deleting a non-existent token should succeed. `delete` is the one
    // method present on both `DynPendingStateStore` (byte core) and the
    // blanket `PendingStateStore`, so disambiguate to the typed surface.
    assert!(PendingStateStore::delete(&store, &token).await.is_ok());
    assert!(PendingStateStore::delete(&store, &token).await.is_ok());
}
