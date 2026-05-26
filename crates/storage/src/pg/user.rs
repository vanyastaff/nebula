//! Postgres implementation of [`UserRepo`].
//!
//! Schema: migration `0001_users.sql` (the `users` table).
//!
//! # Lockout policy
//!
//! `record_login_failure` increments `failed_login_count` and, once the
//! threshold is reached, sets `locked_until = now + LOCKOUT_DURATION`.
//! `record_login_success` resets both columns. The thresholds mirror
//! the in-memory backend exactly so swapping backings does not change
//! user-visible behaviour:
//!
//! - [`LOCKOUT_THRESHOLD`] = 5 consecutive failures
//! - [`LOCKOUT_DURATION`] = 15 minutes
//!
//! Both updates are single `UPDATE` statements. PostgreSQL's default
//! `READ COMMITTED` isolation serializes concurrent same-row updates
//! via row-level locking: the second transaction re-reads the post-
//! commit row and re-applies the `failed_login_count + 1` increment.
//! The inline `CASE` evaluates against the post-increment value, so
//! once any racer crosses [`LOCKOUT_THRESHOLD`], `locked_until` is
//! set \u2014 every subsequent login attempt is fenced
//! by the caller checking `locked_until > NOW()`. No `SELECT ... FOR
//! UPDATE` is needed.

use std::time::Duration;

use sqlx::{Pool, Postgres};

use crate::{error::StorageError, pg::map_db_err, repos::UserRepo, rows::UserRow};

/// Failed-login threshold before [`PgUserRepo::record_login_failure`]
/// arms the lockout.
pub const LOCKOUT_THRESHOLD: i32 = 5;

/// Duration a user account stays locked once the threshold is hit.
pub const LOCKOUT_DURATION: Duration = Duration::from_mins(15);

/// Postgres-backed user repository.
#[derive(Clone)]
pub struct PgUserRepo {
    pool: Pool<Postgres>,
}

impl PgUserRepo {
    /// Construct from an existing pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }
}

// Column order must match every `SELECT ... FROM users` in this file.
type UserTuple = (
    Vec<u8>,                               // id
    String,                                // email
    Option<chrono::DateTime<chrono::Utc>>, // email_verified_at
    String,                                // display_name
    Option<String>,                        // avatar_url
    Option<String>,                        // password_hash
    chrono::DateTime<chrono::Utc>,         // created_at
    Option<chrono::DateTime<chrono::Utc>>, // last_login_at
    Option<chrono::DateTime<chrono::Utc>>, // locked_until
    i32,                                   // failed_login_count
    bool,                                  // mfa_enabled
    Option<Vec<u8>>,                       // mfa_secret
    i64,                                   // version
    Option<chrono::DateTime<chrono::Utc>>, // deleted_at
);

fn tuple_to_row(t: UserTuple) -> UserRow {
    UserRow {
        id: t.0,
        email: t.1,
        email_verified_at: t.2,
        display_name: t.3,
        avatar_url: t.4,
        password_hash: t.5,
        created_at: t.6,
        last_login_at: t.7,
        locked_until: t.8,
        failed_login_count: t.9,
        mfa_enabled: t.10,
        mfa_secret: t.11,
        version: t.12,
        deleted_at: t.13,
    }
}

const SELECT_COLS: &str = "id, email, email_verified_at, display_name, avatar_url, \
     password_hash, created_at, last_login_at, locked_until, failed_login_count, \
     mfa_enabled, mfa_secret, version, deleted_at";

