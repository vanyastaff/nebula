//! Backend conformance matrix (spec-16 §5 / §9).
//!
//! One behavioral suite asserted across every storage backend. Each
//! `#[case]` binds a [`Backend`] implementation; the shared assertions in
//! [`harness`] encode the abstract concurrency + tenancy contract.
//!
//! Skip-clean policy (via `skip_reason`): the Postgres case skips when
//! `DATABASE_URL` is unset or the `postgres` feature is disabled, unless
//! `NEBULA_REQUIRE_POSTGRES` is set, which makes either a hard failure.
//! Invalid configured URLs fail; their values are never printed by the gate.
//! SQLite skips when built without `--features sqlite`. An optional skipped
//! backend prints a WARN and passes; it does not count as runtime verification.
//!
//! Backends whose adapter does not exist yet make `Backend` return the
//! store via `unimplemented!()`, so the suite compiles and that backend's
//! cases are red. That red is the TDD target for the remaining P2 tasks.

#![expect(
    clippy::print_stderr,
    reason = "conformance harness reports skip/diagnostic lines to stderr"
)]

#[path = "conformance/mod.rs"]
mod harness;

use harness::{
    Backend, InMemoryBackend, PostgresBackend, ScopedBackend, SqliteBackend, assert_atomic_triple,
    assert_cas_conflict, assert_control_queue_outbox_and_fencing,
    assert_control_queue_release_returns_row_for_redelivery,
    assert_control_queue_same_processor_aba_is_fenced, assert_create_get_roundtrip,
    assert_cross_scope_commit_is_rejected, assert_cross_scope_get_is_none,
    assert_expired_rollbacks_are_released, assert_get_published_is_highest_numbered,
    assert_idempotency_first_writer_wins, assert_idempotency_store_cross_scope_isolated,
    assert_idempotency_store_first_writer, assert_job_dispatch_exact_flavor,
    assert_job_dispatch_fencing, assert_job_dispatch_requires_primary_plugin,
    assert_job_dispatch_routes_by_plugin, assert_job_dispatch_routes_by_plugin_superset,
    assert_job_dispatch_same_processor_aba_is_fenced, assert_journal_visibility_and_scope,
    assert_live_lease_blocks_acquire, assert_non_resume_row_still_exhausts,
    assert_resume_row_exempt_from_reclaim_budget, assert_resume_target_survives_queue_round_trip,
    assert_save_with_published_version_is_atomic, assert_stale_fencing_is_fenced_out,
    assert_terminal_commit_rejects_incompatible_reference_transition,
    assert_terminal_commit_releases_live_reference, assert_terminal_commit_retains_rollback_window,
    assert_webhook_activation_and_scope, assert_webhook_system_surface,
    assert_workflow_store_contract, skip_reason,
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
            run(backend, |b| async move {
                let _observation = $assertion(b.as_ref()).await;
            })
            .await;
        }
    };
}

