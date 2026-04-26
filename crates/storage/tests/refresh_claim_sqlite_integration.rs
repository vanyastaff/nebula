//! SQLite integration tests for `SqliteRefreshClaimRepo`.
//!
//! Builds a fresh in-memory SQLite pool, runs migrations 0022 + 0023,
//! and exercises the full `RefreshClaimRepo` contract.

#![cfg(feature = "sqlite")]

use std::time::Duration;

use nebula_core::CredentialId;
use nebula_storage::credential::{
    ClaimAttempt, HeartbeatError, RefreshClaimRepo, ReplicaId, SentinelState,
    SqliteRefreshClaimRepo,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// Construct a fresh pool with claim tables installed. Single-connection so
/// every test sees an isolated in-memory DB.
async fn fresh_pool() -> sqlx::SqlitePool {
    let options = SqliteConnectOptions::new()
        .in_memory(true)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("connect sqlite memory");

    // Run only the two migrations our tests care about. Embedding the
    // SQL inline avoids dragging in the full storage migration set
    // (which references unrelated tables).
    sqlx::query(include_str!(
        "../migrations/sqlite/0022_credential_refresh_claims.sql"
    ))
    .execute(&pool)
    .await
    .expect("apply 0022");
    sqlx::query(include_str!(
        "../migrations/sqlite/0023_credential_sentinel_events.sql"
    ))
    .execute(&pool)
    .await
    .expect("apply 0023");

    pool
}

#[tokio::test]
async fn try_claim_acquire_then_release_then_reacquire() {
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool);
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

    // After release, a new acquirer wins. Generation resets because the row
    // was deleted (matches the in-memory contract).
    let second = match repo
        .try_claim(&cid, &ReplicaId::new("B"), Duration::from_secs(30))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
    };
    assert_eq!(second.token.generation, 0);
}

#[tokio::test]
async fn try_claim_returns_contended_when_held() {
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool);
    let cid = CredentialId::new();

    let _first = repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(30))
        .await
        .unwrap();

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
}

#[tokio::test]
async fn heartbeat_extends_expiry_and_rejects_stale_token() {
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool);
    let cid = CredentialId::new();

    let claim = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(5))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
    };

    // Heartbeat with the live token succeeds.
    repo.heartbeat(&claim.token, Duration::from_secs(5))
        .await
        .expect("heartbeat live");

    // Heartbeat with a bumped generation fails.
    let stale = nebula_storage::credential::ClaimToken {
        claim_id: claim.token.claim_id,
        generation: claim.token.generation + 1,
    };
    let result = repo.heartbeat(&stale, Duration::from_secs(5)).await;
    assert!(matches!(result, Err(HeartbeatError::ClaimLost)));
}

#[tokio::test]
async fn mark_sentinel_then_reclaim_returns_in_flight_state() {
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool);
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
    assert_eq!(reclaimed.len(), 1);
    assert_eq!(reclaimed[0].credential_id, cid);
    assert_eq!(
        reclaimed[0].previous_holder,
        ReplicaId::new("crashed-replica")
    );
    assert_eq!(reclaimed[0].previous_generation, 0);
    assert_eq!(reclaimed[0].sentinel, SentinelState::RefreshInFlight);

    // Subsequent reclaim sees no rows.
    let again = repo.reclaim_stuck().await.unwrap();
    assert!(again.is_empty());
}

#[tokio::test]
async fn concurrent_try_claim_across_pool_clones_yields_one_acquired() {
    let pool = fresh_pool().await;
    let repo_a = SqliteRefreshClaimRepo::new(pool.clone());
    let repo_b = SqliteRefreshClaimRepo::new(pool.clone());
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
    // Single pool with max_connections=1 serialises queries, so exactly
    // one wins and one sees Contended.
    assert_eq!(acquired_count, 1, "exactly one acquirer should win");
    assert_eq!(contended_count, 1, "the other must see Contended");
}

