//! Audit logging layer for credential operations.
//!
//! Logs access patterns (who accessed what, when, result) without
//! ever seeing plaintext credential data. Sits above EncryptionLayer
//! in the layer stack.
//!
//! # Design
//!
//! `AuditLayer` wraps any [`CredentialStore`] and delegates every operation
//! to the inner store, emitting an [`AuditEvent`] to the pluggable
//! [`AuditSink`] for each call. Only metadata flows through the sink —
//! credential data never does.
//!
//! # Fail-closed invariant (ADR-0028 §Decision §4)
//!
//! Audit is **in-line durable**: if [`AuditSink::record`] returns an
//! error, the credential operation as a whole returns
//! [`StoreError::AuditFailure`]. There is no "log-and-continue" path.
//! For mutating operations (`put`, `delete`), the layer additionally
//! attempts a best-effort rollback of the inner write when possible
//! (for `PutMode::CreateOnly` puts, it `delete`s the freshly-inserted
//! record) so the store ends in the pre-call state.
//!
//! This is the non-negotiable §14 "no discard-and-log" rule. The
//! [`credential_audit_durable`](../../../../tests/credential_audit_durable.rs)
//! integration test is the CI gate for this invariant.

use std::sync::Arc;

use nebula_credential::{CredentialStore, PutMode, StoreError, StoredCredential};

/// Receives audit events for logging or persistence.
///
/// Implementations might write to a file, send to an event bus, or
/// collect events in memory for testing.
///
/// # Contract
///
/// - `record` must not block the calling task for extended periods.
/// - Implementations must never inspect or log credential data.
/// - Returning `Err(StoreError)` causes the wrapping [`AuditLayer`] to fail the whole credential
///   operation with [`StoreError::AuditFailure`] (fail-closed per ADR-0028 inv 4).
pub trait AuditSink: Send + Sync {
    /// Record an audit event.
    ///
    /// # Errors
    ///
    /// Return an error when the event cannot be durably persisted.
    /// The wrapping [`AuditLayer`] will surface this as
    /// [`StoreError::AuditFailure`] — no silent discard.
    fn record(&self, event: &AuditEvent) -> Result<(), StoreError>;
}

/// A credential store operation recorded for audit purposes.
///
/// Contains only metadata — never credential data or secrets.
#[derive(Debug, Clone)]
pub struct AuditEvent {
    /// When the operation occurred.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// The credential ID involved (`"*"` for list operations).
    pub credential_id: String,
    /// What operation was performed.
    pub operation: AuditOperation,
    /// Outcome of the operation.
    pub result: AuditResult,
}

/// Type of credential store operation.
///
/// Variants without payloads describe `CredentialStore` operations
/// flowing through [`AuditLayer`]. Variants prefixed `RefreshCoord*`
/// describe events emitted by the engine's two-tier refresh coordinator
/// (sub-spec
/// `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md`
/// §6) and carry their structured payload as enum fields. The same
/// [`AuditSink`] receives both families so operators reuse one sink
/// implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuditOperation {
    /// A credential was retrieved.
    Get,
    /// A credential was stored or updated.
    Put,
    /// A credential was deleted.
    Delete,
    /// Credential IDs were listed.
    List,
    /// A credential existence check was performed.
    Exists,
    /// L2 refresh claim acquired by `holder` for `ttl_secs`.
    /// Sub-spec §6 audit event.
    RefreshCoordClaimAcquired {
        /// Replica that holds the claim.
        holder: String,
        /// TTL applied to the claim row.
        ttl_secs: u64,
    },
    /// Sentinel event recorded for a credential — a holder crashed
    /// mid-refresh and the reclaim sweep observed the residual
    /// `RefreshInFlight` state. `recent_count` is the rolling-window
    /// count after the new event was inserted. Sub-spec §6 audit event.
    RefreshCoordSentinelTriggered {
        /// Sentinel event count in the rolling window after this
        /// detection (includes the new event).
        recent_count: u32,
    },
    /// Credential transitioned to `ReauthRequired` after the sentinel
    /// threshold was crossed. `reason` is the textual form of the
    /// `ReauthReason` published on the event bus. Sub-spec §6 audit
    /// event.
    RefreshCoordReauthFlagged {
        /// Sanitized reason string. For sentinel-driven escalations:
        /// `"sentinel_repeated"` (the `ReauthReason::SentinelRepeated`
        /// arm's stable identifier).
        reason: String,
    },
}

