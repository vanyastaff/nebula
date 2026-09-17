//! Acknowledgement must fail closed whenever the caller does not hold the
//! row's current claim — an unknown row, or a token a reclaim superseded.
//! A silent `Ok` would let a worker believe it dispatched a job another
//! attempt now owns.
//!
//! A "wrong processor" case no longer exists by construction: authority is
//! the storage-minted token, and a processor cannot fabricate one. The
//! stronger ABA case (*same* processor, superseded generation) replaces it.

use nebula_storage_port::dto::{ControlCommand, JobDispatchMsg};
use nebula_storage_port::store::{ClaimGeneration, JobClaimToken, JobDispatchQueue};
use nebula_storage_port::{Scope, StorageError as SE};

use super::InMemoryJobDispatchQueue;
use crate::inmem::InMemoryExecutionStore;
use std::time::Duration;

fn make_queue() -> InMemoryJobDispatchQueue {
    let store = InMemoryExecutionStore::new();
    InMemoryJobDispatchQueue::new(&store)
}

fn sample_msg(id: [u8; 16]) -> JobDispatchMsg {
    JobDispatchMsg::new(
        id,
        "exec-1".to_owned(),
        ControlCommand::Start,
        Scope::new("ws-1", "org-1"),
        serde_json::Value::Null,
        None::<String>,
        "plugin-a".parse().unwrap(),
        vec!["plugin-a".parse().unwrap()],
        None::<String>,
        0,
        nebula_core::WorkerFlavorRevisionId::from_bytes([0x11; 32]),
    )
}

