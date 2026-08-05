//! Dispatch-claim → execution-turn handoff conformance (SQLite backend).
//!
//! The property NS05 turns on is that an action's duration never extends the
//! dispatch claim. These cases prove the claim is *finished* at handoff, so
//! there is nothing left to extend.

#![cfg(feature = "sqlite")]

use std::time::Duration;

use nebula_core::PluginKey;
use std::str::FromStr;

use nebula_storage::sqlite::{
    SqliteExecutionStore, SqliteJobDispatchQueue, SqliteTurnHandoff, init_schema,
};
use nebula_storage_port::dto::{ControlCommand, JobDispatchMsg};
use nebula_storage_port::store::{
    ExecutionStore, ExecutionTurnHandoff, JobDispatchQueue, TurnAcceptance, TurnHandoff,
};
use nebula_storage_port::{Scope, StorageError};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

const TTL: Duration = Duration::from_secs(30);

/// Processor identities are observability labels; the claim token carries the
/// authority, so two distinct processors is all these cases need.
const PROCESSOR_A: [u8; 16] = [0xa1; 16];
const PROCESSOR_B: [u8; 16] = [0xb2; 16];

fn scope() -> Scope {
    Scope::new("ws-handoff", "org-handoff")
}

/// An isolated in-memory database with the ordered migration catalog applied.
async fn fresh_pool() -> SqlitePool {
    let database = format!("nebula-handoff-{}", uuid::Uuid::new_v4());
    let url = format!("sqlite:file:{database}?mode=memory&cache=shared");
    let options = SqliteConnectOptions::from_str(&url)
        .expect("in-memory SQLite URL must parse")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .expect("connect to in-memory SQLite");
    init_schema(&pool)
        .await
        .expect("apply the ordered SQLite migration catalog");
    pool
}

struct Fixture {
    store: SqliteExecutionStore,
    queue: SqliteJobDispatchQueue,
    handoff: SqliteTurnHandoff,
    pool: SqlitePool,
}

impl Fixture {
    async fn new() -> Self {
        let pool = fresh_pool().await;
        let store = SqliteExecutionStore::new(pool.clone());
        let queue = SqliteJobDispatchQueue::new(pool.clone());
        let handoff = SqliteTurnHandoff::new(pool.clone());
        Self {
            store,
            queue,
            handoff,
            pool,
        }
    }

    async fn seed(&self, execution_id: &str) -> nebula_storage_port::store::JobClaimToken {
        self.store
            .create(&scope(), execution_id, "wf", serde_json::json!({}))
            .await
            .expect("the execution row is created");
        let plugin: PluginKey = "demo".parse().expect("the fixture plugin key is valid");
        let msg = JobDispatchMsg::new(
            *uuid::Uuid::new_v4().as_bytes(),
            execution_id.to_owned(),
            ControlCommand::Start,
            scope(),
            serde_json::json!({}),
            None::<String>,
            "flavor-sha",
            plugin.clone(),
            vec![plugin],
            None::<String>,
            0,
        );
        self.queue.enqueue(&msg).await.expect("the job enqueues");
        let plugin: PluginKey = "demo".parse().expect("the fixture plugin key is valid");
        let claimed = self
            .queue
            .claim_pending(&PROCESSOR_A, 1, &[plugin])
            .await
            .expect("the job is claimable");
        claimed
            .into_iter()
            .next()
            .expect("exactly one job was claimable")
            .token
    }

    fn request<'a>(
        &self,
        execution_id: &'a str,
        claim: nebula_storage_port::store::JobClaimToken,
        holder: &'a str,
        scope_ref: &'a Scope,
    ) -> TurnHandoff<'a> {
        TurnHandoff {
            scope: scope_ref,
            execution_id,
            claim,
            holder,
            lease_ttl: TTL,
        }
    }
}

/// The happy path: the turn is owned and the claim is finished in one commit.
#[tokio::test]
async fn accepting_a_turn_acknowledges_the_claim_in_one_commit() {
    let fixture = Fixture::new().await;
    let scope = scope();
    let execution = "exe-accept";
    let claim = fixture.seed(execution).await;

    let accepted = fixture
        .handoff
        .accept_turn(&fixture.request(execution, claim, "worker-a", &scope))
        .await
        .expect("a fresh claim on an unleased execution hands off");
    let TurnAcceptance::Accepted { fence } = accepted else {
        panic!("expected an accepted turn, got {accepted:?}");
    };
    assert!(
        fence.generation() > 0,
        "an accepted turn must carry a live fence"
    );

    // The claim is already terminal, so nothing about the action's duration
    // can extend it — acknowledging again is refused.
    assert!(matches!(
        fixture.queue.mark_dispatched(&claim).await,
        Err(StorageError::FencedOut { .. } | StorageError::NotFound { .. })
    ));
}

