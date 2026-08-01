//! C7 component-only expected-RED probe for same-processor-ID ABA fencing.
//!
//! This is deliberately not a product-root scenario: it exercises the
//! `JobDispatchQueue` component contract directly. Raw SQL is fixture-only and
//! limited to exact-ID timestamp backdating plus cleanup.

use std::time::Duration;

use nebula_core::PluginKey;
use nebula_storage::inmem::{InMemoryExecutionStore, InMemoryJobDispatchQueue};
use nebula_storage::sqlite::{SqliteJobDispatchQueue, init_schema as sqlite_init_schema};
use nebula_storage_port::dto::{ControlCommand, JobDispatchMsg};
use nebula_storage_port::store::{JobClaim, JobClaimToken, JobDispatchQueue};
use nebula_storage_port::{Scope, StorageError};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use uuid::Uuid;

const RECLAIM_AFTER: Duration = Duration::from_secs(1);
const MAX_RECLAIM_COUNT: u32 = 2;
const EXPECTED_RED_REASON: &str = "c7-same-processor-aba-accepted";

struct Scenario {
    processor_id: [u8; 16],
    plugin: PluginKey,
    jobs: [JobDispatchMsg; 2],
}

impl Scenario {
    fn unique() -> Self {
        let nonce = Uuid::new_v4().simple().to_string();
        let ack_id = *Uuid::new_v4().as_bytes();
        let nack_id = *Uuid::new_v4().as_bytes();
        assert_ne!(ack_id, nack_id, "SETUP: C7 job ids must be unique");

        let scope = Scope::new(format!("c7_workspace_{nonce}"), format!("c7_org_{nonce}"));
        let plugin = PluginKey::new(format!("c7_plugin_{nonce}"))
            .expect("SETUP: unique C7 plugin key must be valid");
        let ack = make_job(
            ack_id,
            &format!("c7_ack_execution_{nonce}"),
            scope.clone(),
            plugin.clone(),
            "late-ack",
            &nonce,
        );
        let nack = make_job(
            nack_id,
            &format!("c7_nack_execution_{nonce}"),
            scope,
            plugin.clone(),
            "late-nack",
            &nonce,
        );

        Self {
            processor_id: *Uuid::new_v4().as_bytes(),
            plugin,
            jobs: [ack, nack],
        }
    }

    fn ids(&self) -> [[u8; 16]; 2] {
        [self.jobs[0].id, self.jobs[1].id]
    }
}

fn make_job(
    id: [u8; 16],
    execution_id: &str,
    scope: Scope,
    plugin: PluginKey,
    role: &str,
    nonce: &str,
) -> JobDispatchMsg {
    JobDispatchMsg::new(
        id,
        execution_id,
        ControlCommand::Start,
        scope,
        serde_json::json!({"component": "c7", "role": role, "nonce": nonce}),
        Some(format!("c7_event_{role}_{nonce}")),
        format!("c7_flavor_{nonce}"),
        plugin.clone(),
        vec![plugin],
        None::<String>,
        0,
    )
}

/// Claim both jobs as generation N and keep the tokens that claim minted.
///
/// The tokens are the whole point of the scenario: they are the authority the
/// stale worker will present *after* the row has moved on to generation N+1.
async fn enqueue_and_claim_generation_n(
    queue: &impl JobDispatchQueue,
    scenario: &Scenario,
) -> [JobClaimToken; 2] {
    for job in &scenario.jobs {
        queue
            .enqueue(job)
            .await
            .expect("SETUP: enqueue C7 job through the component port");
    }

    let claimed = queue
        .claim_pending(
            &scenario.processor_id,
            2,
            std::slice::from_ref(&scenario.plugin),
        )
        .await
        .expect("SETUP: same processor claims logical generation N");
    assert_claimed(&claimed, scenario.ids(), 0, "logical generation N");
    tokens_for(&claimed, scenario)
}

/// Order the claim's tokens to match `Scenario::jobs` (`[ack, nack]`), so the
/// late acknowledgement and the late failure each address their own row.
fn tokens_for(claimed: &[JobClaim], scenario: &Scenario) -> [JobClaimToken; 2] {
    scenario.jobs.each_ref().map(|job| {
        claimed
            .iter()
            .find(|claim| claim.msg.id == job.id)
            .unwrap_or_else(|| panic!("SETUP: claim batch is missing C7 job {:?}", job.id))
            .token
    })
}

