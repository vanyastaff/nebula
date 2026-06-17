//! Postgres `IdempotencyStore` (durable idempotent-replay cache) +
//! `WebhookActivationStore` (webhook activation specs) over the port-scoped schema.
//!
//! The cache is keyed by `{workspace_id}:{org_id}:{cache_key}` (the store
//! folds the scope into the key, so tenant A can neither probe nor poison
//! tenant B's dedup entry; first-writer-wins via INSERT ... ON CONFLICT DO
//! NOTHING). Webhook activations are keyed by `(workspace, org, slug)` so
//! resolution never crosses a tenant boundary.

use std::time::Duration;

use chrono::{DateTime, Utc};
use nebula_storage_port::dto::{CachedRecord, WebhookActivationRecord, WebhookMode};
use nebula_storage_port::store::{IdempotencyStore, WebhookActivationStore};
use nebula_storage_port::{Scope, StorageError};
use sqlx::{PgPool, Row};

use super::execution::conn_err;

/// Parse an RFC 3339 expiry into epoch milliseconds. A malformed
/// timestamp maps to `i64::MIN` (fail-closed: an unparsable expiry is
/// treated as already-expired so a record we cannot prove fresh is never
/// served and is swept promptly). Mirrors the SQLite backend's
/// `expires_at_ms` so `expires_at_ms BIGINT` is the cross-dialect sweep
/// predicate (no `TIMESTAMPTZ` drift).
fn expires_at_ms(rfc3339: &str) -> i64 {
    DateTime::parse_from_rfc3339(rfc3339)
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(i64::MIN)
}

/// Fold the scope into the cache key (`{workspace_id}:{org_id}:{cache_key}`)
/// so two tenants' keyspaces are disjoint — a raw key from one tenant can
/// never collide with another's (§6.1 replay-oracle).
fn namespaced(scope: &Scope, cache_key: &str) -> String {
    format!("{}:{}:{}", scope.workspace_id, scope.org_id, cache_key)
}

/// Postgres-backed durable idempotent-replay cache.
#[derive(Clone, Debug)]
pub struct PgIdempotencyStore {
    pool: PgPool,
}

impl PgIdempotencyStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IdempotencyStore for PgIdempotencyStore {
    async fn get(
        &self,
        scope: &Scope,
        cache_key: &str,
    ) -> Result<Option<CachedRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT status, headers, body, fingerprint, expires_at \
             FROM port_idempotency_cache WHERE cache_key = $1",
        )
        .bind(namespaced(scope, cache_key))
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(CachedRecord {
            status: row.try_get::<i32, _>("status").map_err(conn_err)? as u16,
            headers: row.try_get("headers").map_err(conn_err)?,
            body: row.try_get("body").map_err(conn_err)?,
            fingerprint: row.try_get("fingerprint").map_err(conn_err)?,
            // `expires_at` is the RFC 3339 text stored verbatim (same as
            // the SQLite backend), returned unchanged — not reconstructed
            // from a `TIMESTAMPTZ` round-trip.
            expires_at: row.try_get("expires_at").map_err(conn_err)?,
        }))
    }

    async fn put(
        &self,
        scope: &Scope,
        cache_key: String,
        record: CachedRecord,
        _ttl: Duration,
    ) -> Result<(), StorageError> {
        // First-writer-wins: ON CONFLICT DO NOTHING keeps the original
        // record (and its fingerprint) on a replay race.
        sqlx::query(
            "INSERT INTO port_idempotency_cache \
             (cache_key, status, headers, body, fingerprint, expires_at, \
              expires_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (cache_key) DO NOTHING",
        )
        .bind(namespaced(scope, &cache_key))
        .bind(i32::from(record.status))
        .bind(&record.headers)
        .bind(&record.body)
        .bind(&record.fingerprint)
        .bind(&record.expires_at)
        .bind(expires_at_ms(&record.expires_at))
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(())
    }

    async fn evict_expired(&self) -> Result<u64, StorageError> {
        // Sweep on `expires_at_ms` (epoch-millis `BIGINT`) against a
        // Rust-supplied `now` — identical predicate and clock source to
        // the SQLite backend, so eviction fires at the same instant on
        // both dialects.
        let now_ms = Utc::now().timestamp_millis();
        let res = sqlx::query("DELETE FROM port_idempotency_cache WHERE expires_at_ms <= $1")
            .bind(now_ms)
            .execute(&self.pool)
            .await
            .map_err(conn_err)?;
        Ok(res.rows_affected())
    }
}

/// Postgres-backed webhook-activation store.
#[derive(Clone, Debug)]
pub struct PgWebhookActivationStore {
    pool: PgPool,
}

impl PgWebhookActivationStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl WebhookActivationStore for PgWebhookActivationStore {
    async fn upsert(
        &self,
        scope: &Scope,
        record: WebhookActivationRecord,
    ) -> Result<(), StorageError> {
        let mode_str = match record.mode {
            WebhookMode::Prod => "prod",
            _ => "test",
        };
        sqlx::query(
            "INSERT INTO port_webhook_activations \
             (workspace_id, org_id, slug, trigger_id, active, \
              workflow_id, webhook_mode, token_hash) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (workspace_id, org_id, slug) DO UPDATE SET \
               trigger_id   = EXCLUDED.trigger_id, \
               active       = EXCLUDED.active, \
               workflow_id  = EXCLUDED.workflow_id, \
               webhook_mode = EXCLUDED.webhook_mode, \
               token_hash   = EXCLUDED.token_hash",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&record.slug)
        .bind(&record.trigger_id)
        .bind(record.active)
        .bind(&record.workflow_id)
        .bind(mode_str)
        .bind(record.token_hash.as_ref())
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(())
    }

