//! Backend conformance matrix (spec-16 §5 / §9).
//!
//! One behavioral suite asserted across every storage backend. Each
//! `#[case]` binds a [`Backend`] implementation; the shared assertions in
//! [`harness`] encode the abstract concurrency + tenancy contract.
//!
//! Skip-clean policy (via `skip_reason`): the Postgres case skips when
//! `DATABASE_URL` is unset; the SQLite case skips when the crate was built
//! without `--features sqlite`. A skipped backend prints a WARN and passes
//! — never a false green claim, never a hard failure on a machine that
//! cannot run that backend.
//!
//! Backends whose adapter does not exist yet make `Backend` return the
//! store via `unimplemented!()`, so the suite compiles and that backend's
//! cases are red. That red is the TDD target for the remaining P2 tasks.

#[path = "conformance/mod.rs"]
mod harness;

use harness::{
    Backend, InMemoryBackend, PostgresBackend, SqliteBackend, assert_atomic_triple,
    assert_cas_conflict, assert_control_queue_outbox_and_fencing, assert_create_get_roundtrip,
    assert_cross_scope_commit_is_rejected, assert_cross_scope_get_is_none,
    assert_idempotency_first_writer_wins, assert_idempotency_store_first_writer_and_scope,
    assert_journal_visibility_and_scope, assert_stale_fencing_is_fenced_out,
    assert_webhook_activation_and_scope, skip_reason,
};
use rstest::rstest;
use std::future::Future;

fn in_memory() -> Box<dyn Backend> {
    Box::new(InMemoryBackend::default())
}

fn sqlite() -> Box<dyn Backend> {
    Box::new(SqliteBackend::default())
}

fn postgres() -> Box<dyn Backend> {
    Box::new(PostgresBackend::default())
}

/// Run `body` against `backend`, skipping cleanly (WARN + pass) when the
/// backend's prerequisites are not met.
async fn run<F, Fut>(backend: Box<dyn Backend>, body: F)
where
    F: FnOnce(Box<dyn Backend>) -> Fut,
    Fut: Future<Output = ()>,
{
    if let Some(reason) = skip_reason(backend.as_ref()) {
        eprintln!("WARN [conformance] {reason}");
        return;
    }
    body(backend).await;
}

macro_rules! matrix {
    ($name:ident, $assertion:path) => {
        #[rstest]
        #[case::in_memory(in_memory())]
        #[case::sqlite(sqlite())]
        #[case::postgres(postgres())]
        #[tokio::test]
        async fn $name(#[case] backend: Box<dyn Backend>) {
            run(backend, |b| async move { $assertion(b.as_ref()).await }).await;
        }
    };
}

matrix!(create_get_roundtrip, assert_create_get_roundtrip);
matrix!(cas_conflict_returns_actual, assert_cas_conflict);
matrix!(
    stale_fencing_is_fenced_out,
    assert_stale_fencing_is_fenced_out
);
matrix!(atomic_triple_all_or_nothing, assert_atomic_triple);
matrix!(
    idempotency_first_writer_wins,
    assert_idempotency_first_writer_wins
);
matrix!(cross_scope_get_is_none, assert_cross_scope_get_is_none);
matrix!(
    cross_scope_commit_is_rejected,
    assert_cross_scope_commit_is_rejected
);
matrix!(
    control_queue_outbox_and_fencing,
    assert_control_queue_outbox_and_fencing
);
matrix!(
    journal_visibility_and_scope,
    assert_journal_visibility_and_scope
);
matrix!(
    idempotency_store_first_writer_and_scope,
    assert_idempotency_store_first_writer_and_scope
);
matrix!(
    webhook_activation_and_scope,
    assert_webhook_activation_and_scope
);