/// Outcome of an audited operation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuditResult {
    /// The operation completed successfully.
    Success,
    /// The requested credential was not found.
    NotFound,
    /// A version or existence conflict occurred.
    Conflict,
    /// The operation failed with a sanitized error message (no secrets).
    Error(String),
}

/// Audit logging layer wrapping a [`CredentialStore`].
///
/// Delegates every operation to the inner store and records an
/// [`AuditEvent`] via the configured [`AuditSink`]. When the sink
/// errors the operation fails with [`StoreError::AuditFailure`]; for
/// `put` / `delete` the layer also attempts a best-effort rollback of
/// the inner write (see module docs).
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_storage::credential::{AuditLayer, InMemoryStore};
/// use std::sync::Arc;
///
/// let sink = Arc::new(my_audit_sink);
/// let store = AuditLayer::new(InMemoryStore::new(), sink);
/// ```
pub struct AuditLayer<S> {
    inner: S,
    sink: Arc<dyn AuditSink>,
}

impl<S> AuditLayer<S> {
    /// Create a new audit layer wrapping the given store.
    pub fn new(inner: S, sink: Arc<dyn AuditSink>) -> Self {
        Self { inner, sink }
    }
}

impl<S: CredentialStore> CredentialStore for AuditLayer<S> {
    async fn get(&self, id: &str) -> Result<StoredCredential, StoreError> {
        let result = self.inner.get(id).await;
        self.sink.record(&AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: id.to_string(),
            operation: AuditOperation::Get,
            result: audit_result(&result),
        })?;
        result
    }

    async fn put(
        &self,
        credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, StoreError> {
        let id = credential.id.clone();
        let result = self.inner.put(credential, mode).await;

        let event = AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: id.clone(),
            operation: AuditOperation::Put,
            result: audit_result(&result),
        };

        if let Err(sink_err) = self.sink.record(&event) {
            // ADR-0028 §4: fail-closed. Best-effort rollback of the
            // inner write so the store ends in the pre-call state.
            // Only attempted on CreateOnly (Overwrite/CAS have no
            // recoverable prior state at this layer).
            if matches!(mode, PutMode::CreateOnly) && result.is_ok() {
                let _ = self.inner.delete(&id).await;
            }
            return Err(sink_err);
        }

        result
    }

    async fn delete(&self, id: &str) -> Result<(), StoreError> {
        let result = self.inner.delete(id).await;
        self.sink.record(&AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: id.to_string(),
            operation: AuditOperation::Delete,
            result: audit_result(&result),
        })?;
        // Delete is already destructive at the inner layer — there is
        // no recoverable prior state at this layer to restore if the
        // sink were to fail after a successful delete. Fail-closed
        // still applies; the caller is expected to retry and the
        // store surface remains consistent with `NotFound` semantics.
        result
    }

    async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
        let result = self.inner.list(state_kind).await;
        self.sink.record(&AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: "*".to_string(),
            operation: AuditOperation::List,
            result: audit_result(&result),
        })?;
        result
    }

    async fn exists(&self, id: &str) -> Result<bool, StoreError> {
        let result = self.inner.exists(id).await;
        self.sink.record(&AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: id.to_string(),
            operation: AuditOperation::Exists,
            result: audit_result(&result),
        })?;
        result
    }
}

