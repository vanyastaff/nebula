//! Postgres `IdempotencyStore` (ADR-0048 durable-replay cache) +
//! `WebhookActivationStore` (ADR-0049) over the port-scoped schema.
//!
//! The cache is keyed by the caller-supplied, already-scope-namespaced
//! `cache_key` (first-writer-wins via INSERT ... ON CONFLICT DO NOTHING).
//! Webhook activations are keyed by `(workspace, org, slug)` so resolution
//! never crosses a tenant boundary.

use std::time::Duration;

use chrono::{DateTime, Utc};
use nebula_storage_port::dto::{CachedRecord, WebhookActivationRecord};
use nebula_storage_port::store::{IdempotencyStore, WebhookActivationStore};
use nebula_storage_port::{Scope, StorageError};
use sqlx::{PgPool, Row};

use super::execution::conn_err;

/// Parse an RFC 3339 expiry. A malformed timestamp maps to the unix epoch
/// (fail-closed: an unparsable expiry is treated as already-expired so a
/// record we cannot prove fresh is never served and is swept promptly).
fn parse_expiry(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
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
    async fn get(&self, cache_key: &str) -> Result<Option<CachedRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT status, headers, body, fingerprint, expires_at \
             FROM port_idempotency_cache WHERE cache_key = $1",
        )
        .bind(cache_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let exp: DateTime<Utc> = row.try_get("expires_at").map_err(conn_err)?;
        Ok(Some(CachedRecord {
            status: row.try_get::<i32, _>("status").map_err(conn_err)? as u16,
            headers: row.try_get("headers").map_err(conn_err)?,
            body: row.try_get("body").map_err(conn_err)?,
            fingerprint: row.try_get("fingerprint").map_err(conn_err)?,
            expires_at: exp.to_rfc3339(),
        }))
    }

    async fn put(
        &self,
        cache_key: String,
        record: CachedRecord,
        _ttl: Duration,
    ) -> Result<(), StorageError> {
        // First-writer-wins: ON CONFLICT DO NOTHING keeps the original
        // record (and its fingerprint) on a replay race.
        sqlx::query(
            "INSERT INTO port_idempotency_cache \
             (cache_key, status, headers, body, fingerprint, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (cache_key) DO NOTHING",
        )
        .bind(&cache_key)
        .bind(i32::from(record.status))
        .bind(&record.headers)
        .bind(&record.body)
        .bind(&record.fingerprint)
        .bind(parse_expiry(&record.expires_at))
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(())
    }

    async fn evict_expired(&self) -> Result<u64, StorageError> {
        let res = sqlx::query("DELETE FROM port_idempotency_cache WHERE expires_at <= now()")
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
        sqlx::query(
            "INSERT INTO port_webhook_activations \
             (workspace_id, org_id, slug, trigger_id, active) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (workspace_id, org_id, slug) DO UPDATE SET \
               trigger_id = EXCLUDED.trigger_id, active = EXCLUDED.active",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&record.slug)
        .bind(&record.trigger_id)
        .bind(record.active)
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
            "SELECT trigger_id FROM port_webhook_activations \
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
            Ok(WebhookActivationRecord {
                trigger_id: r.try_get("trigger_id").map_err(conn_err)?,
                scope: scope.clone(),
                slug: slug.to_string(),
                active: true,
            })
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
}