#[tokio::test]
async fn duplicate_enqueue_preserves_the_active_claim() {
    let queue = make_queue();
    let message = sample_msg([9; 16]);
    queue.enqueue(&message).await.unwrap();
    let claim = queue
        .claim_pending(
            &[2; 16],
            1,
            &["plugin-a".parse().unwrap()],
            nebula_core::WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .unwrap()
        .remove(0);

    assert!(matches!(
        queue.enqueue(&message).await,
        Err(SE::Duplicate {
            entity: "job_dispatch",
            ..
        })
    ));
    queue.mark_dispatched(&claim.token).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn cleanup_removes_only_expired_terminal_rows() {
    let queue = make_queue();
    let terminal = sample_msg([11; 16]);
    let pending = sample_msg([12; 16]);
    queue.enqueue(&terminal).await.unwrap();
    queue.enqueue(&pending).await.unwrap();
    let claim = queue
        .claim_pending(
            &[2; 16],
            1,
            &["plugin-a".parse().unwrap()],
            nebula_core::WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .unwrap()
        .remove(0);
    queue.mark_dispatched(&claim.token).await.unwrap();

    assert_eq!(queue.cleanup(Duration::from_secs(1)).await.unwrap(), 0);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(queue.cleanup(Duration::from_secs(1)).await.unwrap(), 0);
    tokio::time::advance(Duration::from_nanos(1)).await;
    assert_eq!(queue.cleanup(Duration::from_secs(1)).await.unwrap(), 1);

    let remaining = queue
        .claim_pending(
            &[2; 16],
            1,
            &["plugin-a".parse().unwrap()],
            nebula_core::WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].msg.id, pending.id);
}

#[tokio::test(start_paused = true)]
async fn cleanup_retention_starts_when_a_long_running_claim_becomes_terminal() {
    let queue = make_queue();
    let message = sample_msg([13; 16]);
    queue.enqueue(&message).await.unwrap();
    let claim = queue
        .claim_pending(
            &[2; 16],
            1,
            &["plugin-a".parse().unwrap()],
            nebula_core::WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .unwrap()
        .remove(0);

    tokio::time::advance(Duration::from_secs(10)).await;
    queue.mark_dispatched(&claim.token).await.unwrap();

    assert_eq!(queue.cleanup(Duration::from_secs(1)).await.unwrap(), 0);
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(queue.cleanup(Duration::from_secs(1)).await.unwrap(), 1);
}

#[tokio::test]
async fn exact_flavor_mismatch_cannot_hide_matching_job_in_bounded_batch() {
    let queue = make_queue();
    let advertised = nebula_core::WorkerFlavorRevisionId::from_bytes([0x11; 32]);
    let mut wrong = sample_msg([1; 16]);
    wrong.required_worker_flavor_id = nebula_core::WorkerFlavorRevisionId::from_bytes([0x22; 32]);
    let mut matching = sample_msg([2; 16]);
    matching.required_worker_flavor_id = advertised;
    queue.enqueue(&wrong).await.unwrap();
    queue.enqueue(&matching).await.unwrap();
    let claimed = queue
        .claim_pending(
            &[3; 16],
            1,
            &["plugin-a".parse().unwrap()],
            nebula_core::WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(
        claimed[0].msg.id, matching.id,
        "exact revision must filter before the batch limit"
    );
}

#[tokio::test]
async fn mark_dispatched_returns_not_found_for_unknown_job() {
    let queue = make_queue();
    let unknown = JobClaimToken::new(
        [0xABu8; 16],
        ClaimGeneration::new(1),
        Scope::new("ws-1", "org-1"),
    );

    let result = queue.mark_dispatched(&unknown).await;
    assert!(
        matches!(
            result,
            Err(SE::NotFound {
                entity: "job_dispatch",
                ..
            })
        ),
        "mark_dispatched on an unknown job id must return NotFound, got {result:?}"
    );
}

#[tokio::test]
async fn mark_dispatched_is_fenced_out_after_the_claim_is_reclaimed() {
    let queue = make_queue();
    let worker_a: [u8; 16] = [1u8; 16];
    let job_id: [u8; 16] = [3u8; 16];

    queue.enqueue(&sample_msg(job_id)).await.unwrap();
    let claimed = queue
        .claim_pending(
            &worker_a,
            1,
            &["plugin-a".parse().unwrap()],
            nebula_core::WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    let stale = claimed[0].token.clone();

    // The sweep hands the row back; the same worker claims it again. The
    // processor id is identical across both attempts, so only the
    // generation distinguishes them.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(2)).await;
    let outcome = queue
        .reclaim_stuck(Duration::from_secs(1), 5)
        .await
        .unwrap();
    assert_eq!(outcome.reclaimed, 1, "the stuck row must be reclaimed");
    let reclaimed = queue
        .claim_pending(
            &worker_a,
            1,
            &["plugin-a".parse().unwrap()],
            nebula_core::WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .unwrap();
    assert_eq!(reclaimed.len(), 1);
    assert!(
        reclaimed[0].token.generation() > stale.generation(),
        "a reclaimed row must mint a strictly greater generation"
    );

    let result = queue.mark_dispatched(&stale).await;
    assert!(
        matches!(
            result,
            Err(SE::FencedOut {
                entity: "job_dispatch",
                ..
            })
        ),
        "an acknowledgement from the superseded claim must be fenced out, got {result:?}"
    );

    // The fence must also be a no-op: the current claim still owns the row.
    assert!(
        queue.mark_dispatched(&reclaimed[0].token).await.is_ok(),
        "the current claim must still be able to acknowledge the row"
    );
}

#[tokio::test]
async fn mark_failed_is_fenced_out_for_a_superseded_generation() {
    let queue = make_queue();
    let worker_a: [u8; 16] = [1u8; 16];
    let job_id: [u8; 16] = [4u8; 16];

    queue.enqueue(&sample_msg(job_id)).await.unwrap();
    let claimed = queue
        .claim_pending(
            &worker_a,
            1,
            &["plugin-a".parse().unwrap()],
            nebula_core::WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .unwrap();
    let current = &claimed[0].token;
    let superseded = JobClaimToken::new(
        job_id,
        ClaimGeneration::new(current.generation().get() - 1),
        current.scope().clone(),
    );

    let result = queue.mark_failed(&superseded, "some error").await;
    assert!(
        matches!(
            result,
            Err(SE::FencedOut {
                entity: "job_dispatch",
                ..
            })
        ),
        "mark_failed with a superseded generation must be fenced out, got {result:?}"
    );
}

#[tokio::test]
async fn mark_dispatched_succeeds_for_the_current_claim() {
    let queue = make_queue();
    let worker_a: [u8; 16] = [1u8; 16];
    let job_id: [u8; 16] = [5u8; 16];

    queue.enqueue(&sample_msg(job_id)).await.unwrap();
    let claimed = queue
        .claim_pending(
            &worker_a,
            1,
            &["plugin-a".parse().unwrap()],
            nebula_core::WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .unwrap();

    let result = queue.mark_dispatched(&claimed[0].token).await;
    assert!(
        result.is_ok(),
        "the claim's own token must acknowledge the row, got {result:?}"
    );
}
