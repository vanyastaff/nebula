//! Postgres implementation of [`OAuthStateRepo`].
//!
//! Schema: migration `0028_plane_a_oauth_state.sql`
//! (`plane_a_oauth_states` table). Holds Plane-A sign-in-with-OAuth
//! PKCE state \u2014 distinct from the Plane-B credential OAuth pending
//! surface (`pending_credentials`).
//!
//! Atomicity:
//!
//! - [`consume_by_state`](OAuthStateRepo::consume_by_state) uses
//!   `UPDATE ... WHERE consumed_at IS NULL AND expires_at > NOW()
//!   RETURNING ...` so a replay arriving after the first callback is
//!   the loser of the UPDATE and sees `None`. This is the replay
//!   defence for the PKCE callback.
//! - [`consume_by_state_and_provider`](OAuthStateRepo::consume_by_state_and_provider)
//!   adds `AND provider = $2` to the same WHERE clause so a state
//!   value crossed between providers (`google` vs `github`) does not
//!   match — the row is left unconsumed and the wrong-provider caller
//!   sees `None` instead of burning a valid state on the wrong route.
//! - [`admit`](OAuthStateRepo::admit) uses a fixed, deployment-wide
//!   transaction advisory try-lock. Cleanup, the bounded active-row count,
//!   and insertion are serialized without waiting on another replica.

use sqlx::{Pool, Postgres};

use crate::{
    error::StorageError,
    pg::map_db_err,
    repos::{OAUTH_STATE_CAPACITY, OAuthStateAdmission, OAuthStateRepo},
    rows::OAuthStateRow,
};

/// Database-wide namespace/resource pair for Plane-A OAuth-state admission.
///
/// PostgreSQL's two-`int4` advisory-lock key space is shared by every
/// connection to one database. These fixed ASCII-derived values (`NBLA`,
/// `OAUT`) therefore serialize all Nebula replicas while remaining separate
/// from unrelated advisory-lock users. The try-lock form is load-bearing:
/// admission never waits while occupying a pool connection.
const OAUTH_STATE_ADMISSION_LOCK_KEY: (i32, i32) = (0x4E42_4C41, 0x4F41_5554);

const INVALID_ADMISSION_MESSAGE: &str = "invalid OAuth state admission";

/// Postgres-backed Plane-A OAuth PKCE state repository.
#[derive(Clone)]
pub struct PgOAuthStateRepo {
    pool: Pool<Postgres>,
    capacity: u32,
}

impl PgOAuthStateRepo {
    /// Construct from an existing pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self {
            pool,
            capacity: OAUTH_STATE_CAPACITY,
        }
    }

    #[cfg(test)]
    fn with_capacity(pool: Pool<Postgres>, capacity: std::num::NonZeroU32) -> Self {
        Self {
            pool,
            capacity: capacity.get(),
        }
    }

    fn validate_admission(state: &OAuthStateRow) -> Result<(), StorageError> {
        if state.state.is_empty()
            || state.provider.is_empty()
            || state.code_verifier.is_empty()
            || state.consumed_at.is_some()
        {
            return Err(StorageError::Internal(INVALID_ADMISSION_MESSAGE.to_owned()));
        }
        Ok(())
    }
}

async fn rollback_after_failure(
    transaction: sqlx::Transaction<'_, Postgres>,
    primary_error: StorageError,
) -> StorageError {
    match transaction.rollback().await {
        Ok(()) => primary_error,
        Err(rollback_error) => map_db_err("plane_a_oauth_state_admission_rollback", rollback_error),
    }
}

// Column order must match every `SELECT ... FROM plane_a_oauth_states`
// and every `RETURNING ...` in this file.
type StateTuple = (
    String,                                // state
    String,                                // provider
    String,                                // code_verifier
    Option<String>,                        // redirect_uri
    chrono::DateTime<chrono::Utc>,         // created_at
    chrono::DateTime<chrono::Utc>,         // expires_at
    Option<chrono::DateTime<chrono::Utc>>, // consumed_at
);

fn tuple_to_row(t: StateTuple) -> OAuthStateRow {
    OAuthStateRow {
        state: t.0,
        provider: t.1,
        code_verifier: t.2,
        redirect_uri: t.3,
        created_at: t.4,
        expires_at: t.5,
        consumed_at: t.6,
    }
}