    async fn resolve(
        &self,
        scope: &Scope,
        slug: &str,
    ) -> Result<Option<WebhookActivationRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT trigger_id, workflow_id, webhook_mode, token_hash \
             FROM port_webhook_activations \
             WHERE workspace_id = $1 AND org_id = $2 AND slug = $3 \
               AND active = TRUE",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        row.map(|r| {
            // Fail-closed: any unrecognised mode text defaults to Test.
            let mode = match r
                .try_get::<Option<String>, _>("webhook_mode")
                .ok()
                .flatten()
                .as_deref()
            {
                Some("prod") => WebhookMode::Prod,
                _ => WebhookMode::Test,
            };
            // Fail-closed: a malformed or short blob defaults to the
            // all-zeros sentinel (no token assigned yet).
            let token_hash: [u8; 32] = r
                .try_get::<Vec<u8>, _>("token_hash")
                .ok()
                .and_then(|v| v.try_into().ok())
                .unwrap_or([0u8; 32]);
            let trigger_id: String = r.try_get("trigger_id").map_err(conn_err)?;
            let workflow_id: Option<String> = r.try_get("workflow_id").map_err(conn_err)?;
            // `WebhookActivationRecord` is `#[non_exhaustive]`; construct
            // via the public constructor then overwrite the non-default
            // fields through their public field accessors.
            let mut rec = WebhookActivationRecord::new(trigger_id, scope.clone(), slug, true);
            rec.workflow_id = workflow_id;
            rec.mode = mode;
            rec.token_hash = token_hash;
            Ok(rec)
        })
        .transpose()
    }

    async fn deactivate(&self, scope: &Scope, trigger_id: &str) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE port_webhook_activations SET active = FALSE \
             WHERE workspace_id = $1 AND org_id = $2 AND trigger_id = $3",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(trigger_id)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(())
    }

    /// SYSTEM-SURFACE: scope comes out of the returned row, not in.
    /// Rejects the all-zeros sentinel before querying (see trait doc).
    async fn resolve_by_token(
        &self,
        token_hash: &[u8; 32],
    ) -> Result<Option<WebhookActivationRecord>, StorageError> {
        // Sentinel guard: all-zeros means "no token assigned"; never query.
        if token_hash == &[0u8; 32] {
            return Ok(None);
        }
        let row = sqlx::query(
            "SELECT workspace_id, org_id, slug, trigger_id, workflow_id, \
                    webhook_mode, token_hash \
             FROM port_webhook_activations \
             WHERE token_hash = $1 AND active = TRUE",
        )
        .bind(token_hash.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        let Some(r) = row else { return Ok(None) };
        let scope = Scope::new(
            r.try_get::<String, _>("workspace_id").map_err(conn_err)?,
            r.try_get::<String, _>("org_id").map_err(conn_err)?,
        );
        let slug: String = r.try_get("slug").map_err(conn_err)?;
        let trigger_id: String = r.try_get("trigger_id").map_err(conn_err)?;
        let workflow_id: Option<String> = r.try_get("workflow_id").map_err(conn_err)?;
        // Fail-closed: unrecognised mode → Test.
        let mode = match r
            .try_get::<Option<String>, _>("webhook_mode")
            .ok()
            .flatten()
            .as_deref()
        {
            Some("prod") => WebhookMode::Prod,
            _ => WebhookMode::Test,
        };
        // Fail-closed: malformed or short blob → zero sentinel.
        let stored_hash: [u8; 32] = r
            .try_get::<Vec<u8>, _>("token_hash")
            .ok()
            .and_then(|v| v.try_into().ok())
            .unwrap_or([0u8; 32]);
        let mut rec = WebhookActivationRecord::new(trigger_id, scope, slug, true);
        rec.workflow_id = workflow_id;
        rec.mode = mode;
        rec.token_hash = stored_hash;
        Ok(Some(rec))
    }

    /// SYSTEM-SURFACE: cross-tenant enumeration for bootstrap map population.
    async fn list_all_active(&self) -> Result<Vec<WebhookActivationRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT workspace_id, org_id, slug, trigger_id, workflow_id, \
                    webhook_mode, token_hash \
             FROM port_webhook_activations \
             WHERE active = TRUE",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let scope = Scope::new(
                r.try_get::<String, _>("workspace_id").map_err(conn_err)?,
                r.try_get::<String, _>("org_id").map_err(conn_err)?,
            );
            let slug: String = r.try_get("slug").map_err(conn_err)?;
            let trigger_id: String = r.try_get("trigger_id").map_err(conn_err)?;
            let workflow_id: Option<String> = r.try_get("workflow_id").map_err(conn_err)?;
            let mode = match r
                .try_get::<Option<String>, _>("webhook_mode")
                .ok()
                .flatten()
                .as_deref()
            {
                Some("prod") => WebhookMode::Prod,
                _ => WebhookMode::Test,
            };
            let token_hash: [u8; 32] = r
                .try_get::<Vec<u8>, _>("token_hash")
                .ok()
                .and_then(|v| v.try_into().ok())
                .unwrap_or([0u8; 32]);
            let mut rec = WebhookActivationRecord::new(trigger_id, scope, slug, true);
            rec.workflow_id = workflow_id;
            rec.mode = mode;
            rec.token_hash = token_hash;
            out.push(rec);
        }
        Ok(out)
    }
}
