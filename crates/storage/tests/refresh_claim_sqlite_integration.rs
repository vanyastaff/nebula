//! SQLite integration tests for `SqliteRefreshClaimRepo`.
//!
//! Builds a fresh in-memory SQLite pool, runs migrations 0022 + 0023,
//! and exercises the full `RefreshClaimRepo` contract.

#![cfg(feature = "sqlite")]

use std::time::Duration;

use nebula_core::CredentialId;
use nebula_storage::credential::{
    ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, RefreshClaimRepo, ReplicaId, RepoError,
    SqliteRefreshClaimRepo,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// Construct a fresh pool with claim tables installed. Single-connection so
/// every test sees an isolated in-memory DB.
async fn fresh_pool() -> sqlx::SqlitePool {
    fresh_pool_with_capacity(1).await
}

/// Same as [`fresh_pool`] but allows callers to opt into a multi-connection
/// pool. Required for tests that want to genuinely exercise SQL-layer
/// concurrency (e.g. atomic poison-accounting single-winner contention),
/// because a `max_connections=1` pool serialises queries at the pool layer
/// before the SQL atomicity check runs — making the assertion trivially
/// true via serialisation rather than the SQL invariant being tested.
///
/// To make multiple connections see the same in-memory database, we open
/// it via a named URI (`file:<random>?mode=memory&cache=shared`). The
/// random name keeps each test isolated from concurrent ones in the same
/// process. (For real-DB tests this is irrelevant; on-disk SQLite shares
/// state via the file system.)
async fn fresh_pool_with_capacity(max_connections: u32) -> sqlx::SqlitePool {
    use std::str::FromStr;
    // Random DB name so concurrent tests in the same process don't share
    // tables. `?mode=memory&cache=shared` is the canonical SQLite recipe
    // for an in-memory DB visible across multiple connections.
    let db_name = format!("nebula-claim-test-{}", uuid::Uuid::new_v4());
    let url = format!("sqlite:file:{db_name}?mode=memory&cache=shared");
    let options = SqliteConnectOptions::from_str(&url)
        .expect("parse sqlite memory url")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
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
    // The production incident identity lands in shared migration 0039. This
    // focused fixture has no credentials table, so apply only that migration's
    // refresh-claim fragment after the immutable 0022/0023 history.
    sqlx::query("ALTER TABLE credential_sentinel_events ADD COLUMN claim_id TEXT")
        .execute(&pool)
        .await
        .expect("apply 0039 claim identity column");
    sqlx::query(
        "CREATE UNIQUE INDEX idx_credential_sentinel_events_claim_id \
         ON credential_sentinel_events(claim_id) \
         WHERE claim_id IS NOT NULL",
    )
    .execute(&pool)
    .await
    .expect("apply 0039 claim identity index");

    pool
}

async fn seed_sentinel_event(
    pool: &sqlx::SqlitePool,
    credential_id: &CredentialId,
    holder: &ReplicaId,
    generation: i64,
    detected_at_ms: i64,
) {
    sqlx::query(
        "INSERT INTO credential_sentinel_events \
         (credential_id, detected_at, crashed_holder, generation) \
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(credential_id.to_string())
    .bind(detected_at_ms)
    .bind(holder.as_str())
    .bind(generation)
    .execute(pool)
    .await
    .expect("seed test-only sentinel event");
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
        ClaimAttempt::OutcomeUnknown { .. } => panic!("fresh claim cannot be poisoned"),
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
        ClaimAttempt::OutcomeUnknown { .. } => panic!("released claim cannot be poisoned"),
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
        ClaimAttempt::OutcomeUnknown { .. } => panic!("live claim cannot be poisoned"),
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
        ClaimAttempt::OutcomeUnknown { .. } => panic!("fresh claim cannot be poisoned"),
    };

    // Heartbeat with the live token succeeds.
    repo.heartbeat(&claim.token, Duration::from_secs(5))
        .await
        .expect("heartbeat live");

    // Heartbeat with a bumped generation fails.
    let stale = ClaimToken {
        claim_id: claim.token.claim_id,
        generation: claim.token.generation + 1,
    };
    let result = repo.heartbeat(&stale, Duration::from_secs(5)).await;
    assert!(matches!(result, Err(HeartbeatError::ClaimLost)));
}

#[tokio::test]
async fn out_of_range_generation_is_rejected_without_touching_the_claim() {
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool);
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
        ClaimAttempt::OutcomeUnknown { .. } => panic!("fresh claim cannot be poisoned"),
    };
    repo.mark_sentinel(&claim.token).await.unwrap();

    tokio::time::sleep(Duration::from_millis(120)).await;

    let reclaimed = repo.reclaim_stuck().await.unwrap();
    assert_eq!(reclaimed.len(), 1);
    assert!(matches!(
        &reclaimed[0],
        ExpiredClaim::OutcomeUnknownAccounted {
            credential_id,
            previous_holder,
            previous_generation: 0,
        } if *credential_id == cid
            && *previous_holder == ReplicaId::new("crashed-replica")
    ));

    // Subsequent reclaim sees no rows.
    let again = repo.reclaim_stuck().await.unwrap();
    assert!(again.is_empty());
}

