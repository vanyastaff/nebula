//! In-memory pending state store — **behaviour-identical shim** of the
//! canonical impl at `nebula_storage::credential::pending::InMemoryPendingStore`
//! (ADR-0029 §4 / ADR-0032 §7).
//!
//! # Why a dual-home shim?
//!
//! Credential's own `executor.rs` `#[cfg(test)] mod tests` uses an
//! `InMemoryPendingStore` to exercise `execute_resolve` / `execute_continue`
//! against a real `PendingStateStore`. Those tests cannot dev-dep on
//! `nebula-storage`: that dev-dep path produces a two-copies cargo
//! resolution, which means the `PendingStateStore` trait bound on the
//! generic executor no longer accepts the storage-side type
//! (empirically confirmed in P6.2 with `InMemoryStore`).
//!
//! The fix is the same narrowly-scoped exception ADR-0032 §7 carved out
//! for `store_memory`: keep a body-identical copy in credential for
//! internal tests; production consumers and composition roots prefer the
//! storage-side canonical home.
//!
//! If you are adding new production code, reach for
//! `nebula_storage::credential::InMemoryPendingStore`. Only credential's
//! internal `#[cfg(test)]` code should touch this module.
//!
//! Ref: `docs/adr/0032-credential-store-canonical-home.md` §7

use std::{collections::HashMap, sync::Arc};

use chrono::Utc;
use tokio::sync::RwLock;

use crate::{
    PendingState, PendingToken,
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
        let mut entries = self.entries.write().await;
        let entry = entries
            .get(token.as_str())
            .ok_or(PendingStoreError::NotFound)?;

        if Utc::now() > entry.expires_at {
            // Expiry is deterministic; evict here too so repeated `get`
            // probes cannot retain stale rows forever.
            entries.remove(token.as_str());
            return Err(PendingStoreError::Expired);
        }
        let data = entry.data.clone();
        drop(entries);

        serde_json::from_slice(&data).map_err(|e| PendingStoreError::Backend(Box::new(e)))
    }

    async fn get_bound<P: PendingState>(
        &self,
        credential_kind: &str,
        token: &PendingToken,
        owner_id: &str,
        session_id: &str,
    ) -> Result<P, PendingStoreError> {
        let mut entries = self.entries.write().await;
        let entry = entries
            .get(token.as_str())
            .ok_or(PendingStoreError::NotFound)?;

        if Utc::now() > entry.expires_at {
            entries.remove(token.as_str());
            return Err(PendingStoreError::Expired);
        }

        let mismatch = entry.credential_kind != credential_kind
            || entry.owner_id != owner_id
            || entry.session_id != session_id;
        if mismatch {
            return Err(PendingStoreError::ValidationFailed {
                reason: "token bindings do not match".to_owned(),
            });
        }

        let data = entry.data.clone();
        drop(entries);

        serde_json::from_slice(&data).map_err(|e| PendingStoreError::Backend(Box::new(e)))
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
            Duration::from_mins(5)
        }
    }

    fn test_pending(data: &str) -> TestPending {
        TestPending {
            data: data.to_owned(),
        }
    }

    // The full invariant / behaviour test matrix lives next to the
    // canonical impl in `nebula_storage::credential::pending::tests` and
    // in the integration test `crates/engine/tests/credential_pending_lifecycle_tests.rs`.
    // Credential
    // only retains a smoke test here — enough to catch shim drift without
    // duplicating the matrix.
    #[tokio::test]
    async fn shim_put_and_consume_roundtrip() {
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
}
