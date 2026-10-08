//! Repeated execution-lease contention through the spec-16 storage port.
//!
//! Four runners race over four executions for 200 rounds each. Every round
//! keeps the winning lease until all acquire attempts finish, so the oracle
//! observes ownership before release rather than counting a successor as an
//! overlapping holder during test bookkeeping. The issued generation must
//! increase on handoff, and superseded renew, release, and commit must leave
//! the successor unchanged. This exercises the in-memory reference adapter;
//! SQL-backend conformance and the small-interleaving loom probe are separate.
//!
//! Run explicitly with `cargo nextest run -p nebula-storage
//! --test execution_lease_chaos --run-ignored only`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use nebula_core::accessor::Clock;
use nebula_storage::inmem::InMemoryExecutionStore;
use nebula_storage_port::ids::FencingToken;
use nebula_storage_port::store::ExecutionStore;
use nebula_storage_port::{Scope, TransitionBatch, TransitionOutcome};
use tokio::sync::Barrier;

const RUNNERS: usize = 4;
const EXECUTIONS: usize = 4;
const ROUNDS: usize = 200;
const TTL: Duration = Duration::from_secs(30);

/// Contention and explicit release are the workload, so wall-clock pauses
/// must not turn a held lease into an expiry takeover.
struct FixedLeaseClock {
    wall: DateTime<Utc>,
    monotonic: Instant,
}

impl Clock for FixedLeaseClock {
    fn now(&self) -> DateTime<Utc> {
        self.wall
    }

    fn monotonic(&self) -> Instant {
        self.monotonic
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "explicit contention workload; nightly CI runs it with --run-ignored only"]
async fn concurrent_acquire_release_preserves_unique_holder_and_fencing() {
    let clock = Arc::new(FixedLeaseClock {
        wall: Utc::now(),
        monotonic: Instant::now(),
    });
    let store = Arc::new(InMemoryExecutionStore::with_clock(clock));
    let scope = Scope::new("lease-chaos-workspace", "lease-chaos-org");
    let execution_ids: Vec<_> = (0..EXECUTIONS)
        .map(|index| format!("lease-chaos-{index}"))
        .collect();
    for id in &execution_ids {
        store
            .create(&scope, id, "lease-chaos-workflow", serde_json::json!({}))
            .await
            .expect("reference execution must be created");
    }
    let mut previous_tokens = [None::<FencingToken>; EXECUTIONS];
    let mut handoffs = 0;

    for _round in 0..ROUNDS {
        for (index, id) in execution_ids.iter().enumerate() {
            let barrier = Arc::new(Barrier::new(RUNNERS));
            let mut runners = Vec::with_capacity(RUNNERS);
            for runner in 0..RUNNERS {
                let store = Arc::clone(&store);
                let scope = scope.clone();
                let id = id.clone();
                let barrier = Arc::clone(&barrier);
                runners.push(tokio::spawn(async move {
                    let holder = format!("runner-{runner}");
                    barrier.wait().await;
                    let token = store
                        .acquire_lease(&scope, &id, &holder, TTL)
                        .await
                        .expect("reference acquire must succeed or report contention");
                    (holder, token)
                }));
            }
            let mut winners = Vec::new();
            for runner in runners {
                let (holder, token) = runner.await.expect("lease contender must finish");
                if let Some(token) = token {
                    winners.push((holder, token));
                }
            }
            assert_eq!(winners.len(), 1, "exactly one contender must own {id}");
            let (holder, token) = winners.pop().expect("one winner was asserted");
            let before = store
                .get(&scope, id)
                .await
                .expect("read winner")
                .expect("execution remains present");
            assert_eq!(before.lease_holder.as_deref(), Some(holder.as_str()));
            assert_eq!(before.fencing, Some(token.generation()));
            assert!(
                store
                    .acquire_lease(&scope, id, &holder, TTL)
                    .await
                    .expect("same-holder acquire must report contention")
                    .is_none(),
                "a live lease cannot be reacquired even by the same holder"
            );

            if let Some(stale) = previous_tokens[index] {
                assert!(token.generation() > stale.generation());
                assert!(
                    !store
                        .renew_lease(&scope, id, stale, TTL)
                        .await
                        .expect("stale renewal must be rejected")
                );
                assert!(
                    !store
                        .release_lease(&scope, id, stale)
                        .await
                        .expect("stale release must be rejected")
                );
                let batch = TransitionBatch::new(
                    scope.clone(),
                    id.clone(),
                    before.version,
                    stale,
                    serde_json::json!({"stale_write": true}),
                    nebula_storage_port::ExecutionListing::CREATED,
                );
                assert!(matches!(
                    store
                        .commit(batch)
                        .await
                        .expect("stale commit must be rejected"),
                    TransitionOutcome::FencedOut
                ));
                assert_eq!(
                    store
                        .get(&scope, id)
                        .await
                        .expect("read after stale operations"),
                    Some(before),
                    "superseded operations must leave the successor unchanged"
                );
                handoffs += 1;
            }
            assert!(
                store
                    .renew_lease(&scope, id, token, TTL)
                    .await
                    .expect("current holder must renew")
            );
            assert!(
                store
                    .release_lease(&scope, id, token)
                    .await
                    .expect("current holder must release")
            );
            assert!(
                store
                    .get(&scope, id)
                    .await
                    .expect("read after release")
                    .expect("release retains the execution")
                    .lease_holder
                    .is_none()
            );
            previous_tokens[index] = Some(token);
        }
    }
    assert_eq!(handoffs, EXECUTIONS * (ROUNDS - 1));
}
