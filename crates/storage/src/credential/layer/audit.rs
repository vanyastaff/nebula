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
//! # Observation boundary
//!
//! The sink is non-authoritative telemetry and does not share a transaction
//! with credential persistence. A sink failure is emitted as bounded telemetry
//! but never changes the store result, triggers a retry, or compensates an
//! already-confirmed mutation. K3 will replace this interim observation with a
//! backend-owned durable audit/outbox protocol.

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use nebula_core::CredentialId;
use nebula_credential::{AuditEvent, AuditOperation, AuditResult, AuditSink};
use nebula_storage_port::{
    CredentialCommit, CredentialCreate, CredentialOwner, CredentialPersistence,
    CredentialPersistenceError, CredentialReplacement, CredentialSelector, CredentialTombstone,
    StoredCredential, StoredCredentialHead,
};

/// Audit logging layer wrapping a [`CredentialPersistence`].
///
/// Delegates every operation to the inner store and records an
/// [`AuditEvent`] via the configured [`AuditSink`]. Sink errors are observable
/// but never override the authoritative persistence result.
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

    fn observe(&self, event: &AuditEvent) {
        if self.sink.record(event).is_err() {
            tracing::warn!(
                target: "nebula_storage::credential_audit",
                "credential audit observation was not accepted"
            );
        }
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
        self.observe(&AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: selector.credential_id().to_string(),
            operation: AuditOperation::Get,
            result: audit_result(&result),
        });
        result
    }

    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        let result = self.inner.get_head(selector).await;
        self.observe(&AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: selector.credential_id().to_string(),
            operation: AuditOperation::Get,
            result: audit_result(&result),
        });
        result
    }

    async fn create(
        &self,
        selector: &CredentialSelector,
        create: CredentialCreate,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let result = self.inner.create(selector, create).await;
        if result.is_ok() {
            self.observe(&AuditEvent {
                timestamp: chrono::Utc::now(),
                credential_id: selector.credential_id().to_string(),
                operation: AuditOperation::Create,
                result: AuditResult::Success,
            });
        }
        result
    }

    async fn replace(
        &self,
        selector: &CredentialSelector,
        replacement: CredentialReplacement,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let result = self.inner.replace(selector, replacement).await;
        if result.is_ok() {
            self.observe(&AuditEvent {
                timestamp: chrono::Utc::now(),
                credential_id: selector.credential_id().to_string(),
                operation: AuditOperation::Replace,
                result: AuditResult::Success,
            });
        }
        result
    }

    async fn tombstone(
        &self,
        selector: &CredentialSelector,
        tombstone: CredentialTombstone,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let result = self.inner.tombstone(selector, tombstone).await;
        if result.is_ok() {
            self.observe(&AuditEvent {
                timestamp: chrono::Utc::now(),
                credential_id: selector.credential_id().to_string(),
                operation: AuditOperation::Tombstone,
                result: AuditResult::Success,
            });
        }
        result
    }

    async fn list(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<CredentialId>, CredentialPersistenceError> {
        let result = self.inner.list(owner, state_kind).await;
        self.observe(&AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: "*".to_string(),
            operation: AuditOperation::List,
            result: audit_result(&result),
        });
        result
    }

    async fn list_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
        let result = self.inner.list_heads(owner, state_kind).await;
        self.observe(&AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: "*".to_owned(),
            operation: AuditOperation::List,
            result: audit_result(&result),
        });
        result
    }

    async fn exists(
        &self,
        selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError> {
        let result = self.inner.exists(selector).await;
        self.observe(&AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: selector.credential_id().to_string(),
            operation: AuditOperation::Exists,
            result: audit_result(&result),
        });
        result
    }
}

