//! SQLite `WebhookActivationStore`: activations keyed by
//! `(workspace, org, slug)` so resolution never crosses a tenant boundary.

use nebula_storage_port::dto::{WebhookActivationRecord, WebhookMode};
use nebula_storage_port::store::WebhookActivationStore;
use nebula_storage_port::{Scope, StorageError};
use sqlx::{Row, SqlitePool};

use crate::sql_error::storage_error;

/// SQLite-backed webhook-activation store.
#[derive(Clone, Debug)]
pub struct SqliteWebhookActivationStore {
    pool: SqlitePool,
}

impl SqliteWebhookActivationStore {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl WebhookActivationStore for SqliteWebhookActivationStore {
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
              workflow_id, webhook_mode, token_hash, spec_trigger_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (workspace_id, org_id, slug) DO UPDATE SET \
               trigger_id      = excluded.trigger_id, \
               active          = excluded.active, \
               workflow_id     = excluded.workflow_id, \
               webhook_mode    = excluded.webhook_mode, \
               token_hash      = excluded.token_hash, \
               spec_trigger_id = excluded.spec_trigger_id",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(&record.slug)
        .bind(&record.trigger_id)
        .bind(i64::from(record.active))
        .bind(&record.workflow_id)
        .bind(mode_str)
        .bind(record.token_hash.as_ref())
        .bind(&record.spec_trigger_id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    async fn resolve(
        &self,
        scope: &Scope,
        slug: &str,
    ) -> Result<Option<WebhookActivationRecord>, StorageError> {
        // Only an active activation resolves (never route a paused hook),
        // and only within this tenant's scope.
        let row = sqlx::query(
            "SELECT trigger_id, workflow_id, webhook_mode, token_hash, spec_trigger_id \
             FROM port_webhook_activations \
             WHERE workspace_id = ? AND org_id = ? AND slug = ? \
               AND active = 1",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        // Use `transpose()` so column-decode errors inside the closure
        // surface as `Err(StorageError)` instead of being silently swallowed
        // by `unwrap_or_default` / `unwrap_or(None)` — aligns with the
        // Postgres backend that propagates via `.map_err(storage_error)?`.
        row.map(|r| -> Result<WebhookActivationRecord, StorageError> {
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
            let trigger_id: String = r.try_get("trigger_id").map_err(storage_error)?;
            let workflow_id: Option<String> = r.try_get("workflow_id").map_err(storage_error)?;
            let spec_trigger_id: Option<String> =
                r.try_get("spec_trigger_id").map_err(storage_error)?;
            // `WebhookActivationRecord` is `#[non_exhaustive]`; construct
            // via the public constructor then overwrite the non-default
            // fields through their public field accessors.
            let mut rec = WebhookActivationRecord::new(trigger_id, scope.clone(), slug, true);
            rec.workflow_id = workflow_id;
            rec.mode = mode;
            rec.token_hash = token_hash;
            rec.spec_trigger_id = spec_trigger_id;
            Ok(rec)
        })
        .transpose()
    }

    async fn deactivate(&self, scope: &Scope, trigger_id: &str) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE port_webhook_activations SET active = 0 \
             WHERE workspace_id = ? AND org_id = ? AND trigger_id = ?",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(trigger_id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
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
                    webhook_mode, token_hash, spec_trigger_id \
             FROM port_webhook_activations \
             WHERE token_hash = ? AND active = 1",
        )
        .bind(token_hash.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        let Some(r) = row else { return Ok(None) };
        let scope = Scope::new(
            r.try_get::<String, _>("workspace_id")
                .map_err(storage_error)?,
            r.try_get::<String, _>("org_id").map_err(storage_error)?,
        );
        let slug: String = r.try_get("slug").map_err(storage_error)?;
        let trigger_id: String = r.try_get("trigger_id").map_err(storage_error)?;
        let workflow_id: Option<String> = r.try_get("workflow_id").map_err(storage_error)?;
        let spec_trigger_id: Option<String> =
            r.try_get("spec_trigger_id").map_err(storage_error)?;
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
        rec.spec_trigger_id = spec_trigger_id;
        Ok(Some(rec))
    }

    /// SYSTEM-SURFACE: cross-tenant enumeration for bootstrap map population.
    async fn list_all_active(&self) -> Result<Vec<WebhookActivationRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT workspace_id, org_id, slug, trigger_id, workflow_id, \
                    webhook_mode, token_hash, spec_trigger_id \
             FROM port_webhook_activations \
             WHERE active = 1",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let scope = Scope::new(
                r.try_get::<String, _>("workspace_id")
                    .map_err(storage_error)?,
                r.try_get::<String, _>("org_id").map_err(storage_error)?,
            );
            let slug: String = r.try_get("slug").map_err(storage_error)?;
            let trigger_id: String = r.try_get("trigger_id").map_err(storage_error)?;
            let workflow_id: Option<String> = r.try_get("workflow_id").map_err(storage_error)?;
            let spec_trigger_id: Option<String> =
                r.try_get("spec_trigger_id").map_err(storage_error)?;
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
            rec.spec_trigger_id = spec_trigger_id;
            out.push(rec);
        }
        Ok(out)
    }
}