/// A superseded claim cannot take the turn, and writes nothing.
#[tokio::test]
async fn a_superseded_claim_cannot_accept_the_turn() {
    let fixture = Fixture::new().await;
    let scope = scope();
    let execution = "exe-superseded";
    let stale = fixture.seed(execution).await;

    // Age the claim so the sweep is guaranteed to see it, rather than racing
    // the clock. SQLite's reclaim predicate is `processed_at_ms < cutoff`
    // (strict) while the in-memory model uses `elapsed >= reclaim_after`
    // (inclusive), so a zero window reclaims immediately on one backend and
    // never on the other — a boundary divergence worth naming, and one this
    // test must not depend on either way.
    sqlx::query(
        "UPDATE port_job_dispatch_queue SET processed_at_ms = 0 WHERE status = 'Processing'",
    )
    .execute(&fixture.pool)
    .await
    .expect("the claim is backdated");

    // A reclaim sweep hands the row to someone else, bumping the generation.
    fixture
        .queue
        .reclaim_stuck(Duration::from_secs(0), 8)
        .await
        .expect("the sweep runs");
    let plugin: PluginKey = "demo".parse().expect("the fixture plugin key is valid");
    let fresh = fixture
        .queue
        .claim_pending(&PROCESSOR_B, 1, &[plugin])
        .await
        .expect("the reclaimed job is claimable again");
    assert_eq!(fresh.len(), 1, "the sweep returned the row to the queue");

    assert_eq!(
        fixture
            .handoff
            .accept_turn(&fixture.request(execution, stale, "worker-a", &scope))
            .await
            .expect("a superseded claim is a typed outcome, not an error"),
        TurnAcceptance::ClaimSuperseded
    );

    // Nothing was written: the execution still has no lease, so the current
    // claim holder can still take the turn.
    let taken = fixture
        .handoff
        .accept_turn(&fixture.request(
            execution,
            fresh.into_iter().next().expect("one job").token,
            "worker-b",
            &scope,
        ))
        .await
        .expect("the current claim holder can take the turn");
    assert!(
        matches!(taken, TurnAcceptance::Accepted { .. }),
        "a rejected handoff must leave the execution takeable, got {taken:?}"
    );
}

/// A live lease held by another owner blocks the turn and leaves the queue row
/// claimable, so the work is redelivered rather than dropped.
#[tokio::test]
async fn a_live_foreign_lease_blocks_the_turn_without_acknowledging_the_row() {
    let fixture = Fixture::new().await;
    let scope = scope();
    let execution = "exe-contended";
    let claim = fixture.seed(execution).await;

    fixture
        .store
        .acquire_lease(&scope, execution, "worker-z", TTL)
        .await
        .expect("the competing lease is acquirable")
        .expect("no lease existed yet");

    assert_eq!(
        fixture
            .handoff
            .accept_turn(&fixture.request(execution, claim, "worker-a", &scope))
            .await
            .expect("contention is a typed outcome, not an error"),
        TurnAcceptance::TurnHeldByAnotherOwner
    );

    // The row was NOT acknowledged: acknowledging would make it terminal while
    // no owner ever ran the turn.
    fixture
        .queue
        .mark_dispatched(&claim)
        .await
        .expect("the claim still owns a live row");
}

/// A handoff for another tenant's execution is refused.
#[tokio::test]
async fn a_foreign_tenant_cannot_accept_the_turn() {
    let fixture = Fixture::new().await;
    let execution = "exe-tenant";
    let claim = fixture.seed(execution).await;
    let intruder = Scope::new("ws-other", "org-other");

    // The claim predicate carries the tenant, so a foreign scope fails there
    // first and answers `ClaimSuperseded` rather than reporting anything about
    // the execution. That is the stronger reply: it does not reveal whether the
    // execution exists in some other tenant.
    assert_eq!(
        fixture
            .handoff
            .accept_turn(&fixture.request(execution, claim, "worker-a", &intruder))
            .await
            .expect("a foreign tenant is a typed outcome, not an error"),
        TurnAcceptance::ClaimSuperseded
    );
}
