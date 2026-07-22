//! CI gate for error-propagating credential audit (no silent discard).
//!
//! `AuditLayer` reports sink failures to its caller, but the sink does not
//! share a transaction with the wrapped persistence backend. A successful
//! inner mutation therefore remains committed when audit recording fails.
//! The decorator must never attempt a compensating mutation: another writer
//! may have advanced the record between the original commit and sink failure.
//!
//! This test is the seam that stops future PRs from silently
//! regressing the error-propagating audit invariant back to the old
//! `sink.log(event)` fire-and-forget shape.
//!
//! Ref: ADR-0028, ADR-0032 (historical — the maintainers' private design vault)
//! Ref: the maintainers' private design vault §P6.7

// The layers + the durable SQLite store are feature-gated in storage. Gate on
// `sqlite` so this file is only compiled when that backend is available.
#![cfg(feature = "sqlite")]

use std::sync::{Arc, Barrier};

mod common;

use common::make_credential;
use nebula_storage::credential::{
    AuditEvent, AuditLayer, AuditOperation, AuditSink, SqliteCredentialPersistence,
};
use nebula_storage_port::{
    CredentialOwner, CredentialPersistence, CredentialPersistenceError, CredentialSelector,
    CredentialWriteMode,
};

fn owner() -> CredentialOwner {
    CredentialOwner::from_canonical("audit-test-owner")
}

fn selector(id: &str) -> CredentialSelector {
    CredentialSelector::new(owner(), id)
}

/// Sink that always refuses to record, proving that `AuditLayer` surfaces
/// rather than swallows audit failures.
#[derive(Debug, Default)]
struct FailingAuditSink;

impl AuditSink for FailingAuditSink {
    fn record(&self, _event: &AuditEvent) -> Result<(), CredentialPersistenceError> {
        Err(CredentialPersistenceError::AuditFailure(
            "synthetic audit sink failure".into(),
        ))
    }
}

/// Primary gate: `put` with a failing audit sink reports the audit error while
/// preserving the mutation already committed by the inner store.
#[tokio::test]
async fn put_surfaces_audit_failure_after_committed_mutation() {
    let inner = SqliteCredentialPersistence::connect_memory()
        .await
        .expect("in-memory SQLite store");
    // Share the inner store so the test can inspect it directly after
    // the failed put. `SqliteCredentialPersistence` is cheap-cloneable (the pool is
    // `Arc`-backed), so the clone observes the same in-memory database.
    let audited = AuditLayer::new(inner.clone(), Arc::new(FailingAuditSink));

    let credential_id = "cred_audit_failure_put";
    let record = make_credential(credential_id, b"committed-before-audit");

    let result = audited
        .put(
            &selector(&record.id),
            record,
            CredentialWriteMode::CreateOnly,
        )
        .await;

    assert!(
        matches!(result, Err(CredentialPersistenceError::AuditFailure(_))),
        "audit failure must surface as CredentialPersistenceError::AuditFailure, got {result:?}"
    );

    let committed = inner
        .get(&selector(credential_id))
        .await
        .expect("inner mutation commits before audit recording");
    assert_eq!(committed.version, 1);
    assert_eq!(committed.data, b"committed-before-audit");

    let lookup_via_layer = audited.get(&selector(credential_id)).await;
    // Reads still propagate sink failure even though the committed row is
    // observable through the backend.
    assert!(
        matches!(
            lookup_via_layer,
            Err(CredentialPersistenceError::AuditFailure(_))
        ),
        "get via AuditLayer must propagate the sink failure, got {lookup_via_layer:?}"
    );
}

