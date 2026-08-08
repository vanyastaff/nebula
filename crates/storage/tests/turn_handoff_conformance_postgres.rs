//! Dispatch-claim → execution-turn handoff conformance (PostgreSQL backend).
//!
//! PostgreSQL is a deployment backend, so its absence is a job failure rather
//! than a silent substitution: with `NEBULA_REQUIRE_POSTGRES=1` and no
//! `DATABASE_URL`, every case fails.
//!
//! The property NS05 turns on is that an action's duration never extends the
//! dispatch claim. These cases prove the claim is *finished* at handoff, so
//! there is nothing left to extend.

#![cfg(feature = "postgres")]

use std::time::Duration;

use nebula_core::PluginKey;

use nebula_storage::postgres::{PgExecutionStore, PgJobDispatchQueue, PgTurnHandoff, init_schema};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::OnceCell;

static SCHEMA_READY: OnceCell<()> = OnceCell::const_new();
use nebula_storage_port::dto::{ControlCommand, JobDispatchMsg};
use nebula_storage_port::store::{
    ExecutionStore, ExecutionTurnHandoff, JobDispatchQueue, TurnAcceptance, TurnHandoff,
};
use nebula_storage_port::{Scope, StorageError};

const TTL: Duration = Duration::from_secs(30);

/// Processor identities are observability labels; the claim token carries the
/// authority, so two distinct processors is all these cases need.
const PROCESSOR_A: [u8; 16] = [0xa1; 16];
const PROCESSOR_B: [u8; 16] = [0xb2; 16];

/// Execution ids unique per process, so cases share one database without
/// meeting an earlier run's rows.
fn unique(label: &str) -> String {
    format!("exe-{label}-{}", uuid::Uuid::new_v4().simple())
}

fn scope() -> Scope {
    Scope::new("ws-handoff", "org-handoff")
}

/// Connect to `DATABASE_URL` and apply the ordered migration catalog, or report
/// that PostgreSQL is unreachable.
async fn fresh_pool() -> Option<PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            assert_ne!(
                std::env::var("NEBULA_REQUIRE_POSTGRES").as_deref(),
                Ok("1"),
                "DATABASE_URL must be set when NEBULA_REQUIRE_POSTGRES=1: \
                 PostgreSQL is a deployment backend and is never substituted"
            );
            return None;
        },
        Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
    };
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    SCHEMA_READY
        .get_or_init(|| async {
            init_schema(&pool)
                .await
                .expect("apply the ordered PostgreSQL migration catalog");
        })
        .await;
    Some(pool)
}

struct Fixture {
    store: PgExecutionStore,
    queue: PgJobDispatchQueue,
    handoff: PgTurnHandoff,
    pool: PgPool,
}

impl Fixture {
    async fn new() -> Option<Self> {
        let pool = fresh_pool().await?;
        let store = PgExecutionStore::new(pool.clone());
        let queue = PgJobDispatchQueue::new(pool.clone());
        let handoff = PgTurnHandoff::new(pool.clone());
        Some(Self {
            store,
            queue,
            handoff,
            pool,
        })
    }

    /// A plugin key unique to one execution.
    ///
    /// The required PostgreSQL job runs against a shared `DATABASE_URL`, so a
    /// shared routing key would let `claim_pending` return another case's
    /// pending row. Binding the key to the execution keeps each case claiming
    /// only what it enqueued.
    fn plugin_for(execution_id: &str) -> PluginKey {
        execution_id
            .replace('-', "")
            .parse()
            .expect("a hex execution id is a valid plugin key")
    }