/// Map a store result to an [`AuditResult`] for logging.
///
/// Only error classification is recorded — no credential data leaks.
fn audit_result<T>(result: &Result<T, CredentialPersistenceError>) -> AuditResult {
    match result {
        Ok(_) => AuditResult::Success,
        Err(CredentialPersistenceError::NotFound) => AuditResult::NotFound,
        Err(
            CredentialPersistenceError::VersionConflict { .. }
            | CredentialPersistenceError::AlreadyExists { .. },
        ) => AuditResult::Conflict,
        Err(CredentialPersistenceError::VersionExhausted) => {
            AuditResult::Error("version_exhausted".to_owned())
        },
        Err(CredentialPersistenceError::CorruptRecord) => {
            AuditResult::Error("corrupt_record".to_owned())
        },
        Err(CredentialPersistenceError::Unavailable) => {
            AuditResult::Error("unavailable".to_owned())
        },
        Err(CredentialPersistenceError::OutcomeUnknown) => {
            AuditResult::Error("outcome_unknown".to_owned())
        },
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use std::sync::Mutex;

    use nebula_core::CredentialId;
    use nebula_storage_port::{
        CredentialOwner, CredentialSelector, CredentialTombstone, StoredCredential,
        StoredLiveCredential,
    };

    use super::{super::super::sqlite::SqliteCredentialPersistence, *};
    use crate::credential::test_support::make_credential;

    fn owner() -> CredentialOwner {
        CredentialOwner::from_canonical("test-owner")
    }

    fn selector(id: CredentialId) -> CredentialSelector {
        CredentialSelector::new(owner(), id)
    }

    fn into_live(record: StoredCredential) -> StoredLiveCredential {
        let StoredCredential::Live(record) = record else {
            panic!("test fixture must remain live");
        };
        record
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

        let credential_id = CredentialId::new();
        let selector = selector(credential_id);
        store
            .create(&selector, make_credential(b"test-data"))
            .await?;

        store.get(&selector).await?;

        let events = sink.events();
        let get_event = events
            .iter()
            .find(|e| e.operation == AuditOperation::Get)
            .unwrap();
        assert_eq!(get_event.credential_id, credential_id.to_string());
        assert_eq!(get_event.result, AuditResult::Success);
        Ok(())
    }

    #[tokio::test]
    async fn create_logs_audit_event() -> Result<(), CredentialPersistenceError> {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

        let credential_id = CredentialId::new();
        store
            .create(&selector(credential_id), make_credential(b"test-data"))
            .await?;

        let events = sink.events();
        let create_event = events
            .iter()
            .find(|e| e.operation == AuditOperation::Create)
            .unwrap();
        assert_eq!(create_event.credential_id, credential_id.to_string());
        assert_eq!(create_event.result, AuditResult::Success);
        Ok(())
    }

    #[tokio::test]
    async fn tombstone_not_found_emits_no_mutation_event() -> Result<(), CredentialPersistenceError>
    {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

        let result = store
            .tombstone(
                &selector(CredentialId::new()),
                CredentialTombstone::new(
                    nebula_storage_port::CredentialVersion::try_from(1_i64)
                        .expect("test version must be valid"),
                ),
            )
            .await;
        assert_eq!(result, Err(CredentialPersistenceError::NotFound));

        assert!(sink.events().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn operations_pass_through_to_inner() -> Result<(), CredentialPersistenceError> {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

        let credential_id = CredentialId::new();
        let selector = selector(credential_id);
        let created = store
            .create(&selector, make_credential(b"test-data"))
            .await?;
        assert_eq!(created.credential_id(), credential_id);

        // Get returns the same data
        let fetched = into_live(store.get(&selector).await?);
        assert_eq!(fetched.data().as_ref(), b"test-data");

        // Exists returns true
        assert!(store.exists(&selector).await?);

        // List includes the credential
        let ids = store.list(&owner(), None).await?;
        assert!(ids.contains(&credential_id));

        store
            .tombstone(&selector, CredentialTombstone::new(created.version()))
            .await?;
        assert!(!store.exists(&selector).await?);
        assert!(matches!(
            store.get(&selector).await?,
            StoredCredential::Tombstoned(_)
        ));
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
    async fn duplicate_create_emits_only_the_acknowledged_mutation()
    -> Result<(), CredentialPersistenceError> {
        let sink = Arc::new(CollectingSink::new());
        let store = make_store(&sink).await?;

        let credential_id = CredentialId::new();
        let selector = selector(credential_id);
        store.create(&selector, make_credential(b"first")).await?;

        let result = store.create(&selector, make_credential(b"duplicate")).await;
        assert!(result.is_err());

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, AuditOperation::Create);
        assert_eq!(events[0].result, AuditResult::Success);
        Ok(())
    }

    #[tokio::test]
    async fn outcome_unknown_emits_no_mutation_event() -> Result<(), CredentialPersistenceError> {
        let sink = Arc::new(CollectingSink::new());
        let inner = SqliteCredentialPersistence::connect_memory().await?;
        inner.arm_post_commit_outcome_unknown();
        let store = AuditLayer::new(inner.clone(), Arc::clone(&sink) as Arc<dyn AuditSink>);
        let selector = selector(CredentialId::new());

        assert_eq!(
            store
                .create(&selector, make_credential(b"ambiguously-committed"))
                .await,
            Err(CredentialPersistenceError::OutcomeUnknown)
        );
        assert!(sink.events().is_empty());
        assert!(matches!(
            inner.get(&selector).await?,
            StoredCredential::Live(_)
        ));
        Ok(())
    }
}
