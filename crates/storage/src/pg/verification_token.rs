//! Postgres implementation of [`VerificationTokenRepo`].
//!
//! Schema: migration `0002_user_auth.sql` (`verification_tokens`
//! table). The table is shared across all token kinds
//! (`email_verification`, `password_reset`, `org_invite`,
//! `mfa_recovery`, `mfa_challenge`) and keyed by `token_hash`
//! (SHA-256 of the plaintext value).
//!
//! Atomicity:
//!
//! - [`consume_by_hash`](VerificationTokenRepo::consume_by_hash) uses
//!   `UPDATE ... WHERE consumed_at IS NULL AND expires_at > NOW()
//!   RETURNING ...` so the row can only be consumed once even under
//!   concurrent callers; the loser sees `None`.
//! - [`consume_by_hash_and_kind`](VerificationTokenRepo::consume_by_hash_and_kind)
//!   adds `AND kind = $2` to the same WHERE clause so a token of the
//!   wrong kind presented to the wrong route does not match — the row
//!   is left unconsumed and the caller sees `None` instead of an
//!   incorrectly destroyed token.

use sqlx::{Pool, Postgres, types::Json};

use crate::{
    error::StorageError, pg::map_db_err, repos::VerificationTokenRepo, rows::VerificationTokenRow,
};

/// Postgres-backed verification-token repository.
#[derive(Clone)]
pub struct PgVerificationTokenRepo {
    pool: Pool<Postgres>,
}

impl PgVerificationTokenRepo {
    /// Construct from an existing pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }
}

// Column order must match every `SELECT ... FROM verification_tokens`
// and every `RETURNING ...` in this file.
type TokenTuple = (
    Vec<u8>,                               // token_hash
    Vec<u8>,                               // user_id
    String,                                // kind
    Option<Json<serde_json::Value>>,       // payload
    chrono::DateTime<chrono::Utc>,         // created_at
    chrono::DateTime<chrono::Utc>,         // expires_at
    Option<chrono::DateTime<chrono::Utc>>, // consumed_at
);

fn tuple_to_row(t: TokenTuple) -> VerificationTokenRow {
    VerificationTokenRow {
        token_hash: t.0,
        user_id: t.1,
        kind: t.2,
        payload: t.3.map(|j| j.0),
        created_at: t.4,
        expires_at: t.5,
        consumed_at: t.6,
    }
}

const SELECT_COLS: &str = "token_hash, user_id, kind, payload, created_at, expires_at, consumed_at";