/// Regression for the unsafe compensation race: the audit sink pauses after
/// the CreateOnly insert, a concurrent writer advances the row with CAS, and
/// the later sink failure must not delete that newer version.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_create_audit_never_deletes_a_concurrent_update() {
    #[derive(Debug)]
    struct BlockingFailingAuditSink {
        entered: Arc<Barrier>,
        release: Arc<Barrier>,
    }

    impl AuditSink for BlockingFailingAuditSink {
        fn record(&self, event: &AuditEvent) -> Result<(), CredentialPersistenceError> {
            assert_eq!(event.operation, AuditOperation::Put);
            self.entered.wait();
            self.release.wait();
            Err(CredentialPersistenceError::AuditFailure(
                "synthetic delayed audit sink failure".into(),
            ))
        }
    }

    let inner = SqliteCredentialPersistence::connect_memory()
        .await
        .expect("in-memory SQLite store");
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let audited = AuditLayer::new(
        inner.clone(),
        Arc::new(BlockingFailingAuditSink {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
        }),
    );
    let selector = selector("cred_audit_concurrent_update");
    let task_selector = selector.clone();

    let put_task = tokio::spawn(async move {
        audited
            .put(
                &task_selector,
                make_credential(task_selector.credential_id(), b"version-one"),
                CredentialWriteMode::CreateOnly,
            )
            .await
    });

    let entered_wait = Arc::clone(&entered);
    tokio::task::spawn_blocking(move || {
        entered_wait.wait();
    })
    .await
    .expect("audit sink reaches the deterministic pause");

    let committed = inner
        .get(&selector)
        .await
        .expect("CreateOnly insert commits before the sink is called");
    assert_eq!(committed.version, 1);
    assert_eq!(committed.data, b"version-one");

    let mut concurrent_update = committed;
    concurrent_update.data = b"version-two".to_vec().into();
    let updated = inner
        .put(
            &selector,
            concurrent_update,
            CredentialWriteMode::CompareAndSwap {
                expected_version: 1,
            },
        )
        .await
        .expect("concurrent CAS advances the committed row");
    assert_eq!(updated.version, 2);
    assert_eq!(updated.data, b"version-two");

    let release_wait = Arc::clone(&release);
    tokio::task::spawn_blocking(move || {
        release_wait.wait();
    })
    .await
    .expect("blocked audit sink is released");

    let result = put_task.await.expect("audited put task completes");
    assert!(
        matches!(result, Err(CredentialPersistenceError::AuditFailure(_))),
        "delayed audit failure must surface to the original caller, got {result:?}"
    );

    let retained = inner
        .get(&selector)
        .await
        .expect("audit failure must not compensate away a newer write");
    assert_eq!(retained.version, 2);
    assert_eq!(retained.data, b"version-two");
}

/// Read-path gate: a failing sink must fail `get` too (no silent log).
///
/// Exercises the `?` after `sink.record` in the `get` impl.
#[tokio::test]
async fn get_propagates_audit_failure() {
    let inner = SqliteCredentialPersistence::connect_memory()
        .await
        .expect("in-memory SQLite store");
    // Pre-populate via the raw inner (no AuditLayer) so the test
    // isolates the read-path invariant.
    let record = make_credential("cred_audit_read", b"x");
    inner
        .put(
            &selector(&record.id),
            record,
            CredentialWriteMode::CreateOnly,
        )
        .await
        .unwrap();

    let audited = AuditLayer::new(inner, Arc::new(FailingAuditSink));

    let result = audited.get(&selector("cred_audit_read")).await;

    assert!(
        matches!(result, Err(CredentialPersistenceError::AuditFailure(_))),
        "get must surface audit failures, got {result:?}"
    );
}

/// `delete` under a failing sink must also propagate the audit error. The inner
/// delete has already happened (destructive at that layer), but the
/// operation must still surface the audit failure so the caller cannot mistake
/// the mutation for an audit-confirmed success.
#[tokio::test]
async fn delete_propagates_audit_failure_after_committed_delete() {
    let inner = SqliteCredentialPersistence::connect_memory()
        .await
        .expect("in-memory SQLite store");
    let record = make_credential("cred_audit_delete", b"x");
    inner
        .put(
            &selector(&record.id),
            record,
            CredentialWriteMode::CreateOnly,
        )
        .await
        .unwrap();

    let audited = AuditLayer::new(inner.clone(), Arc::new(FailingAuditSink));

    let result = audited.delete(&selector("cred_audit_delete")).await;

    assert!(
        matches!(result, Err(CredentialPersistenceError::AuditFailure(_))),
        "delete must surface audit failures, got {result:?}"
    );
    let lookup = inner.get(&selector("cred_audit_delete")).await;
    assert!(
        matches!(lookup, Err(CredentialPersistenceError::NotFound { .. })),
        "inner delete remains committed after audit failure, got {lookup:?}"
    );
}

/// `list` under a failing sink must propagate the audit error too. Proves the
/// wildcard-event path in `AuditLayer::list` hits the `?` propagation
/// identically to id-scoped operations.
#[tokio::test]
async fn list_propagates_audit_failure() {
    let audited = AuditLayer::new(
        SqliteCredentialPersistence::connect_memory()
            .await
            .expect("in-memory SQLite store"),
        Arc::new(FailingAuditSink),
    );

    let result = audited.list(&owner(), None).await;

    assert!(
        matches!(result, Err(CredentialPersistenceError::AuditFailure(_))),
        "list must surface audit failures, got {result:?}"
    );
}

/// `exists` under a failing sink must propagate the audit error. Closes the last
/// mutating/non-mutating surface on `CredentialPersistence`.
#[tokio::test]
async fn exists_propagates_audit_failure() {
    let audited = AuditLayer::new(
        SqliteCredentialPersistence::connect_memory()
            .await
            .expect("in-memory SQLite store"),
        Arc::new(FailingAuditSink),
    );

    let result = audited.exists(&selector("anything")).await;

    assert!(
        matches!(result, Err(CredentialPersistenceError::AuditFailure(_))),
        "exists must surface audit failures, got {result:?}"
    );
}
