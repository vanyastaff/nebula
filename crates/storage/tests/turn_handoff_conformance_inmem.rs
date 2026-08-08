//! Dispatch-claim → execution-turn handoff conformance (in-memory reference).
//!
//! The property NS05 turns on is that an action's duration never extends the
//! dispatch claim. These cases prove the claim is *finished* at handoff, so
//! there is nothing left to extend.

use std::time::Duration;

use nebula_core::PluginKey;
use nebula_storage::inmem::{
    InMemoryExecutionStore, InMemoryJobDispatchQueue, InMemoryTurnHandoff,
};
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

fn scope() -> Scope {
    Scope::new("ws-handoff", "org-handoff")
}

struct Fixture {
    store: InMemoryExecutionStore,
    queue: InMemoryJobDispatchQueue,
    handoff: InMemoryTurnHandoff,
}

impl Fixture {
    fn new() -> Self {
        let store = InMemoryExecutionStore::new();
        let queue = InMemoryJobDispatchQueue::new(&store);
        let handoff = InMemoryTurnHandoff::new(&store);
        Self {
            store,
            queue,
            handoff,
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
    let fixture = Fixture::new();
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
    let fixture = Fixture::new();
    let scope = scope();
    let execution = "exe-superseded";
    let stale = fixture.seed(execution).await;

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
    let fixture = Fixture::new();
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
    let fixture = Fixture::new();
    let scope = scope();
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
    let fixture = Fixture::new();
    let scope = scope();
    let claimed = "exe-claimed";
    let other = "exe-other";
    let claim = fixture.seed(claimed).await;
    fixture
        .store
        .create(&scope, other, "wf", serde_json::json!({}))
        .await
        .expect("the second execution row is created");

    assert_eq!(
        fixture
            .handoff
            .accept_turn(&fixture.request(other, claim, "worker-a", &scope))
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
        .acquire_lease(&scope, other, "worker-b", TTL)
        .await
        .expect("the unrelated execution is reachable")
        .expect("the unrelated execution was never leased");
}
