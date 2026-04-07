//! Audit logging layer for credential operations.
//!
//! Logs access patterns (who accessed what, when, result) without
//! ever seeing plaintext credential data. Sits above EncryptionLayer
//! in the layer stack.
//!
//! # Design
//!
//! `AuditLayer` wraps any CredentialStore and delegates every operation
//! unchanged, emitting an [`AuditEvent`] to the pluggable [`AuditSink`]
//! after each call completes. Only metadata is logged — credential data
//! never passes through the sink.

use std::sync::Arc;

use crate::store::{CredentialStore, PutMode, StoreError, StoredCredential};

/// Receives audit events for logging or persistence.
///
/// Implementations might write to a file, send to an event bus, or
/// collect events in memory for testing.
///
/// # Contract
///
/// - `log` must not block the calling task for extended periods.
/// - Implementations must never inspect or log credential data.
pub trait AuditSink: Send + Sync {
    /// Called after each credential store operation completes.
    fn log(&self, event: AuditEvent);
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
/// Delegates every operation to the inner store unchanged and logs
/// an [`AuditEvent`] to the configured [`AuditSink`] after each call.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_credential::{AuditLayer, InMemoryStore};
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
        self.sink.log(AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: id.to_string(),
            operation: AuditOperation::Get,
            result: audit_result(&result),
        });
        result
    }

    async fn put(
        &self,
        credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, StoreError> {
        let id = credential.id.clone();
        let result = self.inner.put(credential, mode).await;
        self.sink.log(AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: id,
            operation: AuditOperation::Put,
            result: audit_result(&result),
        });
        result
    }

    async fn delete(&self, id: &str) -> Result<(), StoreError> {
        let result = self.inner.delete(id).await;
        self.sink.log(AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: id.to_string(),
            operation: AuditOperation::Delete,
            result: audit_result(&result),
        });
        result
    }

    async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
        let result = self.inner.list(state_kind).await;
        self.sink.log(AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: "*".to_string(),
            operation: AuditOperation::List,
            result: audit_result(&result),
        });
        result
    }

    async fn exists(&self, id: &str) -> Result<bool, StoreError> {
        let result = self.inner.exists(id).await;
        self.sink.log(AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: id.to_string(),
            operation: AuditOperation::Exists,
            result: audit_result(&result),
        });
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
        }
        Err(e) => AuditResult::Error(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::store::PutMode;
    use crate::store_memory::InMemoryStore;

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
        fn log(&self, event: AuditEvent) {
            self.events.lock().unwrap().push(event);
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
            .filter(|e| e.operation == AuditOperation::Put)
            .last()
            .unwrap();
        assert_eq!(conflict_event.result, AuditResult::Conflict);
    }
}
