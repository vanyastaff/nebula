//! Postgres integration tests for `PgRefreshClaimRepo`.
//!
//! Follows the existing storage convention: tests skip silently when
//! `DATABASE_URL` is absent, fail loudly when it is set but unparsable.
//! Each test scopes its rows to a randomly-generated `CredentialId` so
//! parallel runs do not collide on the `credential_id` PRIMARY KEY.
//!
//! Run locally via:
//!   DATABASE_URL=postgres://... cargo nextest run \
//!     -p nebula-storage --features postgres \
//!     --test refresh_claim_pg_integration

#![cfg(feature = "postgres")]

use std::time::Duration;

use nebula_core::CredentialId;
use nebula_storage::credential::{
    ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, PgRefreshClaimRepo, RefreshClaimRepo,
    ReplicaId, RepoError,
};
use sqlx::{PgPool, Postgres, pool::PoolConnection, postgres::PgPoolOptions};
use tokio::sync::OnceCell;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");
static SCHEMA_READY: OnceCell<()> = OnceCell::const_new();
const RECLAIM_TEST_LOCK_KEY: i64 = 0x4E42_5246_434C_414D;

/// Connect to `DATABASE_URL`, apply migrations, or return `None` to
/// instruct the test to skip. Mirrors the pattern used by
/// `crates/storage/src/pg/control_queue.rs` tests.
async fn pool() -> Option<PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            assert_ne!(
                std::env::var("NEBULA_REQUIRE_POSTGRES").as_deref(),
                Ok("1"),
                "DATABASE_URL must be set when NEBULA_REQUIRE_POSTGRES=1"
            );
            return None;
        },
        Err(err) => panic!("DATABASE_URL is set but invalid: {err}"),
    };
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect");
    SCHEMA_READY
        .get_or_init(|| async {
            MIGRATOR
                .run(&pool)
                .await
                .expect("apply storage postgres migrations");
        })
        .await;
    Some(pool)
}

/// Serialize tests that invoke the global reclaim sweep.
///
/// Each nextest case may run in a separate process, so an in-process mutex
/// cannot prevent one test from consuming another test's expired row. A
/// session advisory lock coordinates across processes. `close_on_drop`
/// guarantees a panic retires the session instead of returning a still-locked
/// connection to the pool.
async fn acquire_reclaim_test_lock(pool: &PgPool) -> PoolConnection<Postgres> {
    let mut connection = pool
        .acquire()
        .await
        .expect("acquire reclaim-test connection");
    connection.close_on_drop();
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(RECLAIM_TEST_LOCK_KEY)
        .execute(&mut *connection)
        .await
        .expect("acquire reclaim-test advisory lock");
    connection
}

#[tokio::test]
async fn try_claim_acquire_then_release_then_reacquire() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let repo = PgRefreshClaimRepo::new(pool);
    let cid = CredentialId::new();

    let first = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(30))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
        ClaimAttempt::OutcomeUnknown { .. } => panic!("fresh claim cannot be poisoned"),
    };
    assert_eq!(first.token.generation, 0);
    assert_eq!(first.credential_id, cid);

    repo.release(first.token.clone()).await.unwrap();

    let second = match repo
        .try_claim(&cid, &ReplicaId::new("B"), Duration::from_secs(30))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
        ClaimAttempt::OutcomeUnknown { .. } => panic!("released claim cannot be poisoned"),
    };
    assert_eq!(second.token.generation, 0);

    repo.release(second.token).await.unwrap();
}

#[tokio::test]
async fn try_claim_returns_contended_when_held() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let repo = PgRefreshClaimRepo::new(pool);
    let cid = CredentialId::new();

    let first = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(30))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
        ClaimAttempt::OutcomeUnknown { .. } => panic!("fresh claim cannot be poisoned"),
    };

    let second = repo
        .try_claim(&cid, &ReplicaId::new("B"), Duration::from_secs(30))
        .await
        .unwrap();

    match second {
        ClaimAttempt::Contended {
            existing_expires_at,
        } => {
            assert!(existing_expires_at > chrono::Utc::now());
        },
        ClaimAttempt::Acquired(_) => panic!("expected Contended"),
        ClaimAttempt::OutcomeUnknown { .. } => panic!("live claim cannot be poisoned"),
    }

    repo.release(first.token).await.unwrap();
}

