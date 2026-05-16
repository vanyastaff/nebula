//! Backend conformance matrix (spec-16 §5 / §9).
//!
//! One behavioral suite asserted across every storage backend. Each
//! `#[case]` binds a [`Backend`] implementation; the shared assertions in
//! [`harness`] encode the abstract concurrency + tenancy contract. The
//! Postgres case is `#[ignore]`d unless `DATABASE_URL` is set, so the suite
//! is green on a machine without a database and exercises Postgres in CI /
//! locally when the URL is provided.
//!
//! Adapters that do not exist yet make their `Backend` return the store via
//! `unimplemented!()`, so this suite compiles and is *red* until the adapter
//! lands. That red is the TDD target for P2 Tasks 9–14.

#[path = "conformance/mod.rs"]
mod harness;

use harness::{
    Backend, InMemoryBackend, PostgresBackend, SqliteBackend, assert_atomic_triple,
    assert_cas_conflict, assert_create_get_roundtrip, assert_cross_scope_commit_is_rejected,
    assert_cross_scope_get_is_none, assert_idempotency_first_writer_wins,
    assert_stale_fencing_is_fenced_out, postgres_available,
};
use rstest::rstest;

fn in_memory() -> Box<dyn Backend> {
    Box::new(InMemoryBackend)
}

fn sqlite() -> Box<dyn Backend> {
    Box::new(SqliteBackend)
}

fn postgres() -> Box<dyn Backend> {
    Box::new(PostgresBackend)
}

#[rstest]
#[case::in_memory(in_memory())]
#[case::sqlite(sqlite())]
#[case::postgres(postgres())]
#[tokio::test]
async fn create_get_roundtrip(#[case] backend: Box<dyn Backend>) {
    if backend.name() == "Postgres" && !postgres_available() {
        eprintln!("WARN [conformance] DATABASE_URL unset; skipping Postgres case");
        return;
    }
    assert_create_get_roundtrip(backend.as_ref()).await;
}

#[rstest]
#[case::in_memory(in_memory())]
#[case::sqlite(sqlite())]
#[case::postgres(postgres())]
#[tokio::test]
async fn cas_conflict_returns_actual(#[case] backend: Box<dyn Backend>) {
    if backend.name() == "Postgres" && !postgres_available() {
        eprintln!("WARN [conformance] DATABASE_URL unset; skipping Postgres case");
        return;
    }
    assert_cas_conflict(backend.as_ref()).await;
}

#[rstest]
#[case::in_memory(in_memory())]
#[case::sqlite(sqlite())]
#[case::postgres(postgres())]
#[tokio::test]
async fn stale_fencing_is_fenced_out(#[case] backend: Box<dyn Backend>) {
    if backend.name() == "Postgres" && !postgres_available() {
        eprintln!("WARN [conformance] DATABASE_URL unset; skipping Postgres case");
        return;
    }
    assert_stale_fencing_is_fenced_out(backend.as_ref()).await;
}

#[rstest]
#[case::in_memory(in_memory())]
#[case::sqlite(sqlite())]
#[case::postgres(postgres())]
#[tokio::test]
async fn atomic_triple_all_or_nothing(#[case] backend: Box<dyn Backend>) {
    if backend.name() == "Postgres" && !postgres_available() {
        eprintln!("WARN [conformance] DATABASE_URL unset; skipping Postgres case");
        return;
    }
    assert_atomic_triple(backend.as_ref()).await;
}

#[rstest]
#[case::in_memory(in_memory())]
#[case::sqlite(sqlite())]
#[case::postgres(postgres())]
#[tokio::test]
async fn idempotency_first_writer_wins(#[case] backend: Box<dyn Backend>) {
    if backend.name() == "Postgres" && !postgres_available() {
        eprintln!("WARN [conformance] DATABASE_URL unset; skipping Postgres case");
        return;
    }
    assert_idempotency_first_writer_wins(backend.as_ref()).await;
}

#[rstest]
#[case::in_memory(in_memory())]
#[case::sqlite(sqlite())]
#[case::postgres(postgres())]
#[tokio::test]
async fn cross_scope_get_is_none(#[case] backend: Box<dyn Backend>) {
    if backend.name() == "Postgres" && !postgres_available() {
        eprintln!("WARN [conformance] DATABASE_URL unset; skipping Postgres case");
        return;
    }
    assert_cross_scope_get_is_none(backend.as_ref()).await;
}

#[rstest]
#[case::in_memory(in_memory())]
#[case::sqlite(sqlite())]
#[case::postgres(postgres())]
#[tokio::test]
async fn cross_scope_commit_is_rejected(#[case] backend: Box<dyn Backend>) {
    if backend.name() == "Postgres" && !postgres_available() {
        eprintln!("WARN [conformance] DATABASE_URL unset; skipping Postgres case");
        return;
    }
    assert_cross_scope_commit_is_rejected(backend.as_ref()).await;
}
