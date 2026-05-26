//! Postgres implementation of [`PatRepo`].
//!
//! Schema: migration `0002_user_auth.sql` (`personal_access_tokens`
//! table) \u2014 the SHA-256 lookup leans on `idx_pat_hash`, which is a
//! partial index over the `hash` column where `revoked_at IS NULL`.
//!
//! Liveness rules:
//!
//! - [`get_by_hash`](PatRepo::get_by_hash) returns `None` for revoked
//!   or expired rows; callers do not need to re-check.
//! - [`touch`](PatRepo::touch) is best-effort and stays a no-op for
//!   revoked or expired rows.
//! - [`revoke`](PatRepo::revoke) is idempotent: the `revoked_at IS NULL`
//!   guard preserves the original timestamp on re-revoke.
//! - [`list_for_principal`](PatRepo::list_for_principal) only returns
//!   active tokens (not revoked, not expired) sorted by `created_at`.

use sqlx::{Pool, Postgres, types::Json};

use crate::{error::StorageError, pg::map_db_err, repos::PatRepo, rows::PersonalAccessTokenRow};

/// Postgres-backed personal access token repository.
#[derive(Clone)]
pub struct PgPatRepo {
    pool: Pool<Postgres>,
}

impl PgPatRepo {
    /// Construct from an existing pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }
}

// Column order must match every `SELECT ... FROM personal_access_tokens`
// in this file.
type PatTuple = (
    Vec<u8>,                               // id
    String,                                // principal_kind
    Vec<u8>,                               // principal_id
    String,                                // name
    String,                                // prefix
    Vec<u8>,                               // hash
    Json<serde_json::Value>,               // scopes
    chrono::DateTime<chrono::Utc>,         // created_at
    Option<chrono::DateTime<chrono::Utc>>, // last_used_at
    Option<chrono::DateTime<chrono::Utc>>, // expires_at
    Option<chrono::DateTime<chrono::Utc>>, // revoked_at
);

fn tuple_to_row(t: PatTuple) -> PersonalAccessTokenRow {
    PersonalAccessTokenRow {
        id: t.0,
        principal_kind: t.1,
        principal_id: t.2,
        name: t.3,
        prefix: t.4,
        hash: t.5,
        scopes: t.6.0,
        created_at: t.7,
        last_used_at: t.8,
        expires_at: t.9,
        revoked_at: t.10,
    }
}

const SELECT_COLS: &str = "id, principal_kind, principal_id, name, prefix, hash, scopes, \
     created_at, last_used_at, expires_at, revoked_at";