#[tokio::test]
async fn heartbeat_extends_expiry_and_rejects_stale_token() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let repo = PgRefreshClaimRepo::new(pool);
    let cid = CredentialId::new();

    let claim = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(5))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
        ClaimAttempt::OutcomeUnknown { .. } => panic!("fresh claim cannot be poisoned"),
    };

    repo.heartbeat(&claim.token, Duration::from_secs(5))
        .await
        .expect("heartbeat live");

    let stale = ClaimToken {
        claim_id: claim.token.claim_id,
        generation: claim.token.generation + 1,
    };
    let result = repo.heartbeat(&stale, Duration::from_secs(5)).await;
    assert!(matches!(result, Err(HeartbeatError::ClaimLost)));

    repo.release(claim.token).await.unwrap();
}

#[tokio::test]
async fn out_of_range_generation_is_rejected_without_touching_the_claim() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let repo = PgRefreshClaimRepo::new(pool);
    let cid = CredentialId::new();
    let claim = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(30))
        .await
        .expect("acquire claim")
    {
        ClaimAttempt::Acquired(claim) => claim,
        ClaimAttempt::Contended { .. } | ClaimAttempt::OutcomeUnknown { .. } => {
            panic!("fresh credential must be claimable")
        },
    };
    let forged = ClaimToken {
        claim_id: claim.token.claim_id,
        generation: u64::MAX,
    };

    assert!(matches!(
        repo.heartbeat(&forged, Duration::from_secs(30)).await,
        Err(HeartbeatError::Repo(RepoError::InvalidState))
    ));
    assert!(matches!(
        repo.mark_sentinel(&forged).await,
        Err(RepoError::InvalidState)
    ));
    assert!(matches!(
        repo.release(forged).await,
        Err(RepoError::InvalidState)
    ));
    assert!(matches!(
        repo.try_claim(&cid, &ReplicaId::new("B"), Duration::from_secs(30))
            .await
            .expect("observe preserved claim"),
        ClaimAttempt::Contended { .. }
    ));
    repo.release(claim.token).await.expect("release real claim");
}

#[tokio::test]
async fn mark_sentinel_then_reclaim_accounts_retained_poison() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let _reclaim_lock = acquire_reclaim_test_lock(&pool).await;
    let repo = PgRefreshClaimRepo::new(pool.clone());
    let cid = CredentialId::new();

    let claim = match repo
        .try_claim(
            &cid,
            &ReplicaId::new("crashed-replica"),
            Duration::from_secs(30),
        )
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
        ClaimAttempt::OutcomeUnknown { .. } => panic!("fresh claim cannot be poisoned"),
    };
    repo.mark_sentinel(&claim.token).await.unwrap();

    sqlx::query("UPDATE credential_refresh_claims SET expires_at = $1 WHERE credential_id = $2")
        .bind(chrono::Utc::now() - chrono::Duration::seconds(1))
        .bind(cid.to_string())
        .execute(&pool)
        .await
        .expect("backdate claim");

    let reclaimed = repo.reclaim_stuck().await.unwrap();
    let our_row = reclaimed
        .iter()
        .find(|outcome| {
            matches!(
                outcome,
                ExpiredClaim::OutcomeUnknownAccounted {
                    credential_id,
                    ..
                } if *credential_id == cid
            )
        })
        .expect("our credential should be in the reclaim batch");
    assert!(matches!(
        our_row,
        ExpiredClaim::OutcomeUnknownAccounted {
            previous_holder,
            previous_generation: 0,
            ..
        } if *previous_holder == ReplicaId::new("crashed-replica")
    ));
}

#[tokio::test]
async fn mark_sentinel_after_reclaim_returns_invalid_state() {
    // Contract (sub-spec §3.4 + trait doc): once a claim has been reclaimed
    // (row removed by `reclaim_stuck`, or generation bumped by an in-place
    // overwrite), the original holder's `mark_sentinel` MUST return
    // `RepoError::InvalidState` so the holder cannot proceed to the IdP
    // POST while another replica owns the credential. Silent success would
    // re-introduce the pre-fix race that this trait change closed.
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let _reclaim_lock = acquire_reclaim_test_lock(&pool).await;
    let repo = PgRefreshClaimRepo::new(pool.clone());
    let cid = CredentialId::new();

    let token = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(30))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c.token,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
        ClaimAttempt::OutcomeUnknown { .. } => panic!("fresh claim cannot be poisoned"),
    };

    sqlx::query("UPDATE credential_refresh_claims SET expires_at = $1 WHERE credential_id = $2")
        .bind(chrono::Utc::now() - chrono::Duration::seconds(1))
        .bind(cid.to_string())
        .execute(&pool)
        .await
        .expect("backdate claim");
    let _reclaimed = repo.reclaim_stuck().await.unwrap();

    let err = repo
        .mark_sentinel(&token)
        .await
        .expect_err("mark_sentinel must fail after reclaim");
    assert!(
        matches!(err, RepoError::InvalidState),
        "expected InvalidState, got {err:?}"
    );
}