#[tokio::test]
async fn mark_sentinel_after_reclaim_returns_invalid_state() {
    // Contract (sub-spec §3.4 + trait doc): once a claim has been reclaimed
    // (row removed by `reclaim_stuck`, or generation bumped by an in-place
    // overwrite), the original holder's `mark_sentinel` MUST return
    // `RepoError::InvalidState` so the holder cannot proceed to the IdP
    // POST while another replica owns the credential. Silent success would
    // re-introduce the pre-fix race that this trait change closed.
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool);
    let cid = CredentialId::new();

    let token = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_millis(50))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c.token,
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
        ClaimAttempt::OutcomeUnknown { .. } => panic!("fresh claim cannot be poisoned"),
    };

    // Wait past TTL and sweep, deleting the row.
    tokio::time::sleep(Duration::from_millis(120)).await;
    let reclaimed = repo.reclaim_stuck().await.unwrap();
    assert_eq!(reclaimed.len(), 1, "the expired row must be reclaimed");
    assert!(matches!(reclaimed[0], ExpiredClaim::ReclaimedNormal { .. }));

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
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool.clone());
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

    sqlx::query("UPDATE credential_refresh_claims SET expires_at = ?1 WHERE credential_id = ?2")
        .bind(chrono::Utc::now().timestamp_millis() - 1_000)
        .bind(cid.to_string())
        .execute(&pool)
        .await
        .expect("backdate claim");

    let error = repo
        .mark_sentinel(&claim.token)
        .await
        .expect_err("an expired claim must not authorize provider egress");
    assert!(matches!(error, RepoError::InvalidState));

    let (sentinel,): (i64,) =
        sqlx::query_as("SELECT sentinel FROM credential_refresh_claims WHERE credential_id = ?1")
            .bind(cid.to_string())
            .fetch_one(&pool)
            .await
            .expect("read preserved claim");
    assert_eq!(sentinel, 0, "rejected mark must not mutate sentinel state");
}