/// Map a store result to an [`AuditResult`] for logging.
///
/// Only error classification is recorded — no credential data leaks.
fn audit_result<T>(result: &Result<T, StoreError>) -> AuditResult {
    match result {
        Ok(_) => AuditResult::Success,
        Err(StoreError::NotFound { .. }) => AuditResult::NotFound,
        Err(StoreError::VersionConflict { .. } | StoreError::AlreadyExists { .. }) => {
            AuditResult::Conflict
        },
        Err(e) => AuditResult::Error(e.to_string()),
    }
}

// Tests gated on `test-util` so storage compiles without features
// (credential's `test_helpers` is itself behind `test-util`).
#[cfg(all(test, feature = "test-util"))]
mod tests {
    use std::sync::Mutex;

    use nebula_credential::PutMode;

    use super::{super::super::memory::InMemoryStore, *};

    struct CollectingSink {
        events: Mutex<Vec<AuditEvent>>,
    }

    impl CollectingSink {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }

        fn events(&self) -> Vec<AuditEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl AuditSink for CollectingSink {
        fn record(&self, event: &AuditEvent) -> Result<(), StoreError> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    fn make_credential(id: &str) -> StoredCredential {
        StoredCredential {
            id: id.into(),
            credential_key: "test_credential".into(),
            data: b"test-data".to_vec(),
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

    fn make_store(sink: &Arc<CollectingSink>) -> AuditLayer<InMemoryStore> {
        AuditLayer::new(InMemoryStore::new(), Arc::clone(sink) as Arc<dyn AuditSink>)
    }

    #[tokio::test]
    async fn get_logs_audit_event() {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink);

        let cred = make_credential("audit-1");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        store.get("audit-1").await.unwrap();

        let events = sink.events();
        let get_event = events
            .iter()
            .find(|e| e.operation == AuditOperation::Get)
            .unwrap();
        assert_eq!(get_event.credential_id, "audit-1");
        assert_eq!(get_event.result, AuditResult::Success);
    }

    #[tokio::test]
    async fn put_logs_audit_event() {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink);

        let cred = make_credential("audit-2");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let events = sink.events();
        let put_event = events
            .iter()
            .find(|e| e.operation == AuditOperation::Put)
            .unwrap();
        assert_eq!(put_event.credential_id, "audit-2");
        assert_eq!(put_event.result, AuditResult::Success);
    }

    #[tokio::test]
    async fn delete_not_found_logs_not_found() {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink);

        let result = store.delete("nonexistent").await;
        assert!(result.is_err());

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, AuditOperation::Delete);
        assert_eq!(events[0].result, AuditResult::NotFound);
    }

    #[tokio::test]
    async fn operations_pass_through_to_inner() {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink);

        // Put a credential
        let cred = make_credential("audit-3");
        let stored = store.put(cred, PutMode::CreateOnly).await.unwrap();
        assert_eq!(stored.id, "audit-3");
        assert_eq!(stored.data, b"test-data");

        // Get returns the same data
        let fetched = store.get("audit-3").await.unwrap();
        assert_eq!(fetched.data, b"test-data");

        // Exists returns true
        assert!(store.exists("audit-3").await.unwrap());

        // List includes the credential
        let ids = store.list(None).await.unwrap();
        assert!(ids.contains(&"audit-3".to_string()));

        // Delete succeeds
        store.delete("audit-3").await.unwrap();
        assert!(!store.exists("audit-3").await.unwrap());
    }

    #[tokio::test]
    async fn list_uses_wildcard_credential_id() {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink);

        store.list(None).await.unwrap();

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].credential_id, "*");
        assert_eq!(events[0].operation, AuditOperation::List);
    }

    #[tokio::test]
    async fn duplicate_put_logs_conflict() {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink);

        let cred = make_credential("audit-dup");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let cred2 = make_credential("audit-dup");
        let result = store.put(cred2, PutMode::CreateOnly).await;
        assert!(result.is_err());

        let events = sink.events();
        let conflict_event = events
            .iter()
            .rfind(|e| e.operation == AuditOperation::Put)
            .unwrap();
        assert_eq!(conflict_event.result, AuditResult::Conflict);
    }
}