#[tokio::test]
async fn expired_claim_cannot_be_marked_in_flight() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let repo = PgRefreshClaimRepo::new(pool.clone());
    let cid = CredentialId::new();

    let claim = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(30))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(claim) => claim,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
        ClaimAttempt::OutcomeUnknown { .. } => panic!("fresh claim cannot be poisoned"),
    };

    sqlx::query("UPDATE credential_refresh_claims SET expires_at = $1 WHERE credential_id = $2")
        .bind(chrono::Utc::now() - chrono::Duration::seconds(1))
        .bind(cid.to_string())
        .execute(&pool)
        .await
        .expect("backdate claim");

    let error = repo
        .mark_sentinel(&claim.token)
        .await
        .expect_err("an expired claim must not authorize provider egress");
    assert!(matches!(error, RepoError::InvalidState));

    let (sentinel,): (i16,) =
        sqlx::query_as("SELECT sentinel FROM credential_refresh_claims WHERE credential_id = $1")
            .bind(cid.to_string())
            .fetch_one(&pool)
            .await
            .expect("read preserved claim");
    assert_eq!(sentinel, 0, "rejected mark must not mutate sentinel state");

    let cleanup = match repo
        .try_claim(&cid, &ReplicaId::new("cleanup"), Duration::from_secs(30))
        .await
        .expect("replace expired Normal claim")
    {
        ClaimAttempt::Acquired(claim) => claim,
        ClaimAttempt::Contended { .. } | ClaimAttempt::OutcomeUnknown { .. } => {
            panic!("expired Normal claim must be replaceable")
        },
    };
    repo.release(cleanup.token).await.expect("cleanup claim");
}

#[tokio::test]
async fn expired_in_flight_claim_is_preserved_until_reclaim() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let _reclaim_lock = acquire_reclaim_test_lock(&pool).await;
    let repo = PgRefreshClaimRepo::new(pool.clone());
    let cid = CredentialId::new();

    let first = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(30))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(claim) => claim,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
        ClaimAttempt::OutcomeUnknown { .. } => panic!("fresh claim cannot be poisoned"),
    };
    repo.mark_sentinel(&first.token)
        .await
        .expect("live holder may mark provider egress");

    sqlx::query("UPDATE credential_refresh_claims SET expires_at = $1 WHERE credential_id = $2")
        .bind(chrono::Utc::now() - chrono::Duration::seconds(1))
        .bind(cid.to_string())
        .execute(&pool)
        .await
        .expect("backdate claim");

    let attempt = repo
        .try_claim(&cid, &ReplicaId::new("B"), Duration::from_secs(30))
        .await
        .expect("poisoned acquisition");
    assert!(
        matches!(attempt, ClaimAttempt::OutcomeUnknown { .. }),
        "try_claim must fail closed on an expired in-flight claim"
    );
    let repeated = repo
        .try_claim(&cid, &ReplicaId::new("C"), Duration::from_secs(30))
        .await
        .expect("repeated poisoned acquisition");
    assert!(matches!(repeated, ClaimAttempt::OutcomeUnknown { .. }));

    let reclaimed = repo.reclaim_stuck().await.expect("reclaim expired claim");
    let reclaimed = reclaimed
        .iter()
        .find(|outcome| {
            matches!(
                outcome,
                ExpiredClaim::OutcomeUnknownAccounted {
                    credential_id,
                    ..
                } if *credential_id == cid
            )
        })
        .expect("our expired row must be reclaimed");
    assert!(matches!(
        reclaimed,
        ExpiredClaim::OutcomeUnknownAccounted {
            previous_holder,
            previous_generation: 0,
            ..
        } if *previous_holder == ReplicaId::new("A")
    ));
    let recorded = repo
        .count_sentinel_events_in_window(&cid, Duration::from_mins(1))
        .await
        .expect("count atomically recorded evidence");
    assert_eq!(
        recorded, 1,
        "reclaim must durably account in-flight evidence while retaining poison"
    );

    let next = repo
        .try_claim(&cid, &ReplicaId::new("B"), Duration::from_secs(30))
        .await
        .expect("poison after accounting");
    assert!(matches!(next, ClaimAttempt::OutcomeUnknown { .. }));
    assert!(
        repo.reclaim_stuck().await.unwrap().is_empty(),
        "retained poison evidence must be recorded exactly once"
    );
}

