//! CI gate for fail-closed credential audit (no discard-and-log):
//! audit is in-line durable. If the audit sink errors, the credential
//! operation errors. No "log and continue" path.
//!
//! This test is the seam that stops future PRs from silently
//! regressing the fail-closed audit invariant back to the old
//! `sink.log(event)` fire-and-forget shape.
//!
//! Ref: ADR-0028, ADR-0032 (historical — the maintainers' private design vault)
//! Ref: the maintainers' private design vault §P6.7

// The layers + the durable SQLite store are feature-gated in storage. Gate on
// `sqlite` so this file is only compiled when that backend is available.
#![cfg(feature = "sqlite")]

use std::sync::Arc;

mod common;

use common::make_credential;
use nebula_credential::{CredentialStore, PutMode, StoreError};
use nebula_storage::credential::{AuditEvent, AuditLayer, AuditSink, SqliteCredentialStore};

/// Sink that always refuses to record, to prove `AuditLayer` surfaces
/// (does not swallow) audit failures.
#[derive(Debug, Default)]
struct FailingAuditSink;

impl AuditSink for FailingAuditSink {
    fn record(&self, _event: &AuditEvent) -> Result<(), StoreError> {
        Err(StoreError::AuditFailure(
            "synthetic audit sink failure".into(),
        ))
    }
}

/// Primary gate: `put` with a failing audit sink must return
/// `StoreError::AuditFailure`, AND the inner store must remain
/// unchanged (fail-closed — the write is rolled back).
#[tokio::test]
async fn put_returns_audit_failure_and_rolls_back_inner() {
    let inner = SqliteCredentialStore::connect_memory()
        .await
        .expect("in-memory SQLite store");
    // Share the inner store so the test can inspect it directly after
    // the failed put. `SqliteCredentialStore` is cheap-cloneable (the pool is
    // `Arc`-backed), so the clone observes the same in-memory database.
    let audited = AuditLayer::new(inner.clone(), Arc::new(FailingAuditSink));

    let credential_id = "cred_audit_durable_put";
    let record = make_credential(credential_id, b"audit-rollback-payload");

    let result = audited.put(record, PutMode::CreateOnly).await;

    assert!(
        matches!(result, Err(StoreError::AuditFailure(_))),
        "audit failure must surface as StoreError::AuditFailure, got {result:?}"
    );

    // Defense-in-depth: the inner store must NOT have the record.
    // Fail-closed — the audit failure aborts the whole operation and
    // the `AuditLayer` rolls back the best-effort CreateOnly write.
    let lookup = inner.get(credential_id).await;
    assert!(
        matches!(lookup, Err(StoreError::NotFound { .. })),
        "inner store must be unchanged on audit failure, got {lookup:?}"
    );

    // The audited store must agree (no residual state observable
    // through either surface).
    let lookup_via_layer = audited.get(credential_id).await;
    // `get` also goes through the failing sink, so this must ALSO
    // fail with AuditFailure — proving the read path is fail-closed
    // too.
    assert!(
        matches!(lookup_via_layer, Err(StoreError::AuditFailure(_))),
        "get via AuditLayer with failing sink must fail-closed, got {lookup_via_layer:?}"
    );
}

/// Read-path gate: a failing sink must fail `get` too (no silent log).
///
/// Exercises the `?` after `sink.record` in the `get` impl.
#[tokio::test]
async fn get_is_fail_closed_under_audit_failure() {
    let inner = SqliteCredentialStore::connect_memory()
        .await
        .expect("in-memory SQLite store");
    // Pre-populate via the raw inner (no AuditLayer) so the test
    // isolates the read-path invariant.
    inner
        .put(
            make_credential("cred_audit_read", b"x"),
            PutMode::CreateOnly,
        )
        .await
        .unwrap();

    let audited = AuditLayer::new(inner, Arc::new(FailingAuditSink));

    let result = audited.get("cred_audit_read").await;

    assert!(
        matches!(result, Err(StoreError::AuditFailure(_))),
        "get must surface audit failures, got {result:?}"
    );
}

/// `delete` under a failing sink must also fail-closed. The inner
/// delete has already happened (destructive at that layer), but the
/// operation must still surface the audit failure so the caller can
/// retry-and-observe rather than silently succeed.
#[tokio::test]
async fn delete_is_fail_closed_under_audit_failure() {
    let inner = SqliteCredentialStore::connect_memory()
        .await
        .expect("in-memory SQLite store");
    inner
        .put(
            make_credential("cred_audit_delete", b"x"),
            PutMode::CreateOnly,
        )
        .await
        .unwrap();

    let audited = AuditLayer::new(inner, Arc::new(FailingAuditSink));

    let result = audited.delete("cred_audit_delete").await;

    assert!(
        matches!(result, Err(StoreError::AuditFailure(_))),
        "delete must surface audit failures, got {result:?}"
    );
}

/// `list` under a failing sink must fail-closed too. Proves the
/// wildcard-event path in `AuditLayer::list` hits the `?` propagation
/// identically to id-scoped operations.
#[tokio::test]
async fn list_is_fail_closed_under_audit_failure() {
    let audited = AuditLayer::new(
        SqliteCredentialStore::connect_memory()
            .await
            .expect("in-memory SQLite store"),
        Arc::new(FailingAuditSink),
    );

    let result = audited.list(None).await;

    assert!(
        matches!(result, Err(StoreError::AuditFailure(_))),
        "list must surface audit failures, got {result:?}"
    );
}

/// `exists` under a failing sink must fail-closed. Closes the last
/// mutating/non-mutating surface on `CredentialStore`.
#[tokio::test]
async fn exists_is_fail_closed_under_audit_failure() {
    let audited = AuditLayer::new(
        SqliteCredentialStore::connect_memory()
            .await
            .expect("in-memory SQLite store"),
        Arc::new(FailingAuditSink),
    );

    let result = audited.exists("anything").await;

    assert!(
        matches!(result, Err(StoreError::AuditFailure(_))),
        "exists must surface audit failures, got {result:?}"
    );
}
