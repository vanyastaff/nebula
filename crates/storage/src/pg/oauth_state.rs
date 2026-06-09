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

use sqlx::{Pool, Postgres};

use crate::{error::StorageError, pg::map_db_err, repos::OAuthStateRepo, rows::OAuthStateRow};

/// Postgres-backed Plane-A OAuth PKCE state repository.
#[derive(Clone)]
pub struct PgOAuthStateRepo {
    pool: Pool<Postgres>,
}

impl PgOAuthStateRepo {
    /// Construct from an existing pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
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
    async fn create(&self, state: &OAuthStateRow) -> Result<(), StorageError> {
        debug_assert!(!state.state.is_empty(), "state value must not be empty");
        debug_assert!(!state.provider.is_empty(), "provider must not be empty");
        debug_assert!(
            !state.code_verifier.is_empty(),
            "code_verifier must not be empty"
        );
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
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("plane_a_oauth_state", e))?;
        Ok(())
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
    use chrono::{Duration, Utc};
    use sqlx::postgres::PgPoolOptions;

    use super::*;
    use crate::test_support::random_id;

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

    fn fresh_state(provider: &str) -> OAuthStateRow {
        let state = format!("st_{}", hex::encode(&random_id()[..12]));
        let verifier = format!("vrf_{}", hex::encode(&random_id()[..16]));
        OAuthStateRow {
            state,
            provider: provider.to_string(),
            code_verifier: verifier,
            redirect_uri: Some("https://nebula.local/auth/callback".to_string()),
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::minutes(10),
            consumed_at: None,
        }
    }

    #[tokio::test]
    async fn create_get_roundtrip() {
        let Some(pool) = pool().await else { return };
        let repo = PgOAuthStateRepo::new(pool);
        let state = fresh_state("google");
        let state_value = state.state.clone();

        repo.create(&state).await.expect("create");
        let loaded = repo
            .get_by_state(&state_value)
            .await
            .expect("get_by_state")
            .expect("some");
        assert_eq!(loaded.state, state_value);
        assert_eq!(loaded.provider, "google");
        assert_eq!(loaded.code_verifier, state.code_verifier);
        assert!(loaded.consumed_at.is_none());
    }

    #[tokio::test]
    async fn duplicate_state_is_rejected() {
        let Some(pool) = pool().await else { return };
        let repo = PgOAuthStateRepo::new(pool);
        let state = fresh_state("github");
        repo.create(&state).await.expect("first create");

        let err = repo
            .create(&state)
            .await
            .expect_err("duplicate state must reject");
        assert!(
            matches!(
                err,
                StorageError::Duplicate {
                    entity: "plane_a_oauth_state",
                    ..
                }
            ),
            "expected Duplicate {{ entity: 'plane_a_oauth_state', .. }}, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn consume_by_state_is_single_shot() {
        let Some(pool) = pool().await else { return };
        let repo = PgOAuthStateRepo::new(pool);
        let state = fresh_state("microsoft");
        let state_value = state.state.clone();
        let verifier = state.code_verifier.clone();
        repo.create(&state).await.expect("create");

        let first = repo
            .consume_by_state(&state_value)
            .await
            .expect("consume")
            .expect("some");
        assert_eq!(first.code_verifier, verifier);

        let second = repo
            .consume_by_state(&state_value)
            .await
            .expect("consume second");
        assert!(
            second.is_none(),
            "second consume must return None (replay defence)"
        );
    }

    #[tokio::test]
    async fn consume_by_state_and_provider_matching_provider_consumes() {
        let Some(pool) = pool().await else { return };
        let repo = PgOAuthStateRepo::new(pool);
        let state = fresh_state("google");
        let state_value = state.state.clone();
        let verifier = state.code_verifier.clone();
        repo.create(&state).await.expect("create");

        let consumed = repo
            .consume_by_state_and_provider(&state_value, "google")
            .await
            .expect("consume")
            .expect("some");
        assert_eq!(consumed.code_verifier, verifier);

        // Replay against the same provider returns None.
        let replay = repo
            .consume_by_state_and_provider(&state_value, "google")
            .await
            .expect("replay");
        assert!(
            replay.is_none(),
            "second consume must return None (replay defence)"
        );
    }

    #[tokio::test]
    async fn consume_by_state_and_provider_mismatched_provider_does_not_consume() {
        let Some(pool) = pool().await else { return };
        let repo = PgOAuthStateRepo::new(pool);
        // Persisted under provider=google, but the caller claims github.
        let state = fresh_state("google");
        let state_value = state.state.clone();
        repo.create(&state).await.expect("create");

        let wrong_provider = repo
            .consume_by_state_and_provider(&state_value, "github")
            .await
            .expect("consume");
        assert!(
            wrong_provider.is_none(),
            "mismatched provider must NOT consume the row"
        );

        // The valid callback at the correct provider still succeeds
        // — the row was not burned by the mismatched attempt.
        let correct = repo
            .consume_by_state_and_provider(&state_value, "google")
            .await
            .expect("consume correct")
            .expect("row still consumable");
        assert_eq!(correct.provider, "google");
    }

    #[tokio::test]
    async fn cleanup_expired_deletes_only_past_rows() {
        let Some(pool) = pool().await else { return };
        let repo = PgOAuthStateRepo::new(pool);

        let mut expired = fresh_state("google");
        expired.expires_at = Utc::now() - Duration::minutes(1);
        let expired_value = expired.state.clone();
        let live = fresh_state("google");
        let live_value = live.state.clone();

        repo.create(&expired).await.expect("create expired");
        repo.create(&live).await.expect("create live");

        let _deleted = repo.cleanup_expired().await.expect("cleanup");

        assert!(
            repo.get_by_state(&expired_value)
                .await
                .expect("get expired")
                .is_none(),
            "expired row must be deleted"
        );
        assert!(
            repo.get_by_state(&live_value)
                .await
                .expect("get live")
                .is_some(),
            "live row must remain"
        );
    }
}