const SELECT_COLS: &str =
    "state, provider, code_verifier, redirect_uri, created_at, expires_at, consumed_at";

impl OAuthStateRepo for PgOAuthStateRepo {
    #[tracing::instrument(
        level = "debug",
        skip(self, state),
        fields(provider = %state.provider)
    )]
    async fn admit(&self, state: &OAuthStateRow) -> Result<OAuthStateAdmission, StorageError> {
        Self::validate_admission(state)?;

        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| map_db_err("plane_a_oauth_state_admission", error))?;
        let acquired =
            match sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_xact_lock($1, $2)")
                .bind(OAUTH_STATE_ADMISSION_LOCK_KEY.0)
                .bind(OAUTH_STATE_ADMISSION_LOCK_KEY.1)
                .fetch_one(&mut *transaction)
                .await
            {
                Ok(acquired) => acquired,
                Err(error) => {
                    let error = map_db_err("plane_a_oauth_state_admission", error);
                    return Err(rollback_after_failure(transaction, error).await);
                },
            };
        if !acquired {
            transaction
                .rollback()
                .await
                .map_err(|error| map_db_err("plane_a_oauth_state_admission_rollback", error))?;
            return Ok(OAuthStateAdmission::Contended);
        }

        let admission = async {
            sqlx::query(
                "DELETE FROM plane_a_oauth_states \
                 WHERE expires_at <= statement_timestamp()",
            )
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_db_err("plane_a_oauth_state", error))?;

            let capacity = i64::from(self.capacity);
            let inserted = sqlx::query(
                "INSERT INTO plane_a_oauth_states \
                 (state, provider, code_verifier, redirect_uri, created_at, expires_at, consumed_at) \
                 SELECT $1, $2, $3, $4, $5, $6, NULL \
                 WHERE ( \
                     SELECT COUNT(*) FROM ( \
                         SELECT 1 FROM plane_a_oauth_states \
                         WHERE consumed_at IS NULL \
                           AND expires_at > statement_timestamp() \
                         LIMIT $7 \
                     ) AS active_states \
                 ) < $7",
            )
            .bind(&state.state)
            .bind(&state.provider)
            .bind(&state.code_verifier)
            .bind(state.redirect_uri.as_deref())
            .bind(state.created_at)
            .bind(state.expires_at)
            .bind(capacity)
            .execute(&mut *transaction)
            .await
            .map_err(|error| map_db_err("plane_a_oauth_state", error))?;

            Ok(if inserted.rows_affected() == 1 {
                OAuthStateAdmission::Created
            } else {
                OAuthStateAdmission::AtCapacity
            })
        }
        .await;
        let admission = match admission {
            Ok(admission) => admission,
            Err(error) => return Err(rollback_after_failure(transaction, error).await),
        };

        transaction
            .commit()
            .await
            .map_err(|error| map_db_err("plane_a_oauth_state_admission_commit", error))?;
        Ok(admission)
    }

    #[tracing::instrument(level = "debug", skip(self, state))]
    async fn consume_by_state(&self, state: &str) -> Result<Option<OAuthStateRow>, StorageError> {
        debug_assert!(!state.is_empty(), "state value must not be empty");
        // Single-statement atomicity: the UPDATE only matches an
        // unconsumed, unexpired row; the replay/race loser sees None.
        let sql = format!(
            "UPDATE plane_a_oauth_states SET consumed_at = NOW() \
             WHERE state = $1 AND consumed_at IS NULL AND expires_at > NOW() \
             RETURNING {SELECT_COLS}"
        );
        let row = sqlx::query_as::<_, StateTuple>(sqlx::AssertSqlSafe(sql))
            .bind(state)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("plane_a_oauth_state", e))?;
        Ok(row.map(tuple_to_row))
    }

    #[tracing::instrument(level = "debug", skip(self, state), fields(provider))]
    async fn consume_by_state_and_provider(
        &self,
        state: &str,
        provider: &str,
    ) -> Result<Option<OAuthStateRow>, StorageError> {
        debug_assert!(!state.is_empty(), "state value must not be empty");
        debug_assert!(!provider.is_empty(), "provider must not be empty");
        // Single-statement atomicity: the `AND provider = $2` filter
        // inside the UPDATE means a state value crossed between
        // providers does not match and is NOT consumed. The valid
        // callback at the correct provider can still succeed.
        let sql = format!(
            "UPDATE plane_a_oauth_states SET consumed_at = NOW() \
             WHERE state = $1 AND provider = $2 \
               AND consumed_at IS NULL AND expires_at > NOW() \
             RETURNING {SELECT_COLS}"
        );
        let row = sqlx::query_as::<_, StateTuple>(sqlx::AssertSqlSafe(sql))
            .bind(state)
            .bind(provider)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("plane_a_oauth_state", e))?;
        Ok(row.map(tuple_to_row))
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn cleanup_expired(&self) -> Result<u64, StorageError> {
        let result = sqlx::query("DELETE FROM plane_a_oauth_states WHERE expires_at <= NOW()")
            .execute(&self.pool)
            .await
            .map_err(|e| map_db_err("plane_a_oauth_state", e))?;
        Ok(result.rows_affected())
    }

    #[tracing::instrument(level = "debug", skip(self, state))]
    async fn get_by_state(&self, state: &str) -> Result<Option<OAuthStateRow>, StorageError> {
        debug_assert!(!state.is_empty(), "state value must not be empty");
        let sql = format!("SELECT {SELECT_COLS} FROM plane_a_oauth_states WHERE state = $1");
        let row = sqlx::query_as::<_, StateTuple>(sqlx::AssertSqlSafe(sql))
            .bind(state)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("plane_a_oauth_state", e))?;
        Ok(row.map(tuple_to_row))
    }
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use std::{num::NonZeroU32, str::FromStr, sync::Arc, time::Duration as StdDuration};

    use chrono::{Duration, Utc};
    use sqlx::{
        ConnectOptions, Executor,
        postgres::{PgConnectOptions, PgPoolOptions},
    };
    use tokio::{sync::Barrier, task::JoinSet};

    use super::*;
    use crate::test_support::random_id;

    static ADMISSION_TEST_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn postgres_is_required() -> bool {
        match std::env::var("NEBULA_REQUIRE_POSTGRES") {
            Ok(value) if value == "1" => true,
            Ok(value) if value == "0" => false,
            Ok(_) => panic!("NEBULA_REQUIRE_POSTGRES must be either 0 or 1"),
            Err(std::env::VarError::NotPresent) => false,
            Err(std::env::VarError::NotUnicode(_)) => {
                panic!("NEBULA_REQUIRE_POSTGRES must be valid Unicode")
            },
        }
    }

    #[expect(
        clippy::print_stderr,
        reason = "local PostgreSQL tests must report an explicit missing-DATABASE_URL skip"
    )]
    fn database_url_for_test(
        postgres_required: bool,
        database_url: Result<String, std::env::VarError>,
    ) -> Option<String> {
        match database_url {
            Ok(url) => Some(url),
            Err(std::env::VarError::NotPresent) if postgres_required => {
                panic!("NEBULA_REQUIRE_POSTGRES=1 requires DATABASE_URL")
            },
            Err(std::env::VarError::NotPresent) => {
                eprintln!("OAuth admission PostgreSQL test skipped: DATABASE_URL is absent");
                None
            },
            Err(std::env::VarError::NotUnicode(_)) => {
                panic!("DATABASE_URL must be valid Unicode")
            },
        }
    }

    #[test]
    #[should_panic(expected = "NEBULA_REQUIRE_POSTGRES=1 requires DATABASE_URL")]
    fn required_postgres_mode_rejects_missing_database_url() {
        let _ = database_url_for_test(true, Err(std::env::VarError::NotPresent));
    }

    struct TestDatabase {
        admin_pool: Pool<Postgres>,
        first_pool: Pool<Postgres>,
        second_pool: Pool<Postgres>,
        schema: String,
    }

    impl TestDatabase {
        async fn create() -> Option<Self> {
            let url = database_url_for_test(postgres_is_required(), std::env::var("DATABASE_URL"))?;
            let admin_pool = PgPoolOptions::new()
                .max_connections(4)
                .connect(&url)
                .await
                .expect("connect admin pool");
            let schema = format!("oauth_state_test_{}", hex::encode(&random_id()[..12]));
            let create_schema = format!("CREATE SCHEMA {schema}");
            admin_pool
                .execute(sqlx::AssertSqlSafe(create_schema))
                .await
                .expect("create isolated schema");

            let connect_options = PgConnectOptions::from_str(&url)
                .expect("parse DATABASE_URL")
                .options([("search_path", schema.as_str())])
                .disable_statement_logging();
            let first_pool = PgPoolOptions::new()
                .max_connections(16)
                .connect_with(connect_options.clone())
                .await
                .expect("connect first isolated pool");
            let second_pool = PgPoolOptions::new()
                .max_connections(16)
                .connect_with(connect_options)
                .await
                .expect("connect second isolated pool");
            sqlx::query(
                "CREATE TABLE plane_a_oauth_states (\
                    state TEXT PRIMARY KEY, \
                    provider TEXT NOT NULL, \
                    code_verifier TEXT NOT NULL, \
                    redirect_uri TEXT, \
                    created_at TIMESTAMPTZ NOT NULL, \
                    expires_at TIMESTAMPTZ NOT NULL, \
                    consumed_at TIMESTAMPTZ\
                )",
            )
            .execute(&first_pool)
            .await
            .expect("create isolated OAuth state table");
            Some(Self {
                admin_pool,
                first_pool,
                second_pool,
                schema,
            })
        }

        async fn cleanup(self) {
            self.first_pool.close().await;
            self.second_pool.close().await;
            let drop_schema = format!("DROP SCHEMA {} CASCADE", self.schema);
            self.admin_pool
                .execute(sqlx::AssertSqlSafe(drop_schema))
                .await
                .expect("drop isolated schema");
            self.admin_pool.close().await;
        }
    }

    fn lazy_pool() -> Pool<Postgres> {
        PgPoolOptions::new()
            .acquire_timeout(StdDuration::from_millis(50))
            .connect_lazy("postgres://127.0.0.1:1/unused")
            .expect("syntactically valid lazy pool")
    }

    async fn insert_unchecked(pool: &Pool<Postgres>, state: &OAuthStateRow) {
        sqlx::query(
            "INSERT INTO plane_a_oauth_states \
             (state, provider, code_verifier, redirect_uri, created_at, expires_at, consumed_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&state.state)
        .bind(&state.provider)
        .bind(&state.code_verifier)
        .bind(state.redirect_uri.as_deref())
        .bind(state.created_at)
        .bind(state.expires_at)
        .bind(state.consumed_at)
        .execute(pool)
        .await
        .expect("seed isolated OAuth state row");
    }

    async fn active_count(pool: &Pool<Postgres>) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM plane_a_oauth_states \
             WHERE consumed_at IS NULL AND expires_at > statement_timestamp()",
        )
        .fetch_one(pool)
        .await
        .expect("count active OAuth state rows")
    }

    async fn total_count(pool: &Pool<Postgres>) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM plane_a_oauth_states")
            .fetch_one(pool)
            .await
            .expect("count all OAuth state rows")
    }

    async fn retry_contended(
        repo: &PgOAuthStateRepo,
        state: &OAuthStateRow,
    ) -> OAuthStateAdmission {
        for _ in 0..1_000 {
            match repo.admit(state).await.expect("admission") {
                OAuthStateAdmission::Contended => tokio::task::yield_now().await,
                settled => return settled,
            }
        }
        panic!("admission remained contended after bounded test retries")
    }

    #[tokio::test]
    async fn consumed_candidate_is_rejected_before_database_access_without_echoing_secrets() {
        const STATE_CANARY: &str = "OAUTH_STATE_ADMISSION_CANARY";
        let repo = PgOAuthStateRepo::new(lazy_pool());
        let mut state = fresh_state("google");
        state.state = STATE_CANARY.to_owned();
        state.consumed_at = Some(Utc::now());

        let error = repo
            .admit(&state)
            .await
            .expect_err("already-consumed candidate must be rejected");

        assert_eq!(error.to_string(), "internal: invalid OAuth state admission");
        assert!(!format!("{error:?}").contains(STATE_CANARY));
    }

    #[tokio::test]
    async fn admit_enforces_the_exact_sequential_capacity() {
        let _serial = ADMISSION_TEST_SERIAL.lock().await;
        let Some(database) = TestDatabase::create().await else {
            return;
        };
        let capacity = NonZeroU32::new(3).expect("non-zero test capacity");
        let repo = PgOAuthStateRepo::with_capacity(database.first_pool.clone(), capacity);

        for _ in 0..capacity.get() {
            assert_eq!(
                repo.admit(&fresh_state("google")).await.expect("admit"),
                OAuthStateAdmission::Created
            );
        }
        assert_eq!(
            repo.admit(&fresh_state("google")).await.expect("admit"),
            OAuthStateAdmission::AtCapacity
        );
        assert_eq!(active_count(&database.first_pool).await, 3);

        drop(repo);
        database.cleanup().await;
    }

    #[tokio::test]
    async fn admit_reclaims_expired_rows_before_capacity_check() {
        let _serial = ADMISSION_TEST_SERIAL.lock().await;
        let Some(database) = TestDatabase::create().await else {
            return;
        };
        let repo = PgOAuthStateRepo::with_capacity(
            database.first_pool.clone(),
            NonZeroU32::new(1).expect("non-zero test capacity"),
        );
        let mut expired = fresh_state("google");
        expired.expires_at = Utc::now() - Duration::minutes(1);
        insert_unchecked(&database.first_pool, &expired).await;

        assert_eq!(
            repo.admit(&fresh_state("google")).await.expect("admit"),
            OAuthStateAdmission::Created
        );
        assert_eq!(active_count(&database.first_pool).await, 1);
        assert_eq!(total_count(&database.first_pool).await, 1);

        drop(repo);
        database.cleanup().await;
    }

    #[tokio::test]
    async fn consumed_live_rows_do_not_use_active_capacity() {
        let _serial = ADMISSION_TEST_SERIAL.lock().await;
        let Some(database) = TestDatabase::create().await else {
            return;
        };
        let repo = PgOAuthStateRepo::with_capacity(
            database.first_pool.clone(),
            NonZeroU32::new(1).expect("non-zero test capacity"),
        );
        let mut consumed = fresh_state("google");
        consumed.consumed_at = Some(Utc::now());
        insert_unchecked(&database.first_pool, &consumed).await;

        assert_eq!(
            repo.admit(&fresh_state("google")).await.expect("admit"),
            OAuthStateAdmission::Created
        );
        assert_eq!(active_count(&database.first_pool).await, 1);
        assert_eq!(total_count(&database.first_pool).await, 2);

        drop(repo);
        database.cleanup().await;
    }

    #[tokio::test]
    async fn held_admission_lock_returns_contended_promptly_and_rollback_recovers() {
        let _serial = ADMISSION_TEST_SERIAL.lock().await;
        let Some(database) = TestDatabase::create().await else {
            return;
        };
        let mut holder = database.first_pool.begin().await.expect("begin holder");
        sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
            .bind(OAUTH_STATE_ADMISSION_LOCK_KEY.0)
            .bind(OAUTH_STATE_ADMISSION_LOCK_KEY.1)
            .execute(&mut *holder)
            .await
            .expect("hold admission lock");
        let repo = PgOAuthStateRepo::with_capacity(
            database.second_pool.clone(),
            NonZeroU32::new(1).expect("non-zero test capacity"),
        );

        let outcome = tokio::time::timeout(
            StdDuration::from_secs(1),
            repo.admit(&fresh_state("google")),
        )
        .await
        .expect("try-lock admission must return promptly")
        .expect("contended admission remains a normal outcome");
        assert_eq!(outcome, OAuthStateAdmission::Contended);

        holder.rollback().await.expect("release held lock");
        assert_eq!(
            repo.admit(&fresh_state("google"))
                .await
                .expect("admit after release"),
            OAuthStateAdmission::Created
        );

        drop(repo);
        database.cleanup().await;
    }

    #[tokio::test]
    async fn committed_admission_releases_transaction_lock() {
        let _serial = ADMISSION_TEST_SERIAL.lock().await;
        let Some(database) = TestDatabase::create().await else {
            return;
        };
        let repo = PgOAuthStateRepo::with_capacity(
            database.first_pool.clone(),
            NonZeroU32::new(1).expect("non-zero test capacity"),
        );
        assert_eq!(
            repo.admit(&fresh_state("google")).await.expect("admit"),
            OAuthStateAdmission::Created
        );

        let mut verifier = database.second_pool.begin().await.expect("begin verifier");
        let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1, $2)")
            .bind(OAUTH_STATE_ADMISSION_LOCK_KEY.0)
            .bind(OAUTH_STATE_ADMISSION_LOCK_KEY.1)
            .fetch_one(&mut *verifier)
            .await
            .expect("try admission lock");
        assert!(acquired, "committed admission must release its xact lock");
        verifier.rollback().await.expect("release verifier lock");

        drop(repo);
        database.cleanup().await;
    }

    #[tokio::test]
    async fn failed_admission_explicitly_rolls_back_and_releases_transaction_lock() {
        let _serial = ADMISSION_TEST_SERIAL.lock().await;
        let Some(database) = TestDatabase::create().await else {
            return;
        };
        sqlx::query("DROP TABLE plane_a_oauth_states")
            .execute(&database.first_pool)
            .await
            .expect("remove table to force a post-lock SQL failure");
        let repo = PgOAuthStateRepo::with_capacity(
            database.first_pool.clone(),
            NonZeroU32::new(1).expect("non-zero test capacity"),
        );

        repo.admit(&fresh_state("google"))
            .await
            .expect_err("missing table must fail admission");

        let mut verifier = database.second_pool.begin().await.expect("begin verifier");
        let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1, $2)")
            .bind(OAUTH_STATE_ADMISSION_LOCK_KEY.0)
            .bind(OAUTH_STATE_ADMISSION_LOCK_KEY.1)
            .fetch_one(&mut *verifier)
            .await
            .expect("try admission lock after failure");
        assert!(
            acquired,
            "post-lock SQL failure must explicitly roll back and release its xact lock"
        );
        verifier.rollback().await.expect("release verifier lock");

        drop(repo);
        database.cleanup().await;
    }

    #[tokio::test]
    async fn two_pools_saturate_under_64_way_contention_without_exceeding_cap() {
        let _serial = ADMISSION_TEST_SERIAL.lock().await;
        let Some(database) = TestDatabase::create().await else {
            return;
        };
        let capacity = NonZeroU32::new(11).expect("non-zero test capacity");
        let first_repo = PgOAuthStateRepo::with_capacity(database.first_pool.clone(), capacity);
        let second_repo = PgOAuthStateRepo::with_capacity(database.second_pool.clone(), capacity);
        let barrier = Arc::new(Barrier::new(64));
        let mut tasks = JoinSet::new();
        for index in 0..64 {
            let repo = if index % 2 == 0 {
                first_repo.clone()
            } else {
                second_repo.clone()
            };
            let barrier = Arc::clone(&barrier);
            let state = fresh_state("google");
            tasks.spawn(async move {
                barrier.wait().await;
                retry_contended(&repo, &state).await
            });
        }

        let mut created = 0;
        let mut at_capacity = 0;
        while let Some(result) = tasks.join_next().await {
            match result.expect("admission task") {
                OAuthStateAdmission::Created => created += 1,
                OAuthStateAdmission::AtCapacity => at_capacity += 1,
                OAuthStateAdmission::Contended => {
                    panic!("bounded retry helper must settle contention")
                },
            }
        }
        assert_eq!(created, capacity.get());
        assert_eq!(at_capacity, 64 - capacity.get());
        assert_eq!(
            active_count(&database.first_pool).await,
            i64::from(capacity.get())
        );

        drop(first_repo);
        drop(second_repo);
        database.cleanup().await;
    }

    #[tokio::test]
    async fn create_get_roundtrip() {
        let _serial = ADMISSION_TEST_SERIAL.lock().await;
        let Some(database) = TestDatabase::create().await else {
            return;
        };
        let repo = PgOAuthStateRepo::new(database.first_pool.clone());
        let state = fresh_state("google");
        let state_value = state.state.clone();

        assert_eq!(
            repo.admit(&state).await.expect("admit"),
            OAuthStateAdmission::Created
        );
        let loaded = repo
            .get_by_state(&state_value)
            .await
            .expect("get_by_state")
            .expect("some");
        assert_eq!(loaded.state, state_value);
        assert_eq!(loaded.provider, "google");
        assert_eq!(loaded.code_verifier, state.code_verifier);
        assert!(loaded.consumed_at.is_none());

        drop(repo);
        database.cleanup().await;
    }

    #[tokio::test]
    async fn duplicate_state_is_rejected() {
        let _serial = ADMISSION_TEST_SERIAL.lock().await;
        let Some(database) = TestDatabase::create().await else {
            return;
        };
        let repo = PgOAuthStateRepo::new(database.first_pool.clone());
        let state = fresh_state("github");
        assert_eq!(
            repo.admit(&state).await.expect("first admit"),
            OAuthStateAdmission::Created
        );

        let error = repo
            .admit(&state)
            .await
            .expect_err("duplicate state must reject");
        assert!(
            matches!(
                error,
                StorageError::Duplicate {
                    entity: "plane_a_oauth_state",
                    ..
                }
            ),
            "expected duplicate OAuth state, got: {error:?}"
        );

        drop(repo);
        database.cleanup().await;
    }

    #[tokio::test]
    async fn consume_by_state_is_single_shot() {
        let Some(database) = TestDatabase::create().await else {
            return;
        };
        let repo = PgOAuthStateRepo::new(database.first_pool.clone());
        let state = fresh_state("google");
        let state_value = state.state.clone();
        let verifier = state.code_verifier.clone();
        insert_unchecked(&database.first_pool, &state).await;

        let first = repo
            .consume_by_state(&state_value)
            .await
            .expect("consume")
            .expect("some");
        assert_eq!(first.code_verifier, verifier);
        assert!(
            repo.consume_by_state(&state_value)
                .await
                .expect("replay")
                .is_none(),
            "state consumption must be single-shot"
        );

        drop(repo);
        database.cleanup().await;
    }

    #[tokio::test]
    async fn consume_by_state_and_provider_mismatch_does_not_burn_state() {
        let Some(database) = TestDatabase::create().await else {
            return;
        };
        let repo = PgOAuthStateRepo::new(database.first_pool.clone());
        let state = fresh_state("google");
        let state_value = state.state.clone();
        insert_unchecked(&database.first_pool, &state).await;

        assert!(
            repo.consume_by_state_and_provider(&state_value, "github")
                .await
                .expect("wrong provider")
                .is_none()
        );
        assert!(
            repo.consume_by_state_and_provider(&state_value, "google")
                .await
                .expect("right provider")
                .is_some(),
            "wrong-provider callback must leave state consumable"
        );

        drop(repo);
        database.cleanup().await;
    }

    #[tokio::test]
    async fn cleanup_expired_deletes_only_past_rows() {
        let Some(database) = TestDatabase::create().await else {
            return;
        };
        let repo = PgOAuthStateRepo::new(database.first_pool.clone());
        let mut expired = fresh_state("google");
        expired.expires_at = Utc::now() - Duration::minutes(1);
        let expired_value = expired.state.clone();
        let live = fresh_state("google");
        let live_value = live.state.clone();
        insert_unchecked(&database.first_pool, &expired).await;
        insert_unchecked(&database.first_pool, &live).await;

        let deleted = repo.cleanup_expired().await.expect("cleanup");
        assert_eq!(deleted, 1);
        assert!(
            repo.get_by_state(&expired_value)
                .await
                .expect("get expired")
                .is_none()
        );
        assert!(
            repo.get_by_state(&live_value)
                .await
                .expect("get live")
                .is_some()
        );

        drop(repo);
        database.cleanup().await;
    }

    fn fresh_state(provider: &str) -> OAuthStateRow {
        let now = Utc::now();
        OAuthStateRow {
            state: format!("st_{}", hex::encode(&random_id()[..12])),
            provider: provider.to_owned(),
            code_verifier: format!("vrf_{}", hex::encode(&random_id()[..16])),
            redirect_uri: Some("https://nebula.local/auth/callback".to_owned()),
            created_at: now,
            expires_at: now + Duration::minutes(10),
            consumed_at: None,
        }
    }
}