    async fn seed(&self, execution_id: &str) -> nebula_storage_port::store::JobClaimToken {
        self.store
            .create(&scope(), execution_id, "wf", serde_json::json!({}))
            .await
            .expect("the execution row is created");
        let plugin = Self::plugin_for(execution_id);
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
        let claimed = self
            .queue
            .claim_pending(&PROCESSOR_A, 1, &[Self::plugin_for(execution_id)])
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
    let Some(fixture) = Fixture::new().await else {
        eprintln!("PostgreSQL unreachable in this environment");
        return;
    };
    let scope = scope();
    let execution = &unique("accept");
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
    let Some(fixture) = Fixture::new().await else {
        eprintln!("PostgreSQL unreachable in this environment");
        return;
    };
    let scope = scope();
    let execution = &unique("superseded");
    let stale = fixture.seed(execution).await;

    // Age the claim so the sweep is guaranteed to see it, rather than racing
    // the clock. Postgres's reclaim predicate is `processed_at_ms < cutoff`
    // (strict) while the in-memory model uses `elapsed >= reclaim_after`
    // (inclusive), so a zero window reclaims immediately on one backend and
    // never on the other — a boundary divergence worth naming, and one this
    // test must not depend on either way.
    sqlx::query(
        "UPDATE port_job_dispatch_queue SET processed_at_ms = 0 \
         WHERE status = 'Processing' AND execution_id = $1",
    )
    .bind(execution)
    .execute(&fixture.pool)
    .await
    .expect("the claim is backdated");

    // A reclaim sweep hands the row to someone else, bumping the generation.
    fixture
        .queue
        .reclaim_stuck(Duration::from_secs(0), 8)
        .await
        .expect("the sweep runs");
    let fresh = fixture
        .queue
        .claim_pending(&PROCESSOR_B, 1, &[Fixture::plugin_for(execution)])
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
    let Some(fixture) = Fixture::new().await else {
        eprintln!("PostgreSQL unreachable in this environment");
        return;
    };
    let scope = scope();
    let execution = &unique("contended");
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
    let Some(fixture) = Fixture::new().await else {
        eprintln!("PostgreSQL unreachable in this environment");
        return;
    };
    let scope = scope();
    let execution = &unique("tenant");
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

    // The refusal wrote nothing: the original handoff, retried with its own
    // scope and claim, is still accepted — the claim was not consumed and the
    // execution was not leased under the foreign scope.
    let taken = fixture
        .handoff
        .accept_turn(&fixture.request(execution, claim, "worker-a", &scope))
        .await
        .expect("the original handoff still runs after the foreign refusal");
    assert!(
        matches!(taken, TurnAcceptance::Accepted { .. }),
        "a foreign-tenant refusal must leave the original claim intact, got {taken:?}"
    );
}

/// A claim token is authority over *one* row, not a bearer pass.
///
/// Pairing a valid token from one job with a different execution id would
/// otherwise lease the wrong aggregate and acknowledge — dropping — the job
/// that was actually claimed.
#[tokio::test]
async fn a_claim_token_cannot_be_paired_with_another_execution() {
    let Some(fixture) = Fixture::new().await else {
        eprintln!("PostgreSQL unreachable in this environment");
        return;
    };
    let scope = scope();
    let claimed = unique("claimed");
    let other = unique("other");
    let claim = fixture.seed(&claimed).await;
    fixture
        .store
        .create(&scope, &other, "wf", serde_json::json!({}))
        .await
        .expect("the second execution row is created");

    assert_eq!(
        fixture
            .handoff
            .accept_turn(&fixture.request(&other, claim, "worker-a", &scope))
            .await
            .expect("a mismatched pairing is a typed outcome, not an error"),
        TurnAcceptance::ClaimSuperseded,
        "a token proves ownership of one row, so it cannot drive another execution"
    );

    // Neither side moved: the claimed row is still acknowledgeable, and the
    // unrelated execution was never leased.
    fixture
        .queue
        .mark_dispatched(&claim)
        .await
        .expect("the claimed row is untouched");
    fixture
        .store
        .acquire_lease(&scope, &other, "worker-b", TTL)
        .await
        .expect("the unrelated execution is reachable")
        .expect("the unrelated execution was never leased");
}
