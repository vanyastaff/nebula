//! CI gate for the non-authoritative credential audit observation boundary.
//!
//! `AuditLayer` never shares a transaction with the wrapped persistence
//! backend. Sink rejection is therefore telemetry only: it must not replace a
//! successful store result, retry a mutation, or compensate committed state.

#![cfg(feature = "sqlite")]

use std::sync::{Arc, Barrier};

use nebula_core::CredentialId;
use nebula_storage::credential::{
    AuditEvent, AuditLayer, AuditOperation, AuditSink, SqliteCredentialPersistence,
};
use nebula_storage_port::{
    CredentialCreate, CredentialOwner, CredentialPersistence, CredentialPersistenceError,
    CredentialReplacement, CredentialSelector, CredentialTombstone, CredentialVersion, SecretBytes,
    StoredCredential,
};

fn owner() -> CredentialOwner {
    CredentialOwner::from_canonical("audit-test-owner")
}

fn selector(id: CredentialId) -> CredentialSelector {
    CredentialSelector::new(owner(), id)
}

fn file_url(path: &std::path::Path) -> String {
    format!("sqlite://{}?mode=rwc", path.display())
}

fn version(value: i64) -> CredentialVersion {
    CredentialVersion::try_from(value).expect("test version must be valid")
}

fn create(data: &[u8]) -> CredentialCreate {
    CredentialCreate::new(
        "test_credential".to_owned(),
        SecretBytes::new(data.to_vec()),
        "test".to_owned(),
        1,
        None,
        None,
        false,
        Default::default(),
    )
}

fn replacement(expected_version: CredentialVersion, data: &[u8]) -> CredentialReplacement {
    CredentialReplacement::new(
        expected_version,
        SecretBytes::new(data.to_vec()),
        "test".to_owned(),
        1,
        None,
        None,
        false,
        Default::default(),
        nebula_storage_port::CredentialMaterialTransition::advance(),
    )
}

#[derive(Debug, Default)]
struct FailingAuditSink;

impl AuditSink for FailingAuditSink {
    fn record(&self, _event: &AuditEvent) -> Result<(), CredentialPersistenceError> {
        Err(CredentialPersistenceError::Unavailable)
    }
}

#[tokio::test]
async fn create_preserves_success_and_committed_mutation_when_sink_rejects() {
    let inner = SqliteCredentialPersistence::connect_memory()
        .await
        .expect("in-memory SQLite store");
    let audited = AuditLayer::new(inner.clone(), Arc::new(FailingAuditSink));
    let selector = selector(CredentialId::new());

    let committed = audited
        .create(&selector, create(b"committed-before-audit"))
        .await
        .expect("audit observation cannot override the authoritative commit");
    assert_eq!(committed.version(), version(1));

    let StoredCredential::Live(stored) = inner
        .get(&selector)
        .await
        .expect("inner mutation commits before audit observation")
    else {
        panic!("create must persist a live record");
    };
    assert_eq!(stored.version(), version(1));
    assert_eq!(stored.data().as_ref(), b"committed-before-audit");

    let StoredCredential::Live(via_layer) = audited
        .get(&selector)
        .await
        .expect("read results also remain authoritative when observation fails")
    else {
        panic!("created record must remain live");
    };
    assert_eq!(via_layer.data().as_ref(), b"committed-before-audit");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delayed_sink_rejection_never_compensates_a_concurrent_replacement() {
    #[derive(Debug)]
    struct BlockingFailingAuditSink {
        entered: Arc<Barrier>,
        release: Arc<Barrier>,
    }

    impl AuditSink for BlockingFailingAuditSink {
        fn record(&self, event: &AuditEvent) -> Result<(), CredentialPersistenceError> {
            assert_eq!(event.operation, AuditOperation::Create);
            self.entered.wait();
            self.release.wait();
            Err(CredentialPersistenceError::Unavailable)
        }
    }

    let directory = tempfile::tempdir().expect("temporary directory must be created");
    let url = file_url(&directory.path().join("audit-observation-race.sqlite"));
    let inner = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("audited SQLite store");
    let concurrent = SqliteCredentialPersistence::connect(&url)
        .await
        .expect("concurrent writer must use an independent pool");
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let audited = AuditLayer::new(
        inner.clone(),
        Arc::new(BlockingFailingAuditSink {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
        }),
    );
    let selector = selector(CredentialId::new());
    let task_selector = selector.clone();

    let create_task =
        tokio::spawn(async move { audited.create(&task_selector, create(b"version-one")).await });

    let entered_wait = Arc::clone(&entered);
    tokio::task::spawn_blocking(move || entered_wait.wait())
        .await
        .expect("audit sink reaches the deterministic pause");

    let StoredCredential::Live(first) = concurrent
        .get(&selector)
        .await
        .expect("create commits before the sink is called")
    else {
        panic!("create fixture must be live");
    };
    assert_eq!(first.version(), version(1));

    let updated = concurrent
        .replace(&selector, replacement(version(1), b"version-two"))
        .await
        .expect("concurrent CAS advances the committed row");
    assert_eq!(updated.version(), version(2));

    let release_wait = Arc::clone(&release);
    tokio::task::spawn_blocking(move || release_wait.wait())
        .await
        .expect("blocked audit sink is released");

    let original = create_task
        .await
        .expect("audited create task completes")
        .expect("sink rejection cannot replace create success");
    assert_eq!(original.version(), version(1));

    let StoredCredential::Live(retained) = concurrent
        .get(&selector)
        .await
        .expect("sink rejection must not compensate a newer write")
    else {
        panic!("concurrent replacement must remain live");
    };
    assert_eq!(retained.version(), version(2));
    assert_eq!(retained.data().as_ref(), b"version-two");
}

#[tokio::test]
async fn read_management_and_tombstone_results_ignore_sink_rejection() {
    let inner = SqliteCredentialPersistence::connect_memory()
        .await
        .expect("in-memory SQLite store");
    let selector = selector(CredentialId::new());
    inner
        .create(&selector, create(b"x"))
        .await
        .expect("fixture create");
    let audited = AuditLayer::new(inner.clone(), Arc::new(FailingAuditSink));

    assert!(matches!(
        audited
            .get(&selector)
            .await
            .expect("get remains authoritative"),
        StoredCredential::Live(_)
    ));
    assert_eq!(
        audited
            .list(&owner(), None)
            .await
            .expect("list remains authoritative"),
        vec![selector.credential_id()]
    );
    assert!(
        audited
            .exists(&selector)
            .await
            .expect("exists remains authoritative")
    );

    let tombstoned = audited
        .tombstone(&selector, CredentialTombstone::new(version(1)))
        .await
        .expect("tombstone remains authoritative");
    assert_eq!(tombstoned.version(), version(2));
    assert!(matches!(
        inner
            .get(&selector)
            .await
            .expect("committed tombstone remains physical"),
        StoredCredential::Tombstoned(_)
    ));
}