impl VerificationTokenRepo for PgVerificationTokenRepo {
    #[tracing::instrument(
        level = "debug",
        skip(self, token),
        fields(kind = %token.kind, user_id = %hex::encode(&token.user_id))
    )]
    async fn create(&self, token: &VerificationTokenRow) -> Result<(), StorageError> {
        debug_assert!(!token.token_hash.is_empty(), "token_hash must not be empty");
        debug_assert!(!token.user_id.is_empty(), "user_id must not be empty");
        debug_assert!(!token.kind.is_empty(), "kind must not be empty");
        sqlx::query(
            "INSERT INTO verification_tokens \
             (token_hash, user_id, kind, payload, created_at, expires_at, consumed_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&token.token_hash)
        .bind(&token.user_id)
        .bind(&token.kind)
        .bind(token.payload.as_ref().map(Json))
        .bind(token.created_at)
        .bind(token.expires_at)
        .bind(token.consumed_at)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("verification_token", e))?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, token_hash))]
    async fn consume_by_hash(
        &self,
        token_hash: &[u8],
    ) -> Result<Option<VerificationTokenRow>, StorageError> {
        debug_assert!(!token_hash.is_empty(), "token_hash must not be empty");
        // Single-statement atomicity: only an unconsumed, unexpired
        // row matches the WHERE clause, so two concurrent consumers
        // cannot both win.
        let sql = format!(
            "UPDATE verification_tokens SET consumed_at = NOW() \
             WHERE token_hash = $1 AND consumed_at IS NULL AND expires_at > NOW() \
             RETURNING {SELECT_COLS}"
        );
        let row = sqlx::query_as::<_, TokenTuple>(&sql)
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("verification_token", e))?;
        Ok(row.map(tuple_to_row))
    }

    #[tracing::instrument(level = "debug", skip(self, token_hash), fields(kind))]
    async fn consume_by_hash_and_kind(
        &self,
        token_hash: &[u8],
        kind: &str,
    ) -> Result<Option<VerificationTokenRow>, StorageError> {
        debug_assert!(!token_hash.is_empty(), "token_hash must not be empty");
        debug_assert!(!kind.is_empty(), "kind must not be empty");
        // Single-statement atomicity: the `AND kind = $2` filter inside
        // the same UPDATE means a token of the wrong kind sent to the
        // wrong route does not match and is NOT consumed. The valid
        // follow-up at the right route can still succeed.
        let sql = format!(
            "UPDATE verification_tokens SET consumed_at = NOW() \
             WHERE token_hash = $1 AND kind = $2 \
               AND consumed_at IS NULL AND expires_at > NOW() \
             RETURNING {SELECT_COLS}"
        );
        let row = sqlx::query_as::<_, TokenTuple>(&sql)
            .bind(token_hash)
            .bind(kind)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("verification_token", e))?;
        Ok(row.map(tuple_to_row))
    }

    #[tracing::instrument(level = "debug", skip(self, token_hash))]
    async fn get_by_hash(
        &self,
        token_hash: &[u8],
    ) -> Result<Option<VerificationTokenRow>, StorageError> {
        debug_assert!(!token_hash.is_empty(), "token_hash must not be empty");
        let sql = format!("SELECT {SELECT_COLS} FROM verification_tokens WHERE token_hash = $1");
        let row = sqlx::query_as::<_, TokenTuple>(&sql)
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("verification_token", e))?;
        Ok(row.map(tuple_to_row))
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn cleanup_expired(&self) -> Result<u64, StorageError> {
        let result = sqlx::query("DELETE FROM verification_tokens WHERE expires_at <= NOW()")
            .execute(&self.pool)
            .await
            .map_err(|e| map_db_err("verification_token", e))?;
        Ok(result.rows_affected())
    }

    #[tracing::instrument(
        level = "debug",
        skip(self),
        fields(user_id = %hex::encode(user_id), kind)
    )]
    async fn revoke_all_for_user(&self, user_id: &[u8], kind: &str) -> Result<u64, StorageError> {
        debug_assert!(!user_id.is_empty(), "user_id must not be empty");
        debug_assert!(!kind.is_empty(), "kind must not be empty");
        let result = sqlx::query(
            "UPDATE verification_tokens SET consumed_at = NOW() \
             WHERE user_id = $1 AND kind = $2 AND consumed_at IS NULL",
        )
        .bind(user_id)
        .bind(kind)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("verification_token", e))?;
        Ok(result.rows_affected())
    }
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use chrono::{Duration, Utc};
    use serde_json::json;
    use sqlx::postgres::PgPoolOptions;

    use super::*;
    use crate::{
        pg::user::PgUserRepo,
        repos::UserRepo,
        test_support::{random_id, test_user},
    };

    static SPEC16_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/postgres");
    static SCHEMA_READY: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

    async fn pool() -> Option<Pool<Postgres>> {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(std::env::VarError::NotPresent) => return None,
            Err(err) => panic!("DATABASE_URL is set but invalid: {err}"),
        };
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .expect("connect");
        SCHEMA_READY
            .get_or_init(|| async {
                SPEC16_MIGRATOR
                    .run(&pool)
                    .await
                    .expect("spec-16 postgres migrations");
            })
            .await;
        Some(pool)
    }

    async fn seed_user(pool: &Pool<Postgres>, prefix: &str) -> Vec<u8> {
        let users = PgUserRepo::new(pool.clone());
        let suffix = hex::encode(&random_id()[..4]);
        let user = test_user(&format!("{prefix}-{suffix}@example.test"));
        users.create(&user).await.expect("seed user");
        user.id
    }

    fn fresh_token(user_id: &[u8], kind: &str) -> VerificationTokenRow {
        let hash = {
            let mut h = [0u8; 32];
            let src = random_id();
            h[..src.len().min(32)].copy_from_slice(&src[..src.len().min(32)]);
            h.to_vec()
        };
        VerificationTokenRow {
            token_hash: hash,
            user_id: user_id.to_vec(),
            kind: kind.to_string(),
            payload: Some(json!({"reason": "test"})),
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::hours(1),
            consumed_at: None,
        }
    }

    #[tokio::test]
    async fn create_get_roundtrip() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "vt-roundtrip").await;
        let repo = PgVerificationTokenRepo::new(pool);
        let token = fresh_token(&user_id, "email_verification");
        let hash = token.token_hash.clone();

        repo.create(&token).await.expect("create");
        let loaded = repo
            .get_by_hash(&hash)
            .await
            .expect("get_by_hash")
            .expect("some");
        assert_eq!(loaded.user_id, user_id);
        assert_eq!(loaded.kind, "email_verification");
        assert_eq!(loaded.payload, Some(json!({"reason": "test"})));
        assert!(loaded.consumed_at.is_none());
    }

    #[tokio::test]
    async fn duplicate_hash_is_rejected() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "vt-dup").await;
        let repo = PgVerificationTokenRepo::new(pool);
        let token = fresh_token(&user_id, "password_reset");
        repo.create(&token).await.expect("first create");

        let err = repo
            .create(&token)
            .await
            .expect_err("duplicate token_hash must reject");
        assert!(
            matches!(
                err,
                StorageError::Duplicate {
                    entity: "verification_token",
                    ..
                }
            ),
            "expected Duplicate {{ entity: 'verification_token', .. }}, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn consume_by_hash_is_single_shot() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "vt-consume").await;
        let repo = PgVerificationTokenRepo::new(pool);
        let token = fresh_token(&user_id, "password_reset");
        let hash = token.token_hash.clone();
        repo.create(&token).await.expect("create");

        let first = repo
            .consume_by_hash(&hash)
            .await
            .expect("consume")
            .expect("some");
        assert_eq!(first.user_id, user_id);

        let second = repo.consume_by_hash(&hash).await.expect("consume second");
        assert!(second.is_none(), "second consume must return None");

        // The persisted row still has `consumed_at` set.
        let stored = repo
            .get_by_hash(&hash)
            .await
            .expect("get")
            .expect("still present");
        assert!(stored.consumed_at.is_some());
    }

    #[tokio::test]
    async fn consume_by_hash_and_kind_matching_kind_consumes() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "vt-consume-kind").await;
        let repo = PgVerificationTokenRepo::new(pool);
        let token = fresh_token(&user_id, "mfa_challenge");
        let hash = token.token_hash.clone();
        repo.create(&token).await.expect("create");

        let first = repo
            .consume_by_hash_and_kind(&hash, "mfa_challenge")
            .await
            .expect("consume")
            .expect("some");
        assert_eq!(first.user_id, user_id);
        assert_eq!(first.kind, "mfa_challenge");

        // Replay against the same kind returns None.
        let replay = repo
            .consume_by_hash_and_kind(&hash, "mfa_challenge")
            .await
            .expect("second consume");
        assert!(replay.is_none(), "second consume must return None");
    }

    #[tokio::test]
    async fn consume_by_hash_and_kind_mismatched_kind_does_not_consume() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "vt-consume-mismatch").await;
        let repo = PgVerificationTokenRepo::new(pool);
        // Persisted as password_reset, but the caller asks for mfa_challenge.
        let token = fresh_token(&user_id, "password_reset");
        let hash = token.token_hash.clone();
        repo.create(&token).await.expect("create");

        let wrong_kind = repo
            .consume_by_hash_and_kind(&hash, "mfa_challenge")
            .await
            .expect("consume");
        assert!(
            wrong_kind.is_none(),
            "mismatched kind must NOT consume the row"
        );

        // The valid follow-up at the correct kind still succeeds —
        // the row was not burned by the mismatched attempt.
        let correct = repo
            .consume_by_hash_and_kind(&hash, "password_reset")
            .await
            .expect("consume correct")
            .expect("row still consumable");
        assert_eq!(correct.kind, "password_reset");
    }

    #[tokio::test]
    async fn revoke_all_for_user_only_touches_unconsumed_of_kind() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "vt-revoke").await;
        let repo = PgVerificationTokenRepo::new(pool);
        let reset_a = fresh_token(&user_id, "password_reset");
        let reset_b = fresh_token(&user_id, "password_reset");
        let other = fresh_token(&user_id, "email_verification");
        repo.create(&reset_a).await.expect("create a");
        repo.create(&reset_b).await.expect("create b");
        repo.create(&other).await.expect("create other");

        let revoked = repo
            .revoke_all_for_user(&user_id, "password_reset")
            .await
            .expect("revoke");
        assert_eq!(revoked, 2);

        // The other-kind token remains consumable.
        let still = repo
            .consume_by_hash(&other.token_hash)
            .await
            .expect("consume other")
            .expect("other still consumable");
        assert_eq!(still.kind, "email_verification");
    }

    #[tokio::test]
    async fn cleanup_expired_deletes_only_past_rows() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "vt-cleanup").await;
        let repo = PgVerificationTokenRepo::new(pool);

        let mut past = fresh_token(&user_id, "password_reset");
        past.expires_at = Utc::now() - Duration::hours(1);
        let live = fresh_token(&user_id, "email_verification");

        repo.create(&past).await.expect("create past");
        repo.create(&live).await.expect("create live");

        let deleted = repo.cleanup_expired().await.expect("cleanup");
        assert_eq!(deleted, 1, "only the past row should be deleted");

        // Live row survives and is still peekable.
        let still = repo
            .get_by_hash(&live.token_hash)
            .await
            .expect("get live")
            .expect("live still present");
        assert_eq!(still.kind, "email_verification");

        // Past row is gone (DELETE, not soft-consume).
        let gone = repo.get_by_hash(&past.token_hash).await.expect("get past");
        assert!(gone.is_none(), "expired row must be deleted");
    }
}
