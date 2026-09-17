//! In-memory pending state store — **canonical storage-side home**.
//!
//! Data is lost when the store is dropped. Use this in tests and for local
//! development rather than mocking [`PendingStateStore`] directly.
//!
//! This type implements [`DynPendingStateStore`] (the object-safe byte-core
//! port); it acquires the typed [`PendingStateStore`] surface — generic over
//! `<P: PendingState>` — via the blanket impl in
//! `nebula_credential::erased`, which does the serde round-trip. The serde
//! lives in the blanket, not here: this store persists
//! `Zeroizing<Vec<u8>>` plus the binding tuple and absolute expiry.
//!
//! This is the single canonical in-memory `PendingStateStore`. A Business-tier
//! consumer that cannot dev-dep `nebula-storage` (the Exec adapter) keeps a
//! colocated `#[cfg(test)]` double instead of depending on this type.
//!
//! [`PendingStateStore`]: nebula_credential::PendingStateStore
//!
//! # Invariants
//!
//! | # | Invariant                          | Enforcement in this impl                           |
//! |---|------------------------------------|-----------------------------------------------------|
//! | 1 | Encryption at rest                 | **Moot** — process memory, no disk persistence.     |
//! |   |                                    | Durable impls must wrap via a future encrypted layer. |
//! | 2 | TTL ≤ 10 min                       | Determined per-type by `PendingState::expires_in`;  |
//! |   |                                    | expired rows are evicted on `get`/`consume` and     |
//! |   |                                    | surface as `Expired`.                               |
//! | 3 | Single-use (atomic get_then_delete)| `consume()` holds the write lock across validate    |
//! |   |                                    | + remove — no read-then-delete race.                |
//! | 4 | Session-id binding                 | 4-dimensional token binding (credential_kind,       |
//! |   |                                    | owner_id, session_id, token_id); mismatch returns   |
//! |   |                                    | `ValidationFailed` without destroying the entry.    |
//! | 5 | Typed secret fields                | Enforced at the `PendingState` implementer level    |
//! |   |                                    | (e.g. `OAuth2Pending.client_secret: SecretString`). |
//! | 6 | Zeroize on drop                    | Typed state and every serialized byte buffer use    |
//! |   |                                    | zeroizing wrappers.                                 |
//!
//! See `crates/storage/README.md` for credential persistence layout.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use std::{collections::HashMap, sync::Arc};

use chrono::Utc;
use nebula_credential::{DynPendingStateStore, PendingStoreError, PendingToken};
use tokio::sync::RwLock;
use zeroize::Zeroizing;

/// In-memory pending store backed by a `HashMap`.
///
/// Suitable for tests and local development. All data is ephemeral and
/// lost when the store is dropped.
///
/// # Examples
///
/// ```rust
/// # use std::time::Duration;
/// # use serde::{Deserialize, Serialize};
/// # use zeroize::{Zeroize, ZeroizeOnDrop};
/// use nebula_credential::PendingStateStore;
/// use nebula_storage::credential::InMemoryPendingStore;
///
/// # #[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
/// # struct MyPending {
/// #     code: String,
/// # }
/// # impl Zeroize for MyPending {
/// #     fn zeroize(&mut self) {
/// #         self.code.zeroize();
/// #     }
/// # }
/// # impl Drop for MyPending {
/// #     fn drop(&mut self) {
/// #         self.zeroize();
/// #     }
/// # }
/// # impl ZeroizeOnDrop for MyPending {}
/// # impl nebula_credential::PendingState for MyPending {
/// #     const KIND: &'static str = "oauth2";
/// #     fn expires_in(&self) -> Duration {
/// #         Duration::from_secs(300)
/// #     }
/// # }
/// let store = InMemoryPendingStore::new();
/// let pending = MyPending {
///     code: "auth-code".to_owned(),
/// };
///
/// let runtime = tokio::runtime::Runtime::new()?;
/// runtime.block_on(async {
///     // The token binds the entry to (credential_kind, owner_id, session_id);
///     // `consume` is single-use and re-checks that binding.
///     let token = store.put("oauth2", "user_1", "sess_1", pending).await?;
///     let restored: MyPending = store.consume("oauth2", &token, "user_1", "sess_1").await?;
///     assert_eq!(restored.code, "auth-code");
///     Ok::<(), Box<dyn std::error::Error>>(())
/// })?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone)]
pub struct InMemoryPendingStore {
    entries: Arc<RwLock<HashMap<String, PendingEntry>>>,
}

struct PendingEntry {
    credential_kind: String,
    owner_id: String,
    session_id: String,
    data: Zeroizing<Vec<u8>>,
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

impl DynPendingStateStore for InMemoryPendingStore {
    fn put_serialized<'a>(
        &'a self,
        credential_kind: &'a str,
        owner_id: &'a str,
        session_id: &'a str,
        data: Zeroizing<Vec<u8>>,
        expires_in: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<PendingToken, PendingStoreError>> + Send + 'a>> {
        Box::pin(async move {
            let expires_at = Utc::now() + expires_in;
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
        })
    }

    fn get_serialized<'a>(
        &'a self,
        token: &'a PendingToken,
    ) -> Pin<Box<dyn Future<Output = Result<Zeroizing<Vec<u8>>, PendingStoreError>> + Send + 'a>>
    {
        Box::pin(async move {
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
            Ok(entry.data.clone())
        })
    }

    fn get_bound_serialized<'a>(
        &'a self,
        credential_kind: &'a str,
        token: &'a PendingToken,
        owner_id: &'a str,
        session_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Zeroizing<Vec<u8>>, PendingStoreError>> + Send + 'a>>
    {
        Box::pin(async move {
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

            Ok(entry.data.clone())
        })
    }

    fn consume_serialized<'a>(
        &'a self,
        credential_kind: &'a str,
        token: &'a PendingToken,
        owner_id: &'a str,
        session_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Zeroizing<Vec<u8>>, PendingStoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            // Validate *before* removing. A wrong-owner (or otherwise
            // malformed) `consume` request must not be able to destroy the
            // legitimate user's pending state — that would turn any token
            // leak into a single-shot DoS against the in-flight flow. Hold
            // the write lock across the whole check so no concurrent consume
            // can race between validation and removal.
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

            // Only now remove the entry and return the owned bytes. The
            // get() above succeeded under the same write lock, so remove is
            // expected to yield Some; defensively surface a NotFound rather
            // than panicking in library code if a future refactor ever
            // breaks the locking discipline.
            let entry = entries
                .remove(token.as_str())
                .ok_or(PendingStoreError::NotFound)?;
            Ok(entry.data)
        })
    }

    fn delete<'a>(
        &'a self,
        token: &'a PendingToken,
    ) -> Pin<Box<dyn Future<Output = Result<(), PendingStoreError>> + Send + 'a>> {
        Box::pin(async move {
            self.entries.write().await.remove(token.as_str());
            Ok(())
        })
    }
}

#[cfg(test)]
#[path = "pending_tests.rs"]
mod tests;
