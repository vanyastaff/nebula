//! Audit logging layer for credential operations.
//!
//! Logs access patterns (who accessed what, when, result) without
//! ever seeing plaintext credential data. Sits above EncryptionLayer
//! in the layer stack.
//!
//! # Design
//!
//! `AuditLayer` wraps any [`CredentialPersistence`] and delegates every operation
//! to the inner store, emitting an [`AuditEvent`] to the pluggable
//! [`AuditSink`] for each call. Only metadata flows through the sink —
//! credential data never does.
//!
//! # Error propagation and mutation boundary
//!
//! If [`AuditSink::record`] returns an error, `AuditLayer` returns that error
//! instead of silently discarding it. The sink and wrapped store do not share
//! a transaction: a successful `put` or `delete` remains committed when audit
//! recording subsequently fails. The layer deliberately performs no
//! compensating mutation because another writer may have advanced the record
//! after the original commit.
//!
//! Atomic mutation-plus-audit persistence requires a backend transaction or
//! transactional outbox above this decorator. The
//! `credential_audit_failure` integration test gates the interim
//! no-silent-discard/no-compensation contract.

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use nebula_credential::{AuditEvent, AuditOperation, AuditResult, AuditSink};
use nebula_storage_port::{
    CredentialOwner, CredentialPersistence, CredentialPersistenceError, CredentialSelector,
    CredentialWriteMode, StoredCredential, StoredCredentialHead,
};

/// Audit logging layer wrapping a [`CredentialPersistence`].
///
/// Delegates every operation to the inner store and records an
/// [`AuditEvent`] via the configured [`AuditSink`]. Sink errors are returned
/// to the caller without attempting to undo an already-committed mutation
/// (see the module-level mutation boundary).
///
/// # Examples
///
/// Requires the `sqlite` feature; the async `connect` makes this `no_run`
/// (it still type-checks the real API):
///
/// ```rust,no_run
/// # #[cfg(feature = "sqlite")]
/// # async fn doc() -> Result<(), Box<dyn std::error::Error>> {
/// use std::sync::Arc;
///
/// use nebula_credential::{AuditEvent, AuditSink};
/// use nebula_storage::credential::{AuditLayer, SqliteCredentialPersistence};
/// use nebula_storage_port::CredentialPersistenceError;
///
/// // A real sink ships each event to durable audit storage; here it is a stub.
/// struct StdoutSink;
/// impl AuditSink for StdoutSink {
///     fn record(&self, event: &AuditEvent) -> Result<(), CredentialPersistenceError> {
///         println!("{} {:?}", event.credential_id, event.operation);
///         Ok(())
///     }
/// }
///
/// let sink: Arc<dyn AuditSink> = Arc::new(StdoutSink);
/// let backend = SqliteCredentialPersistence::connect("sqlite://creds.db").await?;
/// let store = AuditLayer::new(backend, sink);
/// # let _ = store;
/// # Ok(())
/// # }
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

impl<S> fmt::Debug for AuditLayer<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("AuditLayer").finish_non_exhaustive()
    }
}

#[async_trait]
impl<S: CredentialPersistence> CredentialPersistence for AuditLayer<S> {
    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        let result = self.inner.get(selector).await;
        self.sink.record(&AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: selector.credential_id().to_owned(),
            operation: AuditOperation::Get,
            result: audit_result(&result),
        })?;
        result
    }

    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        let result = self.inner.get_head(selector).await;
        self.sink.record(&AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: selector.credential_id().to_owned(),
            operation: AuditOperation::Get,
            result: audit_result(&result),
        })?;
        result
    }

    async fn put(
        &self,
        selector: &CredentialSelector,
        credential: StoredCredential,
        mode: CredentialWriteMode,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        let id = selector.credential_id().to_owned();
        let result = self.inner.put(selector, credential, mode).await;

        let event = AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: id,
            operation: AuditOperation::Put,
            result: audit_result(&result),
        };
        self.sink.record(&event)?;
        result
    }

    async fn delete(
        &self,
        selector: &CredentialSelector,
    ) -> Result<(), CredentialPersistenceError> {
        let result = self.inner.delete(selector).await;
        self.sink.record(&AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: selector.credential_id().to_owned(),
            operation: AuditOperation::Delete,
            result: audit_result(&result),
        })?;
        // The delete may already be committed when sink recording fails.
        // This decorator reports the failure but never issues a compensating
        // write; see the module-level mutation boundary.
        result
    }

    async fn list(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<String>, CredentialPersistenceError> {
        let result = self.inner.list(owner, state_kind).await;
        self.sink.record(&AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: "*".to_string(),
            operation: AuditOperation::List,
            result: audit_result(&result),
        })?;
        result
    }

    async fn list_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
        let result = self.inner.list_heads(owner, state_kind).await;
        self.sink.record(&AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: "*".to_owned(),
            operation: AuditOperation::List,
            result: audit_result(&result),
        })?;
        result
    }

    async fn exists(
        &self,
        selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError> {
        let result = self.inner.exists(selector).await;
        self.sink.record(&AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: selector.credential_id().to_owned(),
            operation: AuditOperation::Exists,
            result: audit_result(&result),
        })?;
        result
    }
}

