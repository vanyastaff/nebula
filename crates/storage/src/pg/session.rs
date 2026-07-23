//! Postgres implementation of [`SessionRepo`].
//!
//! Schema: migration `0002_user_auth.sql` (`sessions` table).
//!
//! Liveness rules (mirrored from the in-memory backend):
//!
//! - [`get`](SessionRepo::get) returns `None` for any row that is
//!   revoked (`revoked_at IS NOT NULL`) or expired
//!   (`expires_at <= NOW()`); the caller does not need to re-check.
//! - [`touch`](SessionRepo::touch) bumps `last_active_at` to `NOW()`
//!   on live rows and is a no-op for revoked or expired rows.
//! - [`revoke`](SessionRepo::revoke) is idempotent: re-revoking a
//!   row leaves the original `revoked_at` in place.
//! - [`cleanup_expired`](SessionRepo::cleanup_expired) deletes rows
//!   whose `expires_at` is in the past (regardless of revocation),
//!   freeing index space for the sweep.

use sqlx::{Pool, Postgres};

use crate::{
    error::StorageError,
    pg::map_db_err,
    repos::SessionRepo,
    rows::{SessionDraft, SessionRow},
    session_token::{SessionTokenDigest, session_token_digest},
};

/// Postgres-backed session repository.
#[derive(Clone)]
pub struct PgSessionRepo {
    pool: Pool<Postgres>,
}

impl PgSessionRepo {
    /// Construct from an existing pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }
}

// Column order must match every `SELECT ... FROM sessions` in this file.
type SessionTuple = (
    Vec<u8>,                               // token_digest
    Vec<u8>,                               // user_id
    chrono::DateTime<chrono::Utc>,         // created_at
    chrono::DateTime<chrono::Utc>,         // last_active_at
    chrono::DateTime<chrono::Utc>,         // expires_at
    Option<String>,                        // ip_address
    Option<String>,                        // user_agent
    Option<chrono::DateTime<chrono::Utc>>, // revoked_at
);

fn tuple_to_row(t: SessionTuple) -> Result<SessionRow, StorageError> {
    let digest: [u8; 32] = t.0.try_into().map_err(|_: Vec<u8>| {
        StorageError::Serialization("session token digest is not 32 bytes".to_owned())
    })?;
    Ok(SessionRow {
        token_digest: SessionTokenDigest::from_bytes(digest),
        user_id: t.1,
        created_at: t.2,
        last_active_at: t.3,
        expires_at: t.4,
        ip_address: t.5,
        user_agent: t.6,
        revoked_at: t.7,
    })
}

// `host(ip_address)` projects the address without PostgreSQL's implicit
// host-netmask suffix (`/32` for IPv4, `/128` for IPv6).
const SELECT_COLS: &str = "token_digest, user_id, created_at, last_active_at, expires_at, \
     host(ip_address) AS ip_address, user_agent, revoked_at";