matrix!(create_get_roundtrip, assert_create_get_roundtrip);
matrix!(cas_conflict_returns_actual, assert_cas_conflict);
matrix!(
    stale_fencing_is_fenced_out,
    assert_stale_fencing_is_fenced_out
);
matrix!(live_lease_blocks_acquire, assert_live_lease_blocks_acquire);
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
    resume_target_survives_queue_round_trip,
    assert_resume_target_survives_queue_round_trip
);
matrix!(
    resume_row_exempt_from_reclaim_budget,
    assert_resume_row_exempt_from_reclaim_budget
);
matrix!(
    non_resume_row_still_exhausts,
    assert_non_resume_row_still_exhausts
);
matrix!(
    journal_visibility_and_scope,
    assert_journal_visibility_and_scope
);
matrix!(
    idempotency_store_first_writer,
    assert_idempotency_store_first_writer
);
matrix!(
    idempotency_store_cross_scope_isolated,
    assert_idempotency_store_cross_scope_isolated
);
matrix!(
    webhook_activation_and_scope,
    assert_webhook_activation_and_scope
);
matrix!(webhook_system_surface, assert_webhook_system_surface);
matrix!(workflow_store_contract, assert_workflow_store_contract);
matrix!(
    save_with_published_version_is_atomic,
    assert_save_with_published_version_is_atomic
);
matrix!(
    get_published_is_highest_numbered,
    assert_get_published_is_highest_numbered
);
matrix!(
    job_dispatch_routes_by_plugin,
    assert_job_dispatch_routes_by_plugin
);
matrix!(
    job_dispatch_requires_primary_plugin,
    assert_job_dispatch_requires_primary_plugin
);

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_job_cleanup_uses_terminal_transition() {
    let backend = SqliteBackend::default();
    harness::assert_sql_job_cleanup_uses_terminal_transition(&backend).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_job_cleanup_uses_terminal_transition() {
    let backend = PostgresBackend::default();
    if let Some(reason) = skip_reason(&backend) {
        eprintln!("WARN [conformance] {reason}");
        return;
    }
    harness::assert_sql_job_cleanup_uses_terminal_transition(&backend).await;
}
matrix!(
    terminal_commit_releases_live_reference,
    assert_terminal_commit_releases_live_reference
);
matrix!(
    terminal_commit_retains_rollback_window,
    assert_terminal_commit_retains_rollback_window
);
matrix!(
    terminal_commit_rejects_incompatible_reference_transition,
    assert_terminal_commit_rejects_incompatible_reference_transition
);
matrix!(
    expired_rollbacks_are_released,
    assert_expired_rollbacks_are_released
);
matrix!(job_dispatch_fencing, assert_job_dispatch_fencing);
matrix!(
    job_dispatch_same_processor_aba_is_fenced,
    assert_job_dispatch_same_processor_aba_is_fenced
);
matrix!(
    control_queue_same_processor_aba_is_fenced,
    assert_control_queue_same_processor_aba_is_fenced
);
matrix!(
    control_queue_release_returns_row_for_redelivery,
    assert_control_queue_release_returns_row_for_redelivery
);
matrix!(
    job_dispatch_routes_by_plugin_superset,
    assert_job_dispatch_routes_by_plugin_superset
);

#[rstest]
#[case::in_memory(
    in_memory(),
    "in-memory",
    "NEBULA_CLAIM_FENCING_IN_MEMORY_OBSERVATIONS_PATH"
)]
#[case::sqlite(sqlite(), "sqlite", "NEBULA_CLAIM_FENCING_SQLITE_OBSERVATIONS_PATH")]
#[case::postgres(
    postgres(),
    "postgresql",
    "NEBULA_CLAIM_FENCING_POSTGRES_OBSERVATIONS_PATH"
)]
#[tokio::test]
async fn claim_generation_raw_observations(
    #[case] backend: Box<dyn Backend>,
    #[case] backend_name: &str,
    #[case] env: &str,
) {
    if let Some(reason) = skip_reason(backend.as_ref()) {
        eprintln!("WARN [conformance] {reason}");
        return;
    }
    let control = assert_control_queue_same_processor_aba_is_fenced(backend.as_ref()).await;
    let job = assert_job_dispatch_same_processor_aba_is_fenced(backend.as_ref()).await;
    let Ok(path) = std::env::var(env) else {
        return;
    };
    let path = std::path::Path::new(&path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap();
    serde_json::to_writer_pretty(
        std::io::BufWriter::new(file),
        &serde_json::json!({
            "producer_version": 1,
            "contract": "claim-generation-fencing",
            "scenario_inventory_version": 1,
            "backend": backend_name,
            "queues": [control, job]
        }),
    )
    .unwrap();
}

#[rstest]
#[case::in_memory(
    in_memory(),
    "in-memory",
    "NEBULA_PERSISTENCE_AUTHORITY_IN_MEMORY_OBSERVATIONS_PATH"
)]
#[case::sqlite(
    sqlite(),
    "sqlite",
    "NEBULA_PERSISTENCE_AUTHORITY_SQLITE_OBSERVATIONS_PATH"
)]
#[case::postgres(
    postgres(),
    "postgresql",
    "NEBULA_PERSISTENCE_AUTHORITY_POSTGRES_OBSERVATIONS_PATH"
)]
#[tokio::test]
async fn persistence_authority_raw_observations(
    #[case] backend: Box<dyn Backend>,
    #[case] backend_name: &str,
    #[case] env: &str,
) {
    if let Some(reason) = skip_reason(backend.as_ref()) {
        eprintln!("WARN [conformance] {reason}");
        return;
    }
    let owner_fencing = assert_stale_fencing_is_fenced_out(backend.as_ref()).await;
    let lease_recovery = assert_live_lease_blocks_acquire(backend.as_ref()).await;
    let atomic_transition = assert_atomic_triple(backend.as_ref()).await;
    let publication_atomicity =
        assert_save_with_published_version_is_atomic(backend.as_ref()).await;
    let Ok(path) = std::env::var(env) else {
        return;
    };
    let path = std::path::Path::new(&path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap();
    serde_json::to_writer_pretty(
        std::io::BufWriter::new(file),
        &serde_json::json!({
            "producer_version": 1,
            "contract": "persistence-authority",
            "scenario_inventory_version": 1,
            "backend": backend_name,
            "owner_fencing": owner_fencing,
            "lease_recovery": lease_recovery,
            "atomic_transition": atomic_transition,
            "publication_atomicity": publication_atomicity
        }),
    )
    .unwrap();
}

// ── Scoped variant ────────────────────────────────────────────────────────
// The same contract suite, but every store is wrapped in the
// `nebula-tenancy` decorators (bound to one tenant). This proves the
// decorator is *transparent* for same-tenant operations: an assertion
// that runs purely within `scope_a` must stay green when every call is
// forced through the decorator. Cross-tenant *denial* (the security
// property the decorator adds) is proven in `cross_tenant_denial.rs`.
//
// Only the purely-`scope_a` assertions are included. The `cross_scope_*`
// / journal / webhook assertions pass an explicit foreign scope to probe
// the adapter's raw `WHERE` filtering — the decorator substitutes that
// away, so they are exercised in the dedicated denial suite instead.

fn scoped_in_memory() -> Box<dyn Backend> {
    Box::new(ScopedBackend::<InMemoryBackend>::default())
}

fn scoped_sqlite() -> Box<dyn Backend> {
    Box::new(ScopedBackend::<SqliteBackend>::default())
}

fn scoped_postgres() -> Box<dyn Backend> {
    Box::new(ScopedBackend::<PostgresBackend>::default())
}

macro_rules! scoped_matrix {
    ($name:ident, $assertion:path) => {
        #[rstest]
        #[case::in_memory(scoped_in_memory())]
        #[case::sqlite(scoped_sqlite())]
        #[case::postgres(scoped_postgres())]
        #[tokio::test]
        async fn $name(#[case] backend: Box<dyn Backend>) {
            run(backend, |b| async move {
                let _observation = $assertion(b.as_ref()).await;
            })
            .await;
        }
    };
}

scoped_matrix!(scoped_create_get_roundtrip, assert_create_get_roundtrip);
scoped_matrix!(scoped_cas_conflict_returns_actual, assert_cas_conflict);
scoped_matrix!(
    scoped_stale_fencing_is_fenced_out,
    assert_stale_fencing_is_fenced_out
);
scoped_matrix!(
    scoped_live_lease_blocks_acquire,
    assert_live_lease_blocks_acquire
);
scoped_matrix!(scoped_atomic_triple_all_or_nothing, assert_atomic_triple);
scoped_matrix!(
    scoped_idempotency_first_writer_wins,
    assert_idempotency_first_writer_wins
);
scoped_matrix!(
    scoped_control_queue_outbox_and_fencing,
    assert_control_queue_outbox_and_fencing
);
scoped_matrix!(
    scoped_idempotency_store_first_writer,
    assert_idempotency_store_first_writer
);
scoped_matrix!(
    scoped_workflow_store_contract,
    assert_workflow_store_contract
);
scoped_matrix!(
    scoped_save_with_published_version_is_atomic,
    assert_save_with_published_version_is_atomic
);
scoped_matrix!(
    scoped_get_published_is_highest_numbered,
    assert_get_published_is_highest_numbered
);

matrix!(job_dispatch_exact_flavor, assert_job_dispatch_exact_flavor);