async fn reclaim_and_claim_generation_n_plus_one(
    queue: &impl JobDispatchQueue,
    scenario: &Scenario,
) -> [JobClaimToken; 2] {
    let outcome = queue
        .reclaim_stuck(RECLAIM_AFTER, MAX_RECLAIM_COUNT)
        .await
        .expect("SETUP: reclaim C7 generation N");
    assert_eq!(
        outcome.reclaimed, 2,
        "SETUP: both exact C7 jobs must be reclaimed"
    );
    assert_eq!(
        outcome.exhausted, 0,
        "SETUP: neither C7 job may exhaust its reclaim budget"
    );

    let reclaimed = queue
        .claim_pending(
            &scenario.processor_id,
            2,
            std::slice::from_ref(&scenario.plugin),
        )
        .await
        .expect("SETUP: same processor claims logical generation N+1");
    assert_claimed(&reclaimed, scenario.ids(), 1, "logical generation N+1");
    tokens_for(&reclaimed, scenario)
}

fn assert_claimed(
    claimed: &[JobClaim],
    mut expected_ids: [[u8; 16]; 2],
    expected_reclaim_count: u32,
    generation: &str,
) {
    let mut actual_ids: Vec<[u8; 16]> = claimed.iter().map(|claim| claim.msg.id).collect();
    actual_ids.sort_unstable();
    expected_ids.sort_unstable();
    assert_eq!(
        actual_ids,
        expected_ids.as_slice(),
        "SETUP: {generation} must contain both exact C7 job ids"
    );
    assert!(
        claimed
            .iter()
            .all(|claim| claim.msg.reclaim_count == expected_reclaim_count),
        "SETUP: {generation} must expose reclaim_count={expected_reclaim_count}; got {claimed:?}"
    );
}

/// Present generation N's tokens after the rows moved to generation N+1.
///
/// Both calls carry the *same* processor identity that owns generation N+1, so
/// any fence built on processor identity alone accepts them.
async fn stale_generation_n_results(
    queue: &impl JobDispatchQueue,
    generation_n: &[JobClaimToken; 2],
) -> (Result<(), StorageError>, Result<(), StorageError>) {
    let late_ack = queue.mark_dispatched(&generation_n[0]).await;
    let late_nack = queue
        .mark_failed(&generation_n[1], "late logical-generation-N failure")
        .await;
    (late_ack, late_nack)
}