#[tokio::test]
async fn expired_normal_claim_can_be_taken_over_in_place() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let repo = PgRefreshClaimRepo::new(pool.clone());
    let cid = CredentialId::new();

    let first = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(30))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(claim) => claim,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
        ClaimAttempt::OutcomeUnknown { .. } => panic!("fresh claim cannot be poisoned"),
    };
    sqlx::query("UPDATE credential_refresh_claims SET expires_at = $1 WHERE credential_id = $2")
        .bind(chrono::Utc::now() - chrono::Duration::seconds(1))
        .bind(cid.to_string())
        .execute(&pool)
        .await
        .expect("backdate claim");

    let second = repo
        .try_claim(&cid, &ReplicaId::new("B"), Duration::from_secs(30))
        .await
        .expect("expired normal takeover");
    let ClaimAttempt::Acquired(second) = second else {
        panic!("expired normal claim must remain directly reclaimable");
    };
    assert_eq!(second.token.generation, first.token.generation + 1);
}

#[tokio::test]
async fn exact_confirmed_release_clears_expired_in_flight_claim() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let repo = PgRefreshClaimRepo::new(pool.clone());
    let cid = CredentialId::new();

    let claim = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(30))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(claim) => claim,
        ClaimAttempt::Contended { .. } | ClaimAttempt::OutcomeUnknown { .. } => {
            panic!("fresh credential must be claimable")
        },
    };
    repo.mark_sentinel(&claim.token)
        .await
        .expect("mark provider boundary");
    sqlx::query(
        "UPDATE credential_refresh_claims \
         SET expires_at = CURRENT_TIMESTAMP - INTERVAL '1 second' \
         WHERE credential_id = $1",
    )
    .bind(cid.to_string())
    .execute(&pool)
    .await
    .expect("backdate claim");

    repo.release(claim.token)
        .await
        .expect("exact confirmed finalization");
    let next = repo
        .try_claim(&cid, &ReplicaId::new("B"), Duration::from_secs(30))
        .await
        .expect("claim after exact finalization");
    assert!(matches!(next, ClaimAttempt::Acquired(_)));
}

#[tokio::test]
async fn old_generation_zero_evidence_does_not_mask_a_new_claim_lifecycle() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let _reclaim_lock = acquire_reclaim_test_lock(&pool).await;
    let repo = PgRefreshClaimRepo::new(pool.clone());
    let cid = CredentialId::new();
    let holder = ReplicaId::new("same-holder");

    let first = match repo
        .try_claim(&cid, &holder, Duration::from_secs(30))
        .await
        .expect("first claim")
    {
        ClaimAttempt::Acquired(claim) => claim,
        ClaimAttempt::Contended { .. } | ClaimAttempt::OutcomeUnknown { .. } => {
            panic!("fresh credential must be claimable")
        },
    };
    repo.mark_sentinel(&first.token)
        .await
        .expect("mark first provider boundary");
    sqlx::query(
        "UPDATE credential_refresh_claims \
         SET expires_at = CURRENT_TIMESTAMP - INTERVAL '1 second' \
         WHERE credential_id = $1",
    )
    .bind(cid.to_string())
    .execute(&pool)
    .await
    .expect("backdate first claim");
    assert!(
        repo.reclaim_stuck()
            .await
            .expect("account first poison")
            .iter()
            .any(|outcome| matches!(
                outcome,
                ExpiredClaim::OutcomeUnknownAccounted { credential_id, .. }
                    if *credential_id == cid
            ))
    );
    repo.release(first.token)
        .await
        .expect("exactly finalize first lifecycle");

    let second = match repo
        .try_claim(&cid, &holder, Duration::from_secs(30))
        .await
        .expect("second claim")
    {
        ClaimAttempt::Acquired(claim) => claim,
        ClaimAttempt::Contended { .. } | ClaimAttempt::OutcomeUnknown { .. } => {
            panic!("released credential must be claimable")
        },
    };
    assert_eq!(
        second.token.generation, 0,
        "a new row demonstrates why generation alone is not event identity"
    );
    repo.mark_sentinel(&second.token)
        .await
        .expect("mark second provider boundary");
    sqlx::query(
        "UPDATE credential_refresh_claims \
         SET expires_at = CURRENT_TIMESTAMP - INTERVAL '1 second' \
         WHERE credential_id = $1",
    )
    .bind(cid.to_string())
    .execute(&pool)
    .await
    .expect("backdate second claim");

    assert!(
        repo.reclaim_stuck()
            .await
            .expect("account second poison")
            .iter()
            .any(|outcome| matches!(
                outcome,
                ExpiredClaim::OutcomeUnknownAccounted { credential_id, .. }
                    if *credential_id == cid
            )),
        "evidence from the prior row lifecycle must not suppress new poison"
    );
    assert_eq!(
        repo.count_sentinel_events_in_window(&cid, Duration::from_mins(1))
            .await
            .expect("count both lifecycle events"),
        2
    );
}