impl PatRepo for PgPatRepo {
    #[tracing::instrument(
        level = "debug",
        skip(self, pat),
        fields(pat_id = %hex::encode(&pat.id), principal_kind = %pat.principal_kind)
    )]
    async fn create(&self, pat: &PersonalAccessTokenRow) -> Result<(), StorageError> {
        debug_assert!(!pat.id.is_empty(), "pat.id must not be empty");
        debug_assert!(!pat.hash.is_empty(), "pat.hash must not be empty");
        debug_assert!(
            !pat.principal_id.is_empty(),
            "pat.principal_id must not be empty"
        );
        sqlx::query(
            "INSERT INTO personal_access_tokens \
             (id, principal_kind, principal_id, name, prefix, hash, scopes, \
              created_at, last_used_at, expires_at, revoked_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(&pat.id)
        .bind(&pat.principal_kind)
        .bind(&pat.principal_id)
        .bind(&pat.name)
        .bind(&pat.prefix)
        .bind(&pat.hash)
        .bind(Json(&pat.scopes))
        .bind(pat.created_at)
        .bind(pat.last_used_at)
        .bind(pat.expires_at)
        .bind(pat.revoked_at)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("pat", e))?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, hash))]
    async fn get_by_hash(
        &self,
        hash: &[u8],
    ) -> Result<Option<PersonalAccessTokenRow>, StorageError> {
        debug_assert!(!hash.is_empty(), "hash must not be empty");
        // Liveness filter mirrors `idx_pat_hash` (partial on
        // `revoked_at IS NULL`) so the planner can use the index for
        // O(log n) lookup; `expires_at` is then checked in the
        // predicate so we never surface an expired token.
        let sql = format!(
            "SELECT {SELECT_COLS} FROM personal_access_tokens \
             WHERE hash = $1 AND revoked_at IS NULL \
               AND (expires_at IS NULL OR expires_at > NOW())"
        );
        let row = sqlx::query_as::<_, PatTuple>(&sql)
            .bind(hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_db_err("pat", e))?;
        Ok(row.map(tuple_to_row))
    }

    #[tracing::instrument(level = "debug", skip(self), fields(pat_id = %hex::encode(id)))]
    async fn touch(&self, id: &[u8]) -> Result<(), StorageError> {
        debug_assert!(!id.is_empty(), "id must not be empty");
        sqlx::query(
            "UPDATE personal_access_tokens SET last_used_at = NOW() \
             WHERE id = $1 AND revoked_at IS NULL \
               AND (expires_at IS NULL OR expires_at > NOW())",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("pat", e))?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self), fields(pat_id = %hex::encode(id)))]
    async fn revoke(&self, id: &[u8]) -> Result<(), StorageError> {
        debug_assert!(!id.is_empty(), "id must not be empty");
        // Idempotent: the `revoked_at IS NULL` guard preserves the
        // original revocation timestamp on re-revoke.
        sqlx::query(
            "UPDATE personal_access_tokens SET revoked_at = NOW() \
             WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("pat", e))?;
        Ok(())
    }

    #[tracing::instrument(
        level = "debug",
        skip(self),
        fields(principal_kind, principal_id = %hex::encode(principal_id))
    )]
    async fn list_for_principal(
        &self,
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<Vec<PersonalAccessTokenRow>, StorageError> {
        debug_assert!(
            !principal_kind.is_empty(),
            "principal_kind must not be empty"
        );
        debug_assert!(!principal_id.is_empty(), "principal_id must not be empty");
        let sql = format!(
            "SELECT {SELECT_COLS} FROM personal_access_tokens \
             WHERE principal_kind = $1 AND principal_id = $2 \
               AND revoked_at IS NULL \
               AND (expires_at IS NULL OR expires_at > NOW()) \
             ORDER BY created_at"
        );
        let rows = sqlx::query_as::<_, PatTuple>(&sql)
            .bind(principal_kind)
            .bind(principal_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| map_db_err("pat", e))?;
        Ok(rows.into_iter().map(tuple_to_row).collect())
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

    fn fresh_pat(user_id: &[u8]) -> PersonalAccessTokenRow {
        let id = random_id();
        let hash = {
            let mut h = [0u8; 32];
            let src = random_id();
            h[..src.len().min(32)].copy_from_slice(&src[..src.len().min(32)]);
            h.to_vec()
        };
        PersonalAccessTokenRow {
            id,
            principal_kind: "user".to_string(),
            principal_id: user_id.to_vec(),
            name: "nebula-test".to_string(),
            prefix: "nbpat_test_".to_string(),
            hash,
            scopes: json!([]),
            created_at: Utc::now(),
            last_used_at: None,
            expires_at: Some(Utc::now() + Duration::days(7)),
            revoked_at: None,
        }
    }

    #[tokio::test]
    async fn create_get_by_hash_roundtrip() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "pat-roundtrip").await;
        let repo = PgPatRepo::new(pool);
        let pat = fresh_pat(&user_id);
        let hash = pat.hash.clone();

        repo.create(&pat).await.expect("create");
        let loaded = repo
            .get_by_hash(&hash)
            .await
            .expect("get_by_hash")
            .expect("some");
        assert_eq!(loaded.id, pat.id);
        assert_eq!(loaded.name, "nebula-test");
        assert_eq!(loaded.scopes, json!([]));
        assert!(loaded.revoked_at.is_none());
    }

    #[tokio::test]
    async fn duplicate_id_is_rejected() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "pat-dup").await;
        let repo = PgPatRepo::new(pool);
        let pat = fresh_pat(&user_id);
        repo.create(&pat).await.expect("first create");

        let err = repo
            .create(&pat)
            .await
            .expect_err("duplicate pat id must reject");
        assert!(
            matches!(err, StorageError::Duplicate { entity: "pat", .. }),
            "expected Duplicate {{ entity: 'pat', .. }}, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn revoke_hides_token_from_lookup() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "pat-revoke").await;
        let repo = PgPatRepo::new(pool);
        let pat = fresh_pat(&user_id);
        let hash = pat.hash.clone();
        repo.create(&pat).await.expect("create");

        repo.revoke(&pat.id).await.expect("revoke");
        let after = repo.get_by_hash(&hash).await.expect("get_by_hash");
        assert!(after.is_none(), "revoked PAT must not surface");
        // Idempotent: re-revoking is a no-op.
        repo.revoke(&pat.id).await.expect("idempotent revoke");
    }

    #[tokio::test]
    async fn list_for_principal_returns_only_active_tokens() {
        let Some(pool) = pool().await else { return };
        let user_id = seed_user(&pool, "pat-list").await;
        let repo = PgPatRepo::new(pool);

        let live = fresh_pat(&user_id);
        let mut revoked = fresh_pat(&user_id);
        revoked.revoked_at = Some(Utc::now());
        let mut expired = fresh_pat(&user_id);
        expired.expires_at = Some(Utc::now() - Duration::seconds(60));
        repo.create(&live).await.expect("create live");
        repo.create(&revoked).await.expect("create revoked");
        repo.create(&expired).await.expect("create expired");

        let listed = repo
            .list_for_principal("user", &user_id)
            .await
            .expect("list");
        assert_eq!(listed.len(), 1, "only the live token should surface");
        assert_eq!(listed[0].id, live.id);
    }
}