impl SessionRepo for PgSessionRepo {
    #[tracing::instrument(level = "debug", skip(self, presented_token, session))]
    async fn create(
        &self,
        presented_token: &[u8],
        session: &SessionDraft,
    ) -> Result<(), StorageError> {
        debug_assert!(
            !presented_token.is_empty(),
            "presented session token must not be empty"
        );
        debug_assert!(
            !session.user_id.is_empty(),
            "session.user_id must not be empty"
        );
        sqlx::query(
            "INSERT INTO sessions \
             (token_digest, user_id, created_at, last_active_at, expires_at, \
              ip_address, user_agent, revoked_at) \
             VALUES ($1, $2, $3, $4, $5, $6::inet, $7, $8)",
        )
        .bind(session_token_digest(presented_token).as_bytes().as_slice())
        .bind(&session.user_id)
        .bind(session.created_at)
        .bind(session.last_active_at)
        .bind(session.expires_at)
        .bind(session.ip_address.as_deref())
        .bind(session.user_agent.as_deref())
        .bind(session.revoked_at)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("session", e))?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, presented_token))]
    async fn get(&self, presented_token: &[u8]) -> Result<Option<SessionRow>, StorageError> {
        let digest = session_token_digest(presented_token);
        let sql = format!(
            "SELECT {SELECT_COLS} FROM sessions \
             WHERE token_digest = $1 AND revoked_at IS NULL AND expires_at > NOW()"
        );
        let row = sqlx::query_as::<_, SessionTuple>(sqlx::AssertSqlSafe(sql))
            .bind(digest.as_bytes().as_slice())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("session", e))?;
        row.map(tuple_to_row).transpose()
    }

    #[tracing::instrument(level = "debug", skip(self, presented_token))]
    async fn touch(&self, presented_token: &[u8]) -> Result<(), StorageError> {
        let digest = session_token_digest(presented_token);
        sqlx::query(
            "UPDATE sessions SET last_active_at = NOW() \
             WHERE token_digest = $1 AND revoked_at IS NULL AND expires_at > NOW()",
        )
        .bind(digest.as_bytes().as_slice())
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("session", e))?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, presented_token))]
    async fn revoke(&self, presented_token: &[u8]) -> Result<(), StorageError> {
        let digest = session_token_digest(presented_token);
        // Idempotent: `revoked_at IS NULL` guards re-revocation so the
        // original revocation timestamp is preserved.
        sqlx::query(
            "UPDATE sessions SET revoked_at = NOW() \
             WHERE token_digest = $1 AND revoked_at IS NULL",
        )
        .bind(digest.as_bytes().as_slice())
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("session", e))?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn cleanup_expired(&self) -> Result<u64, StorageError> {
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= NOW()")
            .execute(&self.pool)
            .await
            .map_err(|e| map_db_err("session", e))?;
        Ok(result.rows_affected())
    }
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use chrono::{Duration, Utc};
    use sqlx::postgres::PgPoolOptions;

    use super::*;
    use crate::{
        pg::user::PgUserRepo,
        repos::UserRepo,
        rows::SessionDraft,
        session_token::session_token_digest,
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

    fn fresh_session(user_id: &[u8]) -> (Vec<u8>, SessionDraft) {
        let now = Utc::now();
        (
            random_id(),
            SessionDraft {
                user_id: user_id.to_vec(),
                created_at: now,
                last_active_at: now,
                expires_at: now + Duration::hours(2),
                ip_address: Some("192.0.2.1".to_string()),
                user_agent: Some("nebula-test/1.0".to_string()),
                revoked_at: None,
            },
        )
    }

    #[tokio::test]
    async fn create_get_roundtrip() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "sess-create").await;
        let repo = PgSessionRepo::new(pool);
        let (token, session) = fresh_session(&user_id);

        repo.create(&token, &session).await.expect("create");
        let loaded = repo.get(&token).await.expect("get").expect("some");
        assert_eq!(loaded.token_digest, session_token_digest(&token));
        assert_eq!(loaded.user_id, user_id);
        assert_eq!(loaded.ip_address.as_deref(), Some("192.0.2.1"));
        assert!(loaded.revoked_at.is_none());
    }

    #[tokio::test]
    async fn ipv6_roundtrip_preserves_the_bare_host_contract() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "sess-ipv6").await;
        let repo = PgSessionRepo::new(pool);
        let (token, mut session) = fresh_session(&user_id);
        session.ip_address = Some("2001:db8::1".to_owned());

        repo.create(&token, &session).await.expect("create");
        let loaded = repo.get(&token).await.expect("get").expect("some");
        assert_eq!(loaded.ip_address.as_deref(), Some("2001:db8::1"));
    }

    #[tokio::test]
    async fn duplicate_id_is_rejected() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "sess-dup").await;
        let repo = PgSessionRepo::new(pool);
        let (token, session) = fresh_session(&user_id);
        repo.create(&token, &session).await.expect("first create");

        let err = repo
            .create(&token, &session)
            .await
            .expect_err("duplicate session id must reject");
        assert!(
            matches!(
                err,
                StorageError::Duplicate {
                    entity: "session",
                    ..
                }
            ),
            "expected Duplicate {{ entity: 'session', .. }}, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn revoke_then_get_returns_none_idempotent() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "sess-revoke").await;
        let repo = PgSessionRepo::new(pool);
        let (token, session) = fresh_session(&user_id);
        repo.create(&token, &session).await.expect("create");

        repo.revoke(&token).await.expect("revoke");
        let after = repo.get(&token).await.expect("get");
        assert!(after.is_none(), "revoked session must not surface from get");

        // Second revoke is a no-op (no error).
        repo.revoke(&token).await.expect("idempotent revoke");
    }

    #[tokio::test]
    async fn cleanup_expired_deletes_only_past_rows() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "sess-cleanup").await;
        let repo = PgSessionRepo::new(pool);

        let (expired_token, mut expired) = fresh_session(&user_id);
        expired.expires_at = Utc::now() - Duration::seconds(60);
        let (live_token, mut live) = fresh_session(&user_id);
        live.expires_at = Utc::now() + Duration::hours(1);

        let expired_digest = session_token_digest(&expired_token);
        let live_digest = session_token_digest(&live_token);
        repo.create(&expired_token, &expired)
            .await
            .expect("create expired");
        repo.create(&live_token, &live).await.expect("create live");

        let _deleted = repo.cleanup_expired().await.expect("cleanup");

        // Expired row gone; live row remains.
        let after_expired: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE token_digest = $1")
                .bind(expired_digest.as_bytes().as_slice())
                .fetch_one(&repo.pool)
                .await
                .expect("count expired");
        assert_eq!(after_expired, 0);
        let after_live: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE token_digest = $1")
                .bind(live_digest.as_bytes().as_slice())
                .fetch_one(&repo.pool)
                .await
                .expect("count live");
        assert_eq!(after_live, 1);
    }
}