#[tokio::test]
async fn concurrent_try_claim_yields_one_acquired() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let repo_a = PgRefreshClaimRepo::new(pool.clone());
    let repo_b = PgRefreshClaimRepo::new(pool.clone());
    let cid = CredentialId::new();

    let cid_a = cid;
    let cid_b = cid;
    let (a, b) = tokio::join!(
        async move {
            repo_a
                .try_claim(&cid_a, &ReplicaId::new("A"), Duration::from_secs(30))
                .await
                .unwrap()
        },
        async move {
            repo_b
                .try_claim(&cid_b, &ReplicaId::new("B"), Duration::from_secs(30))
                .await
                .unwrap()
        },
    );

    let acquired_count = [&a, &b]
        .into_iter()
        .filter(|o| matches!(o, ClaimAttempt::Acquired(_)))
        .count();
    let contended_count = [&a, &b]
        .into_iter()
        .filter(|o| matches!(o, ClaimAttempt::Contended { .. }))
        .count();
    assert_eq!(acquired_count, 1, "exactly one acquirer should win the CAS");
    assert_eq!(contended_count, 1, "the other must see Contended");

    // Cleanup: release whichever one won.
    let winner = match (a, b) {
        (ClaimAttempt::Acquired(c), _) | (_, ClaimAttempt::Acquired(c)) => c,
        _ => unreachable!("we asserted exactly one Acquired above"),
    };
    let cleanup_repo = PgRefreshClaimRepo::new(pool);
    cleanup_repo.release(winner.token).await.unwrap();
}

#[tokio::test]
async fn concurrent_reclaim_accounts_each_poison_exactly_once() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let _reclaim_lock = acquire_reclaim_test_lock(&pool).await;
    let repo_a = PgRefreshClaimRepo::new(pool.clone());
    let repo_b = PgRefreshClaimRepo::new(pool.clone());
    let holder = ReplicaId::new("crashed-replica");
    let mut expected = std::collections::HashSet::new();

    for _ in 0..24 {
        let cid = CredentialId::new();
        let claim = match repo_a
            .try_claim(&cid, &holder, Duration::from_secs(30))
            .await
            .expect("seed claim")
        {
            ClaimAttempt::Acquired(claim) => claim,
            ClaimAttempt::Contended { .. } | ClaimAttempt::OutcomeUnknown { .. } => {
                panic!("fresh credential must be claimable")
            },
        };
        repo_a
            .mark_sentinel(&claim.token)
            .await
            .expect("mark provider boundary");
        sqlx::query(
            "UPDATE credential_refresh_claims \
             SET expires_at = CURRENT_TIMESTAMP - INTERVAL '1 second' \
             WHERE credential_id = $1",
        )
        .bind(cid.to_string())
        .execute(&pool)
        .await
        .expect("backdate poison");
        expected.insert(cid);
    }

    let (left, right) = tokio::join!(repo_a.reclaim_stuck(), repo_b.reclaim_stuck());
    let left = left.expect("left sweep");
    let right = right.expect("right sweep");
    let accounted_id = |outcome: &ExpiredClaim| match outcome {
        ExpiredClaim::OutcomeUnknownAccounted { credential_id, .. } => Some(*credential_id),
        ExpiredClaim::ReclaimedNormal { .. } => None,
    };
    let left_ids: std::collections::HashSet<_> = left.iter().filter_map(accounted_id).collect();
    let right_ids: std::collections::HashSet<_> = right.iter().filter_map(accounted_id).collect();

    assert!(
        left_ids.is_disjoint(&right_ids),
        "one poisoned generation must be observed by only one sweeper"
    );
    let accounted: std::collections::HashSet<_> = left_ids.union(&right_ids).copied().collect();
    assert_eq!(accounted, expected);

    for cid in expected {
        let count = repo_a
            .count_sentinel_events_in_window(&cid, Duration::from_mins(1))
            .await
            .expect("count accounted poison");
        assert_eq!(count, 1, "each poisoned generation records one event");
        let attempt = repo_a
            .try_claim(&cid, &ReplicaId::new("retry"), Duration::from_secs(30))
            .await
            .expect("read retained poison");
        assert!(matches!(attempt, ClaimAttempt::OutcomeUnknown { .. }));
    }

    assert!(
        repo_a
            .reclaim_stuck()
            .await
            .expect("idempotent repeat sweep")
            .is_empty(),
        "repeat sweep must not duplicate accounted evidence"
    );
}