impl UserRepo for PgUserRepo {
    #[tracing::instrument(level = "debug", skip(self, user), fields(user_id = %hex::encode(&user.id)))]
    async fn create(&self, user: &UserRow) -> Result<(), StorageError> {
        debug_assert!(!user.id.is_empty(), "user.id must not be empty");
        debug_assert!(!user.email.is_empty(), "user.email must not be empty");
        sqlx::query(
            "INSERT INTO users \
             (id, email, email_verified_at, display_name, avatar_url, password_hash, \
              created_at, last_login_at, locked_until, failed_login_count, mfa_enabled, \
              mfa_secret, version, deleted_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(&user.id)
        .bind(&user.email)
        .bind(user.email_verified_at)
        .bind(&user.display_name)
        .bind(user.avatar_url.as_deref())
        .bind(user.password_hash.as_deref())
        .bind(user.created_at)
        .bind(user.last_login_at)
        .bind(user.locked_until)
        .bind(user.failed_login_count)
        .bind(user.mfa_enabled)
        .bind(user.mfa_secret.as_deref())
        .bind(user.version)
        .bind(user.deleted_at)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("user", e))?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self), fields(user_id = %hex::encode(id)))]
    async fn get(&self, id: &[u8]) -> Result<Option<UserRow>, StorageError> {
        let sql = format!("SELECT {SELECT_COLS} FROM users WHERE id = $1 AND deleted_at IS NULL");
        let row = sqlx::query_as::<_, UserTuple>(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("user", e))?;
        Ok(row.map(tuple_to_row))
    }

    #[tracing::instrument(level = "debug", skip(self, email))]
    async fn get_by_email(&self, email: &str) -> Result<Option<UserRow>, StorageError> {
        let sql = format!(
            "SELECT {SELECT_COLS} FROM users \
             WHERE LOWER(email) = LOWER($1) AND deleted_at IS NULL"
        );
        let row = sqlx::query_as::<_, UserTuple>(&sql)
            .bind(email)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("user", e))?;
        Ok(row.map(tuple_to_row))
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, user),
        fields(user_id = %hex::encode(&user.id), expected_version)
    )]
    async fn update(&self, user: &UserRow, expected_version: i64) -> Result<(), StorageError> {
        let rows = sqlx::query(
            "UPDATE users SET \
                 email = $2, email_verified_at = $3, display_name = $4, avatar_url = $5, \
                 password_hash = $6, last_login_at = $7, locked_until = $8, \
                 failed_login_count = $9, mfa_enabled = $10, mfa_secret = $11, \
                 version = version + 1 \
             WHERE id = $1 AND version = $12 AND deleted_at IS NULL",
        )
        .bind(&user.id)
        .bind(&user.email)
        .bind(user.email_verified_at)
        .bind(&user.display_name)
        .bind(user.avatar_url.as_deref())
        .bind(user.password_hash.as_deref())
        .bind(user.last_login_at)
        .bind(user.locked_until)
        .bind(user.failed_login_count)
        .bind(user.mfa_enabled)
        .bind(user.mfa_secret.as_deref())
        .bind(expected_version)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("user", e))?
        .rows_affected();

        if rows == 0 {
            // Distinguish missing vs version mismatch.
            let actual: Option<i64> = sqlx::query_scalar(
                "SELECT version FROM users WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(&user.id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("user", e))?;
            return match actual {
                Some(v) => Err(StorageError::conflict(
                    "user",
                    hex::encode(&user.id),
                    expected_version,
                    v,
                )),
                None => Err(StorageError::not_found("user", hex::encode(&user.id))),
            };
        }
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self), fields(user_id = %hex::encode(id)))]
    async fn soft_delete(&self, id: &[u8]) -> Result<(), StorageError> {
        // No CAS guard here: the trait's `soft_delete` takes no
        // `expected_version` and the `deleted_at IS NULL` predicate
        // makes the operation idempotent under concurrent callers
        // (the second writer's UPDATE matches zero rows, returns Ok).
        // We still bump `version` so any post-delete observer that
        // re-fetches by id (and gets `None`) sees the world advance.
        sqlx::query(
            "UPDATE users SET deleted_at = NOW(), version = version + 1 \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("user", e))?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self), fields(user_id = %hex::encode(id)))]
    async fn record_login_success(&self, id: &[u8]) -> Result<(), StorageError> {
        // Clear failure tracking and refresh `last_login_at`. We do
        // NOT bump `version` here: this is an auth-time bookkeeping
        // update, not a domain mutation, and bumping would cause
        // spurious CAS conflicts for concurrent `update` callers.
        sqlx::query(
            "UPDATE users SET \
                 last_login_at = NOW(), failed_login_count = 0, locked_until = NULL \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("user", e))?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self), fields(user_id = %hex::encode(id)))]
    async fn record_login_failure(&self, id: &[u8]) -> Result<(), StorageError> {
        // Single statement: bump the counter and arm the lockout if
        // we crossed the threshold. The CASE expression keeps the
        // update atomic so a racing failure cannot observe a
        // counter-bumped row that is not yet locked.
        let lockout_secs = i64::try_from(LOCKOUT_DURATION.as_secs()).map_err(|_| {
            StorageError::Configuration(format!(
                "LOCKOUT_DURATION exceeds i64 seconds: {LOCKOUT_DURATION:?}"
            ))
        })?;
        sqlx::query(
            "UPDATE users SET \
                 failed_login_count = failed_login_count + 1, \
                 locked_until = CASE \
                     WHEN failed_login_count + 1 >= $2 \
                         THEN NOW() + make_interval(secs => $3) \
                     ELSE locked_until \
                 END \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .bind(LOCKOUT_THRESHOLD)
        .bind(lockout_secs as f64)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("user", e))?;
        Ok(())
    }
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use chrono::{Duration as ChronoDuration, Utc};
    use sqlx::postgres::PgPoolOptions;

    use super::*;
    use crate::test_support::{random_id, test_user};

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

    fn fresh_user(prefix: &str) -> UserRow {
        let suffix = hex::encode(&random_id()[..4]);
        test_user(&format!("{prefix}-{suffix}@example.test"))
    }

    #[tokio::test]
    async fn create_get_roundtrip() {
        let Some(pool) = pool().await else { return };
        let repo = PgUserRepo::new(pool);
        let user = fresh_user("roundtrip");

        repo.create(&user).await.expect("create");
        let loaded = repo.get(&user.id).await.expect("get").expect("some");
        assert_eq!(loaded.id, user.id);
        assert_eq!(loaded.email, user.email);
        assert_eq!(loaded.display_name, user.display_name);
        assert_eq!(loaded.failed_login_count, 0);
        assert!(!loaded.mfa_enabled);
    }

    #[tokio::test]
    async fn get_by_email_is_case_insensitive() {
        let Some(pool) = pool().await else { return };
        let repo = PgUserRepo::new(pool);
        let mut user = fresh_user("CaseInsensitive");
        // Store mixed-case in the table; lookup with upper-case.
        user.email = user.email.to_lowercase();
        repo.create(&user).await.expect("create");

        let loaded = repo
            .get_by_email(&user.email.to_uppercase())
            .await
            .expect("get_by_email")
            .expect("some");
        assert_eq!(loaded.id, user.id);
    }

    #[tokio::test]
    async fn duplicate_email_among_active_users_is_rejected() {
        let Some(pool) = pool().await else { return };
        let repo = PgUserRepo::new(pool);
        let user_a = fresh_user("dup");
        let mut user_b = fresh_user("dup");
        user_b.email = user_a.email.clone();

        repo.create(&user_a).await.expect("first create");
        let err = repo
            .create(&user_b)
            .await
            .expect_err("duplicate email must reject");
        assert!(
            matches!(err, StorageError::Duplicate { entity: "user", .. }),
            "expected Duplicate {{ entity: 'user', .. }}, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn record_login_failure_increments_then_locks() {
        let Some(pool) = pool().await else { return };
        let repo = PgUserRepo::new(pool);
        let user = fresh_user("lockout");
        repo.create(&user).await.expect("create");

        for expected in 1..LOCKOUT_THRESHOLD {
            repo.record_login_failure(&user.id).await.expect("failure");
            let loaded = repo.get(&user.id).await.expect("get").expect("some");
            assert_eq!(loaded.failed_login_count, expected);
            assert!(
                loaded.locked_until.is_none(),
                "below threshold ({expected}/{LOCKOUT_THRESHOLD}) must not lock"
            );
        }

        // Threshold-th failure arms the lockout.
        let before = Utc::now();
        repo.record_login_failure(&user.id)
            .await
            .expect("threshold failure");
        let loaded = repo.get(&user.id).await.expect("get").expect("some");
        assert_eq!(loaded.failed_login_count, LOCKOUT_THRESHOLD);
        let locked_until = loaded.locked_until.expect("locked_until armed");
        let lockout_chrono = ChronoDuration::from_std(LOCKOUT_DURATION).expect("chrono duration");
        assert!(
            locked_until >= before + lockout_chrono - ChronoDuration::seconds(5)
                && locked_until <= Utc::now() + lockout_chrono + ChronoDuration::seconds(5),
            "locked_until ({locked_until}) outside expected lockout window"
        );
    }

    #[tokio::test]
    async fn record_login_success_clears_failures_and_lock() {
        let Some(pool) = pool().await else { return };
        let repo = PgUserRepo::new(pool);
        let user = fresh_user("success");
        repo.create(&user).await.expect("create");

        // Force lockout.
        for _ in 0..LOCKOUT_THRESHOLD {
            repo.record_login_failure(&user.id).await.expect("failure");
        }
        let mid = repo.get(&user.id).await.expect("get").expect("some");
        assert!(mid.locked_until.is_some());

        repo.record_login_success(&user.id).await.expect("success");
        let after = repo.get(&user.id).await.expect("get").expect("some");
        assert_eq!(after.failed_login_count, 0);
        assert!(after.locked_until.is_none());
        assert!(after.last_login_at.is_some());
    }
}
