//! Composable storage layers for [`CredentialStore`](crate::CredentialStore).
//!
//! Three decorator families live here (ADR-0092 step 3 — relocated from
//! `nebula-storage::credential::layer`):
//!
//! - [`EncryptionLayer`] — AES-256-GCM at-rest encryption, keyed by
//!   [`KeyProvider`]. Rewired onto the [`nebula_crypto::Cipher`] port so the
//!   algorithm is injected rather than hard-coded.
//! - [`CacheLayer`] — moka LRU+TTL cache sitting *below* `EncryptionLayer`,
//!   caching ciphertext so plaintext secrets are never cached.
//! - [`AuditLayer`] — fail-closed audit via [`AuditSink`]; no discard-and-log.
//!
//! The multi-tenant scope layer is re-homed to
//! `nebula_tenancy::CredentialScopeLayer` (spec §8) — scope policy does not
//! belong in the credential contract crate.
//!
//! The `KeyProvider` trait and its built-in impls (`EnvKeyProvider`,
//! `FileKeyProvider`, and the test-only `StaticKeyProvider`) live in the
//! [`key_provider`] submodule.

pub mod audit;
pub mod cache;
pub mod encryption;
pub mod key_provider;

pub use audit::{AuditEvent, AuditLayer, AuditOperation, AuditResult, AuditSink};
pub use cache::{CacheConfig, CacheLayer, CacheStats};
pub use encryption::EncryptionLayer;
#[cfg(any(test, feature = "test-util"))]
pub use key_provider::StaticKeyProvider;
pub use key_provider::{EnvKeyProvider, FileKeyProvider, KeyProvider, ProviderError};

// ============================================================================
// In-memory CredentialStore double (test + test-util)
//
// Lives here rather than in `store::test_helpers` because:
//   1. The decorator tests need a full CAS-correct `CredentialStore` impl.
//   2. The canonical sqlx-backed impls (`SqliteCredentialStore` /
//      `PgCredentialStore`) live in `nebula-storage` — importing them from
//      `nebula-credential` would create a cycle (storage → credential, but
//      then credential → storage).
//   3. A Business-tier consumer that cannot depend on storage can colocate
//      its own `#[cfg(test)]` double; this one is the canonical double for
//      the credential crate's own tests.
// ============================================================================

/// Minimal in-memory [`crate::CredentialStore`] for use in unit tests.
///
/// Provides full CAS semantics (`version` monotonically incremented on every
/// write) so decorator tests can exercise lazy re-encryption and version
/// propagation without a SQLite or Postgres dependency. The inner store is
/// `Clone`-able so tests can hold a second handle to inspect raw (pre-layer)
/// stored bytes — matching the pattern used by the SQLite tests it replaces.
///
/// Gated behind `#[cfg(any(test, feature = "test-util"))]` so it is absent
/// from production release builds (ADR-0023).
#[cfg(any(test, feature = "test-util"))]
#[derive(Clone, Default)]
pub struct InMemoryCredentialStore {
    // `Arc<std::sync::Mutex<_>>` is the right tool here: the store is shared
    // across `Clone`d handles (the `Clone` is the whole point — tests hold
    // both the inner store and the wrapped layer) and the critical section is
    // tiny (HashMap ops only, no async work inside). A `tokio::sync::Mutex`
    // would be unnecessary overhead for a test double.
    inner: std::sync::Arc<std::sync::Mutex<InMemoryState>>,
}

#[cfg(any(test, feature = "test-util"))]
#[derive(Default)]
struct InMemoryState {
    rows: std::collections::HashMap<String, crate::StoredCredential>,
}

#[cfg(any(test, feature = "test-util"))]
impl InMemoryCredentialStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(any(test, feature = "test-util"))]
impl crate::CredentialStore for InMemoryCredentialStore {
    async fn get(&self, id: &str) -> Result<crate::StoredCredential, crate::StoreError> {
        self.inner
            .lock()
            .expect(
                "InMemoryCredentialStore mutex must not be poisoned — only tests hold this lock",
            )
            .rows
            .get(id)
            .cloned()
            .ok_or_else(|| crate::StoreError::NotFound { id: id.to_string() })
    }

    async fn put(
        &self,
        mut credential: crate::StoredCredential,
        mode: crate::PutMode,
    ) -> Result<crate::StoredCredential, crate::StoreError> {
        let mut state = self.inner.lock().expect(
            "InMemoryCredentialStore mutex must not be poisoned — only tests hold this lock",
        );

        let now = chrono::Utc::now();

        match mode {
            crate::PutMode::CreateOnly => {
                if state.rows.contains_key(&credential.id) {
                    return Err(crate::StoreError::AlreadyExists { id: credential.id });
                }
                credential.version = 1;
                credential.created_at = now;
                credential.updated_at = now;
                state.rows.insert(credential.id.clone(), credential.clone());
                Ok(credential)
            },
            crate::PutMode::Overwrite => {
                let next_version = state
                    .rows
                    .get(&credential.id)
                    .map_or(1, |existing| existing.version + 1);
                credential.version = next_version;
                credential.updated_at = now;
                if !state.rows.contains_key(&credential.id) {
                    credential.created_at = now;
                }
                state.rows.insert(credential.id.clone(), credential.clone());
                Ok(credential)
            },
            crate::PutMode::CompareAndSwap { expected_version } => {
                match state.rows.get(&credential.id) {
                    None => Err(crate::StoreError::NotFound {
                        id: credential.id.clone(),
                    }),
                    Some(existing) if existing.version != expected_version => {
                        Err(crate::StoreError::VersionConflict {
                            id: credential.id.clone(),
                            expected: expected_version,
                            actual: existing.version,
                        })
                    },
                    Some(existing) => {
                        credential.version = existing.version + 1;
                        credential.created_at = existing.created_at;
                        credential.updated_at = now;
                        state.rows.insert(credential.id.clone(), credential.clone());
                        Ok(credential)
                    },
                }
            },
        }
    }

    async fn delete(&self, id: &str) -> Result<(), crate::StoreError> {
        let mut state = self.inner.lock().expect(
            "InMemoryCredentialStore mutex must not be poisoned — only tests hold this lock",
        );
        if state.rows.remove(id).is_some() {
            Ok(())
        } else {
            Err(crate::StoreError::NotFound { id: id.to_string() })
        }
    }

    async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, crate::StoreError> {
        let state = self.inner.lock().expect(
            "InMemoryCredentialStore mutex must not be poisoned — only tests hold this lock",
        );
        let ids = state
            .rows
            .values()
            .filter(|c| state_kind.is_none_or(|k| c.state_kind == k))
            .map(|c| c.id.clone())
            .collect();
        Ok(ids)
    }

    async fn exists(&self, id: &str) -> Result<bool, crate::StoreError> {
        let state = self.inner.lock().expect(
            "InMemoryCredentialStore mutex must not be poisoned — only tests hold this lock",
        );
        Ok(state.rows.contains_key(id))
    }
}
