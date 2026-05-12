//! Postgres integration tests for [`PgIdempotencyStore`] (M3.4 / ADR-0048).
//!
//! Mirrors the existing storage test convention: skip silently when
//! `DATABASE_URL` is absent, fail loudly when it is set but unparsable.
//! Each test scopes its rows to a randomly-generated cache key so
//! parallel runs do not collide on the `cache_key` PRIMARY KEY.
//!
//! Run locally via:
//!   DATABASE_URL=postgres://... cargo nextest run \
//!     -p nebula-storage --features postgres \
//!     --test pg_idempotency

#![cfg(feature = "postgres")]

use std::time::Duration;

use nebula_storage::{
    pg::PgIdempotencyStore,
    repos::{CachedRecord, IdempotencyStoreRepo},
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::OnceCell;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");
static SCHEMA_READY: OnceCell<()> = OnceCell::const_new();

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
        .expect("connect to DATABASE_URL");
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

fn random_cache_key(prefix: &str) -> String {
    // Avoid pulling `rand` as a dev-dep just for this — `nanos`-since-epoch
    // is monotonically increasing and unique enough for parallel test
    // runs scoped to a single PG instance.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{nanos:032x}")
}

fn record(body: &[u8], fingerprint: u8) -> CachedRecord {
    CachedRecord {
        status: 200,
        headers: vec![
            ("content-type".into(), b"application/json".to_vec()),
            ("x-test".into(), b"yes".to_vec()),
        ],
        body: body.to_vec(),
        fingerprint: [fingerprint; 32],
    }
}

#[tokio::test]
async fn round_trip_put_get_returns_equal_record() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let repo = PgIdempotencyStore::new(pool);
    let key = random_cache_key("rt");

    repo.put(key.clone(), record(b"hello", 0xab), Duration::from_mins(1))
        .await
        .expect("put");

    let got = repo
        .get(&key)
        .await
        .expect("get must not error")
        .expect("must round-trip");
    assert_eq!(got.status, 200);
    assert_eq!(got.body, b"hello".to_vec());
    assert_eq!(got.fingerprint, [0xab; 32]);
    assert!(
        got.headers
            .iter()
            .any(|(n, v)| n == "content-type" && v == b"application/json"),
        "headers must round-trip"
    );
}

#[tokio::test]
async fn concurrent_first_writer_wins() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let repo = PgIdempotencyStore::new(pool);
    let key = random_cache_key("cfww");

    // Both writers race on the same key; first to commit wins.
    let r1 = record(b"first", 0x01);
    let r2 = record(b"second", 0x02);
    let key_a = key.clone();
    let key_b = key.clone();
    let repo_a = repo.clone();
    let repo_b = repo.clone();

    let (a, b) = tokio::join!(
        async move { repo_a.put(key_a, r1, Duration::from_mins(1)).await },
        async move { repo_b.put(key_b, r2, Duration::from_mins(1)).await },
    );
    a.expect("first put must succeed");
    b.expect("second put must be a no-op (ON CONFLICT DO NOTHING), not an error");

    let got = repo
        .get(&key)
        .await
        .expect("get")
        .expect("must have one record");
    assert!(
        got.body == b"first".to_vec() || got.body == b"second".to_vec(),
        "exactly one writer's record must remain — got body: {:?}",
        std::str::from_utf8(&got.body)
    );
}

#[tokio::test]
async fn body_mismatch_race_keeps_first_record() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let repo = PgIdempotencyStore::new(pool);
    let key = random_cache_key("bmr");

    repo.put(key.clone(), record(b"alpha", 0xa1), Duration::from_mins(1))
        .await
        .expect("first put");

    // Different fingerprint, same key — must be a no-op.
    repo.put(key.clone(), record(b"beta", 0xb2), Duration::from_mins(1))
        .await
        .expect("second put must be a no-op, not an error");

    let got = repo
        .get(&key)
        .await
        .expect("get")
        .expect("must have first record");
    assert_eq!(got.body, b"alpha".to_vec());
    assert_eq!(got.fingerprint, [0xa1; 32]);
}

#[tokio::test]
async fn ttl_expiry_drops_row_after_evict_expired() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set — skipping");
        return;
    };
    let repo = PgIdempotencyStore::new(pool);
    let key = random_cache_key("ttl");

    // Tiny TTL so we don't have to wait long.
    repo.put(
        key.clone(),
        record(b"transient", 0x33),
        Duration::from_millis(50),
    )
    .await
    .expect("put");

    // Read before expiry: present.
    let got = repo.get(&key).await.expect("get");
    assert!(got.is_some(), "row must be present before TTL");

    tokio::time::sleep(Duration::from_millis(150)).await;

    // After the deadline `get` filters by `expires_at > NOW()` — read is None.
    let got = repo.get(&key).await.expect("get");
    assert!(got.is_none(), "expired row must read as None");

    // Sweep should reclaim at least our row.
    let evicted = repo.evict_expired().await.expect("sweep");
    assert!(
        evicted >= 1,
        "evict_expired must drop at least one row (got {evicted})"
    );
}