#[tokio::test]
async fn expired_in_flight_claim_is_preserved_until_reclaim() {
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool.clone());
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

    sqlx::query("UPDATE credential_refresh_claims SET expires_at = ?1 WHERE credential_id = ?2")
        .bind(chrono::Utc::now().timestamp_millis() - 1_000)
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
    assert_eq!(reclaimed.len(), 1);
    assert!(matches!(
        &reclaimed[0],
        ExpiredClaim::OutcomeUnknownAccounted {
            credential_id,
            previous_holder,
            previous_generation: 0,
        } if *credential_id == cid && *previous_holder == ReplicaId::new("A")
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
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool.clone());
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
    sqlx::query("UPDATE credential_refresh_claims SET expires_at = ?1 WHERE credential_id = ?2")
        .bind(chrono::Utc::now().timestamp_millis() - 1_000)
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
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool.clone());
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
    sqlx::query("UPDATE credential_refresh_claims SET expires_at = ?1 WHERE credential_id = ?2")
        .bind(chrono::Utc::now().timestamp_millis() - 1_000)
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
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool.clone());
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
    sqlx::query("UPDATE credential_refresh_claims SET expires_at = ?1 WHERE credential_id = ?2")
        .bind(chrono::Utc::now().timestamp_millis() - 1_000)
        .bind(cid.to_string())
        .execute(&pool)
        .await
        .expect("backdate first claim");
    assert_eq!(
        repo.reclaim_stuck()
            .await
            .expect("account first poison")
            .len(),
        1
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
    sqlx::query("UPDATE credential_refresh_claims SET expires_at = ?1 WHERE credential_id = ?2")
        .bind(chrono::Utc::now().timestamp_millis() - 1_000)
        .bind(cid.to_string())
        .execute(&pool)
        .await
        .expect("backdate second claim");

    assert_eq!(
        repo.reclaim_stuck()
            .await
            .expect("account second poison")
            .len(),
        1,
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
async fn concurrent_try_claim_across_pool_clones_yields_one_acquired() {
    // Capacity ≥ 2 so the two `try_claim` calls genuinely race at the SQL
    // layer rather than queueing behind a single-connection pool. The
    // INSERT-on-conflict-DO-NOTHING (or equivalent) is atomic across
    // connections, so exactly one row wins and the other observes
    // Contended — this is the SQL invariant we're actually testing.
    let pool = fresh_pool_with_capacity(2).await;
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
    // `try_claim` is atomic across connections, so exactly one wins and
    // the other sees Contended — the SQL single-winner property the
    // contract relies on.
    assert_eq!(acquired_count, 1, "exactly one acquirer should win");
    assert_eq!(contended_count, 1, "the other must see Contended");
}

#[tokio::test]
async fn concurrent_reclaim_returns_each_stuck_row_to_exactly_one_sweeper() {
    // Per sub-spec §3.4: each stuck row must be reclaimed by exactly one
    // sweeper across concurrent sweeps. Two sweepers observing the same
    // expired row both as `RefreshInFlight` would double-count toward the
    // N=3 sentinel-event ReauthRequired threshold.
    //
    // Capacity ≥ 2 so the two `reclaim_stuck` calls genuinely race at the
    // SQL layer. With `max_connections=1` they queue behind one
    // connection and the SQL single-winner invariant
    // (locked existence-check + insert on the same row) is never actually
    // contested — the assertion would pass trivially via serialisation.
    let pool = fresh_pool_with_capacity(2).await;
    let repo1 = SqliteRefreshClaimRepo::new(pool.clone());
    let repo2 = SqliteRefreshClaimRepo::new(pool.clone());

    // Insert N expired rows, marking each as RefreshInFlight to mimic the
    // worst case (a presumed mid-IdP-call crash). The credential ids are
    // not retained: the assertion below only checks that the union of both
    // sweepers' results partitions the rows without overlap.
    for _ in 0..50 {
        let cid = CredentialId::new();
        let claim = match repo1
            .try_claim(&cid, &ReplicaId::new("setup"), Duration::from_millis(50))
            .await
            .unwrap()
        {
            ClaimAttempt::Acquired(c) => c,
            ClaimAttempt::Contended { .. } => panic!("setup must always acquire"),
            ClaimAttempt::OutcomeUnknown { .. } => panic!("fresh claim cannot be poisoned"),
        };
        repo1.mark_sentinel(&claim.token).await.unwrap();
    }
    // Wait past expiry so reclaim_stuck has work to do.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Two sweepers race.
    let (a, b) = tokio::join!(repo1.reclaim_stuck(), repo2.reclaim_stuck());
    let a = a.expect("sweeper a");
    let b = b.expect("sweeper b");

    // Each row should appear in exactly one sweeper's result.
    let outcome_id = |outcome: &ExpiredClaim| match outcome {
        ExpiredClaim::OutcomeUnknownAccounted { credential_id, .. } => *credential_id,
        ExpiredClaim::ReclaimedNormal { .. } => {
            panic!("setup marked every row in-flight")
        },
    };
    let a_ids: std::collections::HashSet<_> = a.iter().map(outcome_id).collect();
    let b_ids: std::collections::HashSet<_> = b.iter().map(outcome_id).collect();
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
async fn sentinel_event_count_in_window() {
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool.clone());

    let cid = CredentialId::new();
    let holder = ReplicaId::new("replica-A");

    // Empty window → 0.
    let window = Duration::from_secs(10);
    let count = repo
        .count_sentinel_events_in_window(&cid, window)
        .await
        .unwrap();
    assert_eq!(count, 0, "no events recorded yet");

    // Record three events in quick succession; all must fall inside
    // a 10s window.
    let now_ms = chrono::Utc::now().timestamp_millis();
    seed_sentinel_event(&pool, &cid, &holder, 1, now_ms).await;
    seed_sentinel_event(&pool, &cid, &holder, 2, now_ms).await;
    seed_sentinel_event(&pool, &cid, &holder, 3, now_ms).await;

    let count = repo
        .count_sentinel_events_in_window(&cid, window)
        .await
        .unwrap();
    assert_eq!(count, 3, "three events inside the rolling window");
}

#[tokio::test]
async fn sentinel_count_filters_by_credential_id() {
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool.clone());

    let cid_a = CredentialId::new();
    let cid_b = CredentialId::new();
    let holder = ReplicaId::new("replica-A");

    let now_ms = chrono::Utc::now().timestamp_millis();
    seed_sentinel_event(&pool, &cid_a, &holder, 1, now_ms).await;
    seed_sentinel_event(&pool, &cid_a, &holder, 2, now_ms).await;
    seed_sentinel_event(&pool, &cid_b, &holder, 1, now_ms).await;

    let window = Duration::from_secs(10);
    let count_a = repo
        .count_sentinel_events_in_window(&cid_a, window)
        .await
        .unwrap();
    let count_b = repo
        .count_sentinel_events_in_window(&cid_b, window)
        .await
        .unwrap();
    assert_eq!(count_a, 2, "credential A has 2 events");
    assert_eq!(count_b, 1, "credential B has 1 event");
}

#[tokio::test]
async fn sentinel_count_excludes_events_before_window_start() {
    let pool = fresh_pool().await;
    let repo = SqliteRefreshClaimRepo::new(pool.clone());

    let cid = CredentialId::new();
    let holder = ReplicaId::new("replica-A");

    // Record an event, then sleep past the window we're going to query.
    seed_sentinel_event(
        &pool,
        &cid,
        &holder,
        1,
        chrono::Utc::now().timestamp_millis(),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // A 50ms database-clock window starts after the recorded event.
    let window = Duration::from_millis(50);
    let count = repo
        .count_sentinel_events_in_window(&cid, window)
        .await
        .unwrap();
    assert_eq!(count, 0, "event predates window_start; must not be counted");

    // Record a fresh event and re-query; only the new one falls inside.
    seed_sentinel_event(
        &pool,
        &cid,
        &holder,
        2,
        chrono::Utc::now().timestamp_millis(),
    )
    .await;
    let count = repo
        .count_sentinel_events_in_window(&cid, window)
        .await
        .unwrap();
    assert_eq!(count, 1, "only the post-window event counts");
}
