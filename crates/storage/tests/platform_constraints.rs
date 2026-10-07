//! Platform persistence invariants at the admitted deployment-schema seam.

#![cfg(all(feature = "sqlite", feature = "postgres"))]

mod support {
    pub(crate) mod postgres_schema;
}

use support::postgres_schema;

async fn postgres() -> sqlx::PgPool {
    let url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL is required for PostgreSQL evidence");
    let pool = postgres_schema::connect_with_private_schema(&url, "nebula_platform")
        .await
        .expect("open isolated PostgreSQL schema");
    nebula_storage::postgres::init_schema(&pool)
        .await
        .expect("admit PostgreSQL schema");
    pool
}

#[tokio::test]
async fn replay_rows_require_http_status_and_sha256_fingerprint_on_both_backends() {
    let sqlite = nebula_storage::sqlite::open_memory_deployment()
        .await
        .expect("open SQLite deployment");
    let postgres = postgres().await;
    for (status, fingerprint_len, admitted) in [
        (99, 32, false),
        (1000, 32, false),
        (200, 31, false),
        (200, 32, true),
        (999, 32, true),
    ] {
        let key = format!("status-{status}-fingerprint-{fingerprint_len}");
        let fingerprint = vec![0_u8; fingerprint_len];
        let sqlite_result = sqlx::query(
            "INSERT INTO api_idempotency_dedup \
             (cache_key, status, headers, body, fingerprint, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&key)
        .bind(status)
        .bind(Vec::<u8>::new())
        .bind(Vec::<u8>::new())
        .bind(&fingerprint)
        .bind(1_i64)
        .execute(&sqlite)
        .await;
        assert_eq!(
            sqlite_result.is_ok(),
            admitted,
            "SQLite rejects malformed replay rows"
        );
        let postgres_result = sqlx::query(
            "INSERT INTO api_idempotency_dedup \
             (cache_key, status, headers, body, fingerprint, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&key)
        .bind(i16::try_from(status).expect("fixture status fits SMALLINT"))
        .bind(Vec::<u8>::new())
        .bind(Vec::<u8>::new())
        .bind(&fingerprint)
        .bind(chrono::Utc::now())
        .execute(&postgres)
        .await;
        assert_eq!(
            postgres_result.is_ok(),
            admitted,
            "PostgreSQL rejects malformed replay rows"
        );
    }
    let null_identity = sqlx::query(
        "INSERT INTO api_idempotency_dedup \
         (cache_key, status, headers, body, fingerprint, expires_at) \
         VALUES (NULL, 200, X'', X'', zeroblob(32), 1)",
    )
    .execute(&sqlite)
    .await;
    assert!(
        null_identity.is_err(),
        "SQLite replay identity cannot be NULL"
    );
    postgres.close().await;
    sqlite.close().await;
}

#[tokio::test]
async fn rate_reservations_belong_to_limit_keys_and_sequence_retains_wrapped_bits() {
    let pool = postgres().await;
    let orphan = sqlx::query(
        "INSERT INTO rate_limit_reservations \
         (limit_key, reservation_id, permits, allow_at_ns, end_tat_ns, seq) \
         VALUES ('missing', '00000000000000000000000000000000', 1, 1, 2, -1)",
    )
    .execute(&pool)
    .await;
    assert!(orphan.is_err(), "orphan reservation is rejected");
    sqlx::query(
        "INSERT INTO rate_limits (limit_key, tat_ns, seq, emission_ns, burst) \
         VALUES ('key', 2, -1, 1, 1)",
    )
    .execute(&pool)
    .await
    .expect("wrapped sequence remains valid");
    sqlx::query(
        "INSERT INTO rate_limit_reservations \
         (limit_key, reservation_id, permits, allow_at_ns, end_tat_ns, seq) \
         VALUES ('key', '00000000000000000000000000000000', 1, 1, 2, -1)",
    )
    .execute(&pool)
    .await
    .expect("reservation accepts wrapped sequence");
    sqlx::query("DELETE FROM rate_limits WHERE limit_key = 'key'")
        .execute(&pool)
        .await
        .expect("purge limit key");
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM rate_limit_reservations")
        .fetch_one(&pool)
        .await
        .expect("read reservations");
    assert_eq!(remaining, 0, "purging limit key cascades reservations");
    pool.close().await;
}
