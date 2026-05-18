//! Postgres implementation of [`WebhookActivationRepo`].
//!
//! Schema lives in migrations 0010 (`triggers`) and 0018
//! (`webhook_path` indexed column). Migration 0025 attaches the
//! kind-namespaced JSONB contract as `COMMENT ON COLUMN
//! triggers.config`.
//!
//! # Query shape
//!
//! `list_active` joins
//! ```text
//! triggers ↔ workspaces ↔ orgs
//! ```
//! and filters on `kind = 'webhook' AND state = 'active' AND
//! webhook_path IS NOT NULL AND deleted_at IS NULL`. The
//! `webhook_path` column is partial-indexed (idx_triggers_webhook_path)
//! so `find_by_webhook_path` is `O(log n)` on the index, joined
//! against `workspaces` and `orgs` for the slug projection.
//!
//! # Decode-failure policy
//!
//! `list_active` is best-effort: a single row whose
//! `webhook_activation` JSONB fails to decode is **logged at
//! `warn`** with the trigger ID and skipped, so the rest of the
//! bootstrap proceeds. `find_by_webhook_path` is **strict**: a
//! decode failure for the requested row is a hard `Err` because the
//! caller wants this exact slug.

use async_trait::async_trait;
use sqlx::{Pool, Postgres, Row};

use crate::error::StorageError;
use crate::pg::map_db_err;
use crate::repos::WebhookActivationRepo;
use crate::repos::webhook_activation::{decode_err, record_from_parts};
use crate::rows::{WebhookActivationRecord, WebhookActivationSpec};

/// Postgres-backed webhook-activation repository.
///
/// Holds an `sqlx::Pool<Postgres>` and projects rows directly into
/// [`WebhookActivationRecord`]. Stateless beyond the pool — clone it
/// freely.
#[derive(Clone, Debug)]
pub struct PgWebhookActivationRepo {
    pool: Pool<Postgres>,
}

impl PgWebhookActivationRepo {
    /// Wrap an existing pool. Pool lifetime / sizing is the
    /// composition root's responsibility.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }
}

const LIST_ACTIVE_SQL: &str = "\
SELECT t.id AS trigger_id, \
       o.slug AS org_slug, \
       w.slug AS workspace_slug, \
       t.slug AS trigger_slug, \
       t.config AS config, \
       t.webhook_path AS webhook_path \
  FROM triggers AS t \
  JOIN workspaces AS w ON w.id = t.workspace_id AND w.deleted_at IS NULL \
  JOIN orgs       AS o ON o.id = w.org_id        AND o.deleted_at IS NULL \
 WHERE t.kind = 'webhook' \
   AND t.state = 'active' \
   AND t.webhook_path IS NOT NULL \
   AND t.deleted_at IS NULL";

const FIND_BY_PATH_SQL: &str = "\
SELECT t.id AS trigger_id, \
       o.slug AS org_slug, \
       w.slug AS workspace_slug, \
       t.slug AS trigger_slug, \
       t.config AS config \
  FROM triggers AS t \
  JOIN workspaces AS w ON w.id = t.workspace_id AND w.deleted_at IS NULL \
  JOIN orgs       AS o ON o.id = w.org_id        AND o.deleted_at IS NULL \
 WHERE t.webhook_path = $1 \
   AND t.kind = 'webhook' \
   AND t.state = 'active' \
   AND t.deleted_at IS NULL \
 LIMIT 1";

#[async_trait]
impl WebhookActivationRepo for PgWebhookActivationRepo {
    async fn list_active(&self) -> Result<Vec<WebhookActivationRecord>, StorageError> {
        tracing::debug!(
            target: "nebula::storage::webhook",
            "loading active webhook activations from postgres"
        );

        let rows = sqlx::query(LIST_ACTIVE_SQL)
            .fetch_all(&self.pool)
            .await
            .map_err(|err| map_db_err("webhook_activations", err))?;

        let mut records = Vec::with_capacity(rows.len());
        let mut decode_failures = 0usize;
        for row in rows {
            let trigger_id: Vec<u8> = row.try_get("trigger_id").map_err(serialization)?;
            let org_slug: String = row.try_get("org_slug").map_err(serialization)?;
            let workspace_slug: String = row.try_get("workspace_slug").map_err(serialization)?;
            let trigger_slug: String = row.try_get("trigger_slug").map_err(serialization)?;
            let config: serde_json::Value = row.try_get("config").map_err(serialization)?;

            match WebhookActivationSpec::from_trigger_config(&config) {
                Ok(Some(spec)) => {
                    tracing::debug!(
                        target: "nebula::storage::webhook",
                        trigger_id = %hex(&trigger_id),
                        org = %org_slug,
                        workspace = %workspace_slug,
                        trigger_slug = %trigger_slug,
                        action_kind = %spec.action_kind,
                        "decoded webhook activation spec",
                    );
                    records.push(record_from_parts(
                        trigger_id,
                        org_slug,
                        workspace_slug,
                        trigger_slug,
                        spec,
                    ));
                },
                Ok(None) => {
                    decode_failures += 1;
                    tracing::warn!(
                        target: "nebula::storage::webhook",
                        trigger_id = %hex(&trigger_id),
                        org = %org_slug,
                        workspace = %workspace_slug,
                        "trigger marked kind='webhook' but config has no webhook_activation key; skipping",
                    );
                },
                Err(err) => {
                    decode_failures += 1;
                    tracing::warn!(
                        target: "nebula::storage::webhook",
                        trigger_id = %hex(&trigger_id),
                        org = %org_slug,
                        workspace = %workspace_slug,
                        error = %err,
                        "webhook activation spec decode failed; skipping row",
                    );
                },
            }
        }

        tracing::info!(
            target: "nebula::storage::webhook",
            count = records.len(),
            skipped = decode_failures,
            "loaded active webhook activations",
        );
        Ok(records)
    }

    async fn find_by_webhook_path(
        &self,
        path: &str,
    ) -> Result<Option<WebhookActivationRecord>, StorageError> {
        tracing::debug!(
            target: "nebula::storage::webhook",
            path = %path,
            "find_by_webhook_path",
        );

        let row = sqlx::query(FIND_BY_PATH_SQL)
            .bind(path)
            .fetch_optional(&self.pool)
            .await
            .map_err(|err| map_db_err("webhook_activations", err))?;

        let Some(row) = row else {
            return Ok(None);
        };

        let trigger_id: Vec<u8> = row.try_get("trigger_id").map_err(serialization)?;
        let org_slug: String = row.try_get("org_slug").map_err(serialization)?;
        let workspace_slug: String = row.try_get("workspace_slug").map_err(serialization)?;
        let trigger_slug: String = row.try_get("trigger_slug").map_err(serialization)?;
        let config: serde_json::Value = row.try_get("config").map_err(serialization)?;

        match WebhookActivationSpec::from_trigger_config(&config) {
            Ok(Some(spec)) => Ok(Some(record_from_parts(
                trigger_id,
                org_slug,
                workspace_slug,
                trigger_slug,
                spec,
            ))),
            Ok(None) => Err(StorageError::Serialization(format!(
                "trigger {} matched webhook_path={path} but has no webhook_activation key",
                hex(&trigger_id)
            ))),
            Err(err) => Err(decode_err(&trigger_id, err)),
        }
    }
}

fn serialization<E: std::fmt::Display>(err: E) -> StorageError {
    StorageError::Serialization(format!("webhook_activations row decode failed: {err}"))
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(s, "{byte:02x}");
    }
    s
}