#[tokio::test]
async fn concurrent_reclaim_returns_each_stuck_row_to_exactly_one_sweeper() {
    // Per sub-spec §3.4: each stuck row must be reclaimed by exactly one
    // sweeper across concurrent sweeps. Two sweepers observing the same
    // expired row both as `RefreshInFlight` would double-count toward the
    // N=3 sentinel-event ReauthRequired threshold.
    let pool = fresh_pool().await;
    let repo1 = SqliteRefreshClaimRepo::new(pool.clone());
    let repo2 = SqliteRefreshClaimRepo::new(pool.clone());

    // Insert N expired rows, marking each as RefreshInFlight to mimic the
    // worst case (a presumed mid-IdP-call crash).
    let mut credential_ids = Vec::new();
    for _ in 0..50 {
        let cid = CredentialId::new();
        let claim = match repo1
            .try_claim(&cid, &ReplicaId::new("setup"), Duration::from_millis(50))
            .await
            .unwrap()
        {
            ClaimAttempt::Acquired(c) => c,
            ClaimAttempt::Contended { .. } => panic!("setup must always acquire"),
        };
        repo1.mark_sentinel(&claim.token).await.unwrap();
        credential_ids.push(cid);
    }
    // Wait past expiry so reclaim_stuck has work to do.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Two sweepers race.
    let (a, b) = tokio::join!(repo1.reclaim_stuck(), repo2.reclaim_stuck());
    let a = a.expect("sweeper a");
    let b = b.expect("sweeper b");

    // Each row should appear in exactly one sweeper's result.
    let a_ids: std::collections::HashSet<_> = a.iter().map(|r| r.credential_id).collect();
    let b_ids: std::collections::HashSet<_> = b.iter().map(|r| r.credential_id).collect();
    let overlap = a_ids.intersection(&b_ids).count();
    assert_eq!(
        overlap, 0,
        "no row should appear in both sweepers (would double-count toward N=3)"
    );
    assert_eq!(
        a_ids.len() + b_ids.len(),
        50,
        "every row reclaimed by exactly one sweeper"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Stage 3.2: sentinel-event recording + rolling-window count
// ──────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn record_sentinel_event_and_count_in_window() {
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool);

    let cid = CredentialId::new();
    let holder = ReplicaId::new("replica-A");

    // Empty window → 0.
    let window_start = chrono::Utc::now() - chrono::Duration::seconds(10);
    let count = repo
        .count_sentinel_events_in_window(&cid, window_start)
        .await
        .unwrap();
    assert_eq!(count, 0, "no events recorded yet");

    // Record three events in quick succession; all must fall inside
    // a 10s window.
    repo.record_sentinel_event(&cid, &holder, 1).await.unwrap();
    repo.record_sentinel_event(&cid, &holder, 2).await.unwrap();
    repo.record_sentinel_event(&cid, &holder, 3).await.unwrap();

    let count = repo
        .count_sentinel_events_in_window(&cid, window_start)
        .await
        .unwrap();
    assert_eq!(count, 3, "three events inside the rolling window");
}

#[tokio::test]
async fn sentinel_count_filters_by_credential_id() {
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool);

    let cid_a = CredentialId::new();
    let cid_b = CredentialId::new();
    let holder = ReplicaId::new("replica-A");

    repo.record_sentinel_event(&cid_a, &holder, 1)
        .await
        .unwrap();
    repo.record_sentinel_event(&cid_a, &holder, 2)
        .await
        .unwrap();
    repo.record_sentinel_event(&cid_b, &holder, 1)
        .await
        .unwrap();

    let window_start = chrono::Utc::now() - chrono::Duration::seconds(10);
    let count_a = repo
        .count_sentinel_events_in_window(&cid_a, window_start)
        .await
        .unwrap();
    let count_b = repo
        .count_sentinel_events_in_window(&cid_b, window_start)
        .await
        .unwrap();
    assert_eq!(count_a, 2, "credential A has 2 events");
    assert_eq!(count_b, 1, "credential B has 1 event");
}

#[tokio::test]
async fn sentinel_count_excludes_events_before_window_start() {
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool);

    let cid = CredentialId::new();
    let holder = ReplicaId::new("replica-A");

    // Record an event, then sleep past the window we're going to query.
    repo.record_sentinel_event(&cid, &holder, 1).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Window starts AFTER the recorded event.
    let window_start = chrono::Utc::now() - chrono::Duration::milliseconds(50);
    let count = repo
        .count_sentinel_events_in_window(&cid, window_start)
        .await
        .unwrap();
    assert_eq!(count, 0, "event predates window_start; must not be counted");

    // Record a fresh event and re-query; only the new one falls inside.
    repo.record_sentinel_event(&cid, &holder, 2).await.unwrap();
    let count = repo
        .count_sentinel_events_in_window(&cid, window_start)
        .await
        .unwrap();
    assert_eq!(count, 1, "only the post-window event counts");
}