/// Map a store result to an [`AuditResult`] for logging.
///
/// Only error classification is recorded — no credential data leaks.
fn audit_result<T>(result: &Result<T, CredentialPersistenceError>) -> AuditResult {
    match result {
        Ok(_) => AuditResult::Success,
        Err(CredentialPersistenceError::NotFound { .. }) => AuditResult::NotFound,
        Err(
            CredentialPersistenceError::VersionConflict { .. }
            | CredentialPersistenceError::AlreadyExists { .. },
        ) => AuditResult::Conflict,
        Err(e) => AuditResult::Error(e.to_string()),
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use std::sync::Mutex;

    use nebula_storage_port::{CredentialOwner, CredentialSelector};

    use super::{super::super::sqlite::SqliteCredentialPersistence, *};

    fn owner() -> CredentialOwner {
        CredentialOwner::from_canonical("test-owner")
    }

    fn selector(id: &str) -> CredentialSelector {
        CredentialSelector::new(owner(), id)
    }

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
        fn record(&self, event: &AuditEvent) -> Result<(), CredentialPersistenceError> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    fn make_credential(id: &str) -> StoredCredential {
        StoredCredential {
            id: id.into(),
            name: None,
            credential_key: "test_credential".into(),
            data: b"test-data".to_vec().into(),
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
    ) -> Result<AuditLayer<SqliteCredentialPersistence>, CredentialPersistenceError> {
        Ok(AuditLayer::new(
            SqliteCredentialPersistence::connect_memory().await?,
            Arc::clone(sink) as Arc<dyn AuditSink>,
        ))
    }

    #[tokio::test]
    async fn get_logs_audit_event() -> Result<(), CredentialPersistenceError> {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

        let cred = make_credential("audit-1");
        store
            .put(&selector(&cred.id), cred, CredentialWriteMode::CreateOnly)
            .await
            .unwrap();

        store.get(&selector("audit-1")).await.unwrap();

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
    async fn put_logs_audit_event() -> Result<(), CredentialPersistenceError> {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

        let cred = make_credential("audit-2");
        store
            .put(&selector(&cred.id), cred, CredentialWriteMode::CreateOnly)
            .await
            .unwrap();

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
    async fn delete_not_found_logs_not_found() -> Result<(), CredentialPersistenceError> {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

        let result = store.delete(&selector("nonexistent")).await;
        assert!(result.is_err());

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, AuditOperation::Delete);
        assert_eq!(events[0].result, AuditResult::NotFound);
        Ok(())
    }

    #[tokio::test]
    async fn operations_pass_through_to_inner() -> Result<(), CredentialPersistenceError> {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

        // Put a credential
        let cred = make_credential("audit-3");
        let stored = store
            .put(&selector(&cred.id), cred, CredentialWriteMode::CreateOnly)
            .await
            .unwrap();
        assert_eq!(stored.id, "audit-3");
        assert_eq!(stored.data, b"test-data");

        // Get returns the same data
        let fetched = store.get(&selector("audit-3")).await.unwrap();
        assert_eq!(fetched.data, b"test-data");

        // Exists returns true
        assert!(store.exists(&selector("audit-3")).await.unwrap());

        // List includes the credential
        let ids = store.list(&owner(), None).await.unwrap();
        assert!(ids.contains(&"audit-3".to_string()));

        // Delete succeeds
        store.delete(&selector("audit-3")).await.unwrap();
        assert!(!store.exists(&selector("audit-3")).await.unwrap());
        Ok(())
    }

    #[tokio::test]
    async fn list_uses_wildcard_credential_id() -> Result<(), CredentialPersistenceError> {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

        store.list(&owner(), None).await.unwrap();

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].credential_id, "*");
        assert_eq!(events[0].operation, AuditOperation::List);
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_put_logs_conflict() -> Result<(), CredentialPersistenceError> {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

        let cred = make_credential("audit-dup");
        store
            .put(&selector(&cred.id), cred, CredentialWriteMode::CreateOnly)
            .await
            .unwrap();

        let cred2 = make_credential("audit-dup");
        let result = store
            .put(&selector(&cred2.id), cred2, CredentialWriteMode::CreateOnly)
            .await;
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
