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
    ClaimAttempt, ClaimToken, HeartbeatError, PgRefreshClaimRepo, RefreshClaimRepo, ReplicaId,
    SentinelState,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::OnceCell;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");
static SCHEMA_READY: OnceCell<()> = OnceCell::const_new();

/// Connect to `DATABASE_URL`, apply migrations, or return `None` to
/// instruct the test to skip. Mirrors the pattern used by
/// `crates/storage/src/pg/control_queue.rs` tests.
async fn pool() -> Option<PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => return None,
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
async fn mark_sentinel_then_reclaim_returns_in_flight_state() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let repo = PgRefreshClaimRepo::new(pool);
    let cid = CredentialId::new();

    let claim = match repo
        .try_claim(
            &cid,
            &ReplicaId::new("crashed-replica"),
            Duration::from_millis(50),
        )
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
    };
    repo.mark_sentinel(&claim.token).await.unwrap();

    tokio::time::sleep(Duration::from_millis(120)).await;

    let reclaimed = repo.reclaim_stuck().await.unwrap();
    let our_row = reclaimed
        .iter()
        .find(|r| r.credential_id == cid)
        .expect("our credential should be in the reclaim batch");
    assert_eq!(our_row.previous_holder, ReplicaId::new("crashed-replica"));
    assert_eq!(our_row.previous_generation, 0);
    assert_eq!(our_row.sentinel, SentinelState::RefreshInFlight);
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
