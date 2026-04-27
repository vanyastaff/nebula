//! v2 credential store trait with layered composition.
//!
//! Provides a CRUD abstraction for credential persistence with optimistic
//! concurrency control via [`PutMode::CompareAndSwap`]. Encryption is handled
//! by the `EncryptionLayer` wrapper (in `nebula-storage`), not by store
//! implementations themselves.

use std::future::Future;

use serde_json::Value;

/// How to handle conflicts on put.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PutMode {
    /// Fail if credential already exists.
    CreateOnly,
    /// Overwrite unconditionally.
    Overwrite,
    /// Compare-and-swap: only succeed if current version matches.
    CompareAndSwap {
        /// The version the caller last observed.
        expected_version: u64,
    },
}

/// A stored credential with metadata.
#[derive(Debug, Clone)]
pub struct StoredCredential {
    /// The credential ID.
    pub id: String,
    /// The credential type key (`Credential::KEY`), identifying which
    /// `Credential` implementation produced this stored state.
    pub credential_key: String,
    /// Serialized credential state (encrypted at the `EncryptionLayer` boundary).
    pub data: Vec<u8>,
    /// State type identifier (`CredentialState::KIND`).
    pub state_kind: String,
    /// Schema version (`CredentialState::VERSION`).
    pub state_version: u32,
    /// Monotonic version counter (for CAS).
    pub version: u64,
    /// When this credential was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When this credential was last modified.
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Optional expiration time.
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether the credential requires interactive re-authentication.
    ///
    /// Set to `true` when a refresh attempt returns
    /// [`crate::resolve::RefreshOutcome::ReauthRequired`] (provider rejected the
    /// refresh token, e.g. OAuth2 `invalid_grant`, or sentinel-threshold
    /// escalation per credential refresh sub-spec §3.4 / §3.6). Cleared
    /// (`false`) on a successful `Refreshed` outcome.
    ///
    /// Cross-replica readers (e.g. the L2 post-backoff state-recheck
    /// predicate) consult this flag to short-circuit refresh attempts that
    /// would otherwise produce a duplicate IdP rejection — preventing
    /// `O(replicas)` IdP load on a credential that has already been
    /// rejected.
    ///
    /// Persistence: backends store this either as a dedicated column or as
    /// a key in the metadata blob. Backend row structs that do use serde
    /// should annotate this field with `#[serde(default)]` so older rows
    /// missing the field deserialize as `false`. No dedicated SQL column
    /// is required on `StoredCredential` itself (which has no serde
    /// derives).
    pub reauth_required: bool,
    /// Arbitrary metadata.
    pub metadata: serde_json::Map<String, Value>,
}

/// Error from store operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// Credential not found.
    #[error("credential not found: {id}")]
    NotFound {
        /// The ID that was looked up.
        id: String,
    },
    /// Version conflict on CAS put.
    #[error("version conflict for {id}: expected {expected}, got {actual}")]
    VersionConflict {
        /// The credential ID.
        id: String,
        /// The version the caller expected.
        expected: u64,
        /// The version actually in the store.
        actual: u64,
    },
    /// Credential already exists (`CreateOnly` mode).
    #[error("credential already exists: {id}")]
    AlreadyExists {
        /// The credential ID.
        id: String,
    },
    /// Audit sink refused to record the operation. Fail-closed alarm.
    ///
    /// Per ADR-0028 invariant 4 + §14 "no discard-and-log": a failed
    /// audit sink surfaces as a hard error rather than a log-and-continue.
    /// The underlying store state depends on the operation and rollback
    /// feasibility:
    ///
    /// - `put(PutMode::CreateOnly)` — `AuditLayer` attempts a best-effort `delete` of the
    ///   freshly-inserted record before returning. On a clean rollback path, the write did not
    ///   become externally visible.
    /// - `put(PutMode::Overwrite | PutMode::CompareAndSwap)` / `delete` — no rollback. The mutation
    ///   may already be visible to concurrent readers; this error is a **fail-closed alarm**
    ///   signalling that the audit trail is compromised, not a guarantee that the mutation did not
    ///   commit.
    /// - `get` / `list` / `exists` — read path; no mutation to roll back.
    ///
    /// Consumers should treat this error as actionable (investigate the
    /// audit sink; retry only after the sink is healthy).
    #[error("audit sink refused: {0}")]
    AuditFailure(String),
    /// Backend error.
    #[error("store backend error: {0}")]
    Backend(Box<dyn std::error::Error + Send + Sync>),
}

/// Core CRUD trait for credential persistence.
///
/// Implementations handle raw bytes — encryption/decryption is done
/// by the `EncryptionLayer` wrapper (in `nebula-storage`), not by the
/// store itself.
pub trait CredentialStore: Send + Sync {
    /// Retrieve a stored credential by ID.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotFound`] if no credential with the given ID exists.
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    fn get(&self, id: &str) -> impl Future<Output = Result<StoredCredential, StoreError>> + Send;

    /// Store or update a credential.
    ///
    /// The returned [`StoredCredential`] has its `version`, `created_at`,
    /// and `updated_at` fields set by the store.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::AlreadyExists`] when `mode` is
    /// [`PutMode::CreateOnly`] and the ID already exists.
    ///
    /// Returns [`StoreError::VersionConflict`] when `mode` is
    /// [`PutMode::CompareAndSwap`] and the stored version differs.
    ///
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    fn put(
        &self,
        credential: StoredCredential,
        mode: PutMode,
    ) -> impl Future<Output = Result<StoredCredential, StoreError>> + Send;

    /// Delete a credential by ID.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotFound`] if no credential with the given ID exists.
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    fn delete(&self, id: &str) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// List credential IDs, optionally filtered by `state_kind`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    fn list(
        &self,
        state_kind: Option<&str>,
    ) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send;

    /// Check if a credential exists.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    fn exists(&self, id: &str) -> impl Future<Output = Result<bool, StoreError>> + Send;
}

/// Shared test helper for constructing [`StoredCredential`] instances.
///
/// Exposed publicly under `#[cfg(any(test, feature = "test-util"))]` so
/// sibling crates (e.g. `nebula-storage::credential::memory` tests) can
/// construct minimal instances without duplicating the builder.
#[cfg(any(test, feature = "test-util"))]
pub mod test_helpers {
    use super::StoredCredential;

    /// Build a minimal [`StoredCredential`] for testing.
    pub fn make_credential(id: &str, data: &[u8]) -> StoredCredential {
        StoredCredential {
            id: id.into(),
            credential_key: "test_credential".into(),
            data: data.to_vec(),
            state_kind: "test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            reauth_required: false,
            metadata: Default::default(),
        }
    }
}