#[expect(
    clippy::print_stderr,
    reason = "strict JUnit verifier consumes exact RED marker"
)]
fn assert_fenced_or_emit_expected_red(
    late_ack: Result<(), StorageError>,
    late_nack: Result<(), StorageError>,
) {
    assert!(
        late_ack.is_ok() || matches!(&late_ack, Err(StorageError::FencedOut { .. })),
        "SETUP: C7 stale mark_dispatched returned an unrelated storage error: {late_ack:?}"
    );
    assert!(
        late_nack.is_ok() || matches!(&late_nack, Err(StorageError::FencedOut { .. })),
        "SETUP: C7 stale mark_failed returned an unrelated storage error: {late_nack:?}"
    );

    if late_ack.is_ok() || late_nack.is_ok() {
        eprintln!("EXPECTED_RED:{EXPECTED_RED_REASON}");
        panic!("C7 same-processor-ID ABA behavioral oracle is expected to be RED");
    }

    assert!(
        matches!(late_ack, Err(StorageError::FencedOut { .. })),
        "C7 stale logical generation N mark_dispatched must be FencedOut; got {late_ack:?}"
    );
    assert!(
        matches!(late_nack, Err(StorageError::FencedOut { .. })),
        "C7 stale logical generation N mark_failed must be FencedOut; got {late_nack:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn same_processor_id_aba_in_memory() {
    let scenario = Scenario::unique();
    let store = InMemoryExecutionStore::new();
    let queue = InMemoryJobDispatchQueue::new(&store);

    let generation_n = enqueue_and_claim_generation_n(&queue, &scenario).await;
    tokio::time::advance(Duration::from_secs(2)).await;
    let _generation_n_plus_one = reclaim_and_claim_generation_n_plus_one(&queue, &scenario).await;
    let (late_ack, late_nack) = stale_generation_n_results(&queue, &generation_n).await;

    drop(queue);
    drop(store);
    assert_fenced_or_emit_expected_red(late_ack, late_nack);
}

#[tokio::test]
async fn same_processor_id_aba_file_sqlite() {
    let scenario = Scenario::unique();
    let tempdir = tempfile::tempdir().expect("SETUP: create isolated C7 SQLite directory");
    let database_path = tempdir.path().join("component-c7.sqlite");
    let options = SqliteConnectOptions::new()
        .filename(&database_path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("SETUP: connect C7 file SQLite pool");
    sqlite_init_schema(&pool)
        .await
        .expect("SETUP: install C7 SQLite component schema");
    let queue = SqliteJobDispatchQueue::new(pool.clone());

    let generation_n = enqueue_and_claim_generation_n(&queue, &scenario).await;
    let ids = scenario.ids();
    let backdated = sqlx::query(
        "UPDATE port_job_dispatch_queue \
         SET processed_at_ms = 0 \
         WHERE status = 'Processing' AND (id = ? OR id = ?)",
    )
    .bind(ids[0].as_slice())
    .bind(ids[1].as_slice())
    .execute(&pool)
    .await
    .expect("SETUP: exact-ID SQLite C7 stale barrier")
    .rows_affected();
    assert_eq!(
        backdated, 2,
        "SETUP: SQLite C7 stale barrier must backdate both exact ids"
    );

    let _generation_n_plus_one = reclaim_and_claim_generation_n_plus_one(&queue, &scenario).await;
    let (late_ack, late_nack) = stale_generation_n_results(&queue, &generation_n).await;

    let deleted = sqlx::query("DELETE FROM port_job_dispatch_queue WHERE id = ? OR id = ?")
        .bind(ids[0].as_slice())
        .bind(ids[1].as_slice())
        .execute(&pool)
        .await
        .expect("SETUP: clean exact C7 SQLite fixture rows")
        .rows_affected();
    assert_eq!(deleted, 2, "SETUP: clean both C7 SQLite fixture rows");
    drop(queue);
    pool.close().await;
    tempdir
        .close()
        .expect("SETUP: remove isolated C7 SQLite files");

    assert_fenced_or_emit_expected_red(late_ack, late_nack);
}

#[tokio::test]
async fn same_processor_id_aba_live_postgres() {
    use nebula_storage::postgres::{PgJobDispatchQueue, init_schema as postgres_init_schema};
    use sqlx::postgres::PgPoolOptions;

    let database_url =
        std::env::var("DATABASE_URL").expect("SETUP: live PostgreSQL requires DATABASE_URL");
    let scenario = Scenario::unique();
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("SETUP: connect live PostgreSQL for C7");
    postgres_init_schema(&pool)
        .await
        .expect("SETUP: install C7 PostgreSQL component schema");
    let queue = PgJobDispatchQueue::new(pool.clone());

    let generation_n = enqueue_and_claim_generation_n(&queue, &scenario).await;
    let ids = scenario.ids();
    let backdated = sqlx::query(
        "UPDATE port_job_dispatch_queue \
         SET processed_at_ms = 0 \
         WHERE status = 'Processing' AND (id = $1 OR id = $2)",
    )
    .bind(ids[0].as_slice())
    .bind(ids[1].as_slice())
    .execute(&pool)
    .await
    .expect("SETUP: exact-ID PostgreSQL C7 stale barrier")
    .rows_affected();
    assert_eq!(
        backdated, 2,
        "SETUP: PostgreSQL C7 stale barrier must backdate both exact ids"
    );

    let _generation_n_plus_one = reclaim_and_claim_generation_n_plus_one(&queue, &scenario).await;
    let (late_ack, late_nack) = stale_generation_n_results(&queue, &generation_n).await;

    let deleted = sqlx::query("DELETE FROM port_job_dispatch_queue WHERE id = $1 OR id = $2")
        .bind(ids[0].as_slice())
        .bind(ids[1].as_slice())
        .execute(&pool)
        .await
        .expect("SETUP: clean exact C7 PostgreSQL fixture rows")
        .rows_affected();
    assert_eq!(deleted, 2, "SETUP: clean both C7 PostgreSQL fixture rows");
    drop(queue);
    pool.close().await;

    assert_fenced_or_emit_expected_red(late_ack, late_nack);
}
