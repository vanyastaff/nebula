//! In-memory pending state store for testing and development.
//!
//! Data is lost when the store is dropped. Use this in tests rather
//! than mocking [`PendingStateStore`] directly.

use std::{collections::HashMap, sync::Arc};

use chrono::Utc;
use tokio::sync::RwLock;

use crate::{
    pending::{PendingState, PendingToken},
    pending_store::{PendingStateStore, PendingStoreError},
};

/// In-memory pending store backed by a `HashMap`.
///
/// Suitable for tests and local development. All data is ephemeral and
/// lost when the store is dropped.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_credential::InMemoryPendingStore;
///
/// let store = InMemoryPendingStore::new();
/// let token = store.put("oauth2", "user_1", "sess_1", pending).await?;
/// let state = store.consume::<MyPending>("oauth2", &token, "user_1", "sess_1").await?;
/// ```
#[derive(Clone)]
pub struct InMemoryPendingStore {
    entries: Arc<RwLock<HashMap<String, PendingEntry>>>,
}

struct PendingEntry {
    credential_kind: String,
    owner_id: String,
    session_id: String,
    data: Vec<u8>,
    expires_at: chrono::DateTime<Utc>,
}

impl std::fmt::Debug for InMemoryPendingStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryPendingStore")
            .finish_non_exhaustive()
    }
}

impl InMemoryPendingStore {
    /// Creates a new empty in-memory pending store.
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryPendingStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingStateStore for InMemoryPendingStore {
    async fn put<P: PendingState>(
        &self,
        credential_kind: &str,
        owner_id: &str,
        session_id: &str,
        pending: P,
    ) -> Result<PendingToken, PendingStoreError> {
        let data =
            serde_json::to_vec(&pending).map_err(|e| PendingStoreError::Backend(Box::new(e)))?;
        let expires_at = Utc::now() + pending.expires_in();
        let token = PendingToken::generate();

        let entry = PendingEntry {
            credential_kind: credential_kind.to_owned(),
            owner_id: owner_id.to_owned(),
            session_id: session_id.to_owned(),
            data,
            expires_at,
        };

        self.entries
            .write()
            .await
            .insert(token.as_str().to_owned(), entry);

        Ok(token)
    }

    async fn get<P: PendingState>(&self, token: &PendingToken) -> Result<P, PendingStoreError> {
        let entries = self.entries.read().await;
        let entry = entries
            .get(token.as_str())
            .ok_or(PendingStoreError::NotFound)?;

        if Utc::now() > entry.expires_at {
            return Err(PendingStoreError::Expired);
        }

        serde_json::from_slice(&entry.data).map_err(|e| PendingStoreError::Backend(Box::new(e)))
    }

    async fn consume<P: PendingState>(
        &self,
        credential_kind: &str,
        token: &PendingToken,
        owner_id: &str,
        session_id: &str,
    ) -> Result<P, PendingStoreError> {
        // Validate *before* removing. A wrong-owner (or otherwise malformed)
        // `consume` request must not be able to destroy the legitimate
        // user's pending state — that would turn any token leak into a
        // single-shot DoS against the in-flight flow. Hold the write lock
        // across the whole check so no concurrent consume can race between
        // validation and removal.
        let mut entries = self.entries.write().await;

        let entry = entries
            .get(token.as_str())
            .ok_or(PendingStoreError::NotFound)?;

        if Utc::now() > entry.expires_at {
            // Expiry is deterministic; it's safe to evict the stale row now.
            entries.remove(token.as_str());
            return Err(PendingStoreError::Expired);
        }

        // All three binding checks are folded into one OR so the failure
        // path is indistinguishable and does not hint at which dimension
        // mismatched (cheap mitigation for a timing/oracle probe).
        let mismatch = entry.credential_kind != credential_kind
            || entry.owner_id != owner_id
            || entry.session_id != session_id;
        if mismatch {
            // Intentionally leave the entry in place so the legitimate
            // caller can still consume it.
            return Err(PendingStoreError::ValidationFailed {
                reason: "token bindings do not match".to_owned(),
            });
        }

        // Only now remove the entry and deserialize from the owned bytes.
        let entry = entries
            .remove(token.as_str())
            .expect("entry was just validated via get() under the same lock");
        drop(entries);

        serde_json::from_slice(&entry.data).map_err(|e| PendingStoreError::Backend(Box::new(e)))
    }

    async fn delete(&self, token: &PendingToken) -> Result<(), PendingStoreError> {
        self.entries.write().await.remove(token.as_str());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde::{Deserialize, Serialize};
    use zeroize::Zeroize;

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

    impl PendingState for TestPending {
        const KIND: &'static str = "test_pending";

        fn expires_in(&self) -> Duration {
            Duration::from_secs(300)
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

        // Deleting a non-existent token should succeed.
        store.delete(&token).await.unwrap();
        store.delete(&token).await.unwrap();
    }
}
