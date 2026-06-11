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
//! # Fail-closed invariant (no discard-and-log)
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

use nebula_credential::{
    AuditEvent, AuditOperation, AuditResult, AuditSink, CredentialStore, PutMode, StoreError,
    StoredCredential,
};

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
/// use nebula_storage::credential::{AuditLayer, SqliteCredentialStore};
/// use std::sync::Arc;
///
/// let sink = Arc::new(my_audit_sink);
/// let store = AuditLayer::new(SqliteCredentialStore::connect("sqlite://creds.db").await?, sink);
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
            // Fail-closed: best-effort rollback of the
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

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use std::sync::Mutex;

    use nebula_credential::PutMode;

    use super::{super::super::sqlite::SqliteCredentialStore, *};

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
            name: None,
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

    async fn make_store(
        sink: &Arc<CollectingSink>,
    ) -> Result<AuditLayer<SqliteCredentialStore>, StoreError> {
        Ok(AuditLayer::new(
            SqliteCredentialStore::connect_memory().await?,
            Arc::clone(sink) as Arc<dyn AuditSink>,
        ))
    }

    #[tokio::test]
    async fn get_logs_audit_event() -> Result<(), StoreError> {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

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
        Ok(())
    }

    #[tokio::test]
    async fn put_logs_audit_event() -> Result<(), StoreError> {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

        let cred = make_credential("audit-2");
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let events = sink.events();
        let put_event = events
            .iter()
            .find(|e| e.operation == AuditOperation::Put)
            .unwrap();
        assert_eq!(put_event.credential_id, "audit-2");
        assert_eq!(put_event.result, AuditResult::Success);
        Ok(())
    }

    #[tokio::test]
    async fn delete_not_found_logs_not_found() -> Result<(), StoreError> {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

        let result = store.delete("nonexistent").await;
        assert!(result.is_err());

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, AuditOperation::Delete);
        assert_eq!(events[0].result, AuditResult::NotFound);
        Ok(())
    }

    #[tokio::test]
    async fn operations_pass_through_to_inner() -> Result<(), StoreError> {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

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
        Ok(())
    }

    #[tokio::test]
    async fn list_uses_wildcard_credential_id() -> Result<(), StoreError> {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

        store.list(None).await.unwrap();

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].credential_id, "*");
        assert_eq!(events[0].operation, AuditOperation::List);
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_put_logs_conflict() -> Result<(), StoreError> {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

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
        Ok(())
    }
}
