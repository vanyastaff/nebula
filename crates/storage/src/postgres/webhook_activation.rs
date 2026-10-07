//! Postgres `WebhookActivationStore` over `webhook_activations`: activations
//! keyed by `(org, workspace, slug)` so resolution never crosses a tenant
//! boundary.
//!
//! An activation belongs to the trigger it was built from (`spec_trigger_id`)
//! and dispatches into its workflow; both are foreign keys, and an upsert
//! naming either checks it is live in the same transaction. The port's
//! all-zeros token hash ("no token assigned") is stored as `NULL`.

use nebula_storage_port::dto::{WebhookActivationRecord, WebhookMode};
use nebula_storage_port::store::WebhookActivationStore;
use nebula_storage_port::{Scope, StorageError};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

use crate::sql_error::storage_error;

/// The port's "no token assigned" token hash.
const NO_TOKEN: [u8; 32] = [0u8; 32];

/// Columns [`decode_activation`] reads.
const ACTIVATION_COLUMNS: &str = "org_id, workspace_id, slug, trigger_id, spec_trigger_id, \
     workflow_id, active, webhook_mode, token_hash";

/// An activation routes only while it is active and the trigger and workflow
/// it names are live: archiving either parent stops routing without
/// rewriting the activation row.
const ROUTABLE: &str = "webhook_activations.active \
     AND (webhook_activations.spec_trigger_id IS NULL OR EXISTS ( \
         SELECT 1 FROM triggers t \
         WHERE t.org_id = webhook_activations.org_id \
           AND t.workspace_id = webhook_activations.workspace_id \
           AND t.id = webhook_activations.spec_trigger_id AND t.deleted_at IS NULL)) \
     AND (webhook_activations.workflow_id IS NULL OR EXISTS ( \
         SELECT 1 FROM workflows w \
         WHERE w.org_id = webhook_activations.org_id \
           AND w.workspace_id = webhook_activations.workspace_id \
           AND w.id = webhook_activations.workflow_id AND w.deleted_at IS NULL))";

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

fn encode_mode(mode: WebhookMode) -> &'static str {
    match mode {
        WebhookMode::Prod => "prod",
        _ => "test",
    }
}

fn decode_activation(row: &PgRow) -> Result<WebhookActivationRecord, StorageError> {
    let scope = Scope::new(
        row.try_get::<String, _>("workspace_id")
            .map_err(storage_error)?,
        row.try_get::<String, _>("org_id").map_err(storage_error)?,
    );
    let mode = match row
        .try_get::<String, _>("webhook_mode")
        .map_err(storage_error)?
        .as_str()
    {
        "prod" => WebhookMode::Prod,
        "test" => WebhookMode::Test,
        _ => {
            return Err(StorageError::Corrupt(
                "column `webhook_mode` holds an unknown mode".into(),
            ));
        },
    };
    let token_hash = match row
        .try_get::<Option<Vec<u8>>, _>("token_hash")
        .map_err(storage_error)?
    {
        None => NO_TOKEN,
        Some(bytes) => <[u8; 32]>::try_from(bytes.as_slice())
            .map_err(|_| StorageError::Corrupt("column `token_hash` is not 32 bytes".into()))?,
    };
    let mut record = WebhookActivationRecord::new(
        row.try_get::<String, _>("trigger_id")
            .map_err(storage_error)?,
        scope,
        row.try_get::<String, _>("slug").map_err(storage_error)?,
        row.try_get::<bool, _>("active").map_err(storage_error)?,
    );
    // `WebhookActivationRecord` is `#[non_exhaustive]`: construct it, then
    // set the fields the constructor defaults.
    record.workflow_id = row.try_get("workflow_id").map_err(storage_error)?;
    record.mode = mode;
    record.token_hash = token_hash;
    record.spec_trigger_id = row.try_get("spec_trigger_id").map_err(storage_error)?;
    Ok(record)
}

/// Share-lock the live row `id` of `table` in `scope`, so a concurrent soft
/// delete serializes with the write; a missing or deleted row is `NotFound`.
async fn lock_live_parent(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    table: &'static str,
    entity: &'static str,
    scope: &Scope,
    id: &str,
) -> Result<(), StorageError> {
    // `table` is one of two internal literals, never input.
    let sql = format!(
        "SELECT 1 FROM {table} \
         WHERE org_id = $1 AND workspace_id = $2 AND id = $3 AND deleted_at IS NULL \
         FOR SHARE"
    );
    let live: Option<i32> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(storage_error)?;
    live.map(|_| ())
        .ok_or_else(|| StorageError::not_found(entity, id))
}

#[async_trait::async_trait]
impl WebhookActivationStore for PgWebhookActivationStore {
    async fn upsert(
        &self,
        scope: &Scope,
        record: WebhookActivationRecord,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        if let Some(trigger_id) = &record.spec_trigger_id {
            lock_live_parent(&mut tx, "triggers", "trigger", scope, trigger_id).await?;
        }
        if let Some(workflow_id) = &record.workflow_id {
            lock_live_parent(&mut tx, "workflows", "workflow", scope, workflow_id).await?;
        }
        let token_hash = (record.token_hash != NO_TOKEN).then_some(record.token_hash.as_slice());
        sqlx::query(
            "INSERT INTO webhook_activations \
             (org_id, workspace_id, slug, trigger_id, spec_trigger_id, \
              workflow_id, active, webhook_mode, token_hash) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (org_id, workspace_id, slug) DO UPDATE SET \
               trigger_id      = EXCLUDED.trigger_id, \
               spec_trigger_id = EXCLUDED.spec_trigger_id, \
               workflow_id     = EXCLUDED.workflow_id, \
               active          = EXCLUDED.active, \
               webhook_mode    = EXCLUDED.webhook_mode, \
               token_hash      = EXCLUDED.token_hash",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(&record.slug)
        .bind(&record.trigger_id)
        .bind(&record.spec_trigger_id)
        .bind(&record.workflow_id)
        .bind(record.active)
        .bind(encode_mode(record.mode))
        .bind(token_hash)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;
        Ok(())
    }

    async fn resolve(
        &self,
        scope: &Scope,
        slug: &str,
    ) -> Result<Option<WebhookActivationRecord>, StorageError> {
        let sql = format!(
            "SELECT {ACTIVATION_COLUMNS} FROM webhook_activations \
             WHERE org_id = $1 AND workspace_id = $2 AND slug = $3 AND {ROUTABLE}"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&scope.org_id)
            .bind(&scope.workspace_id)
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage_error)?
            .as_ref()
            .map(decode_activation)
            .transpose()
    }

    async fn deactivate(&self, scope: &Scope, trigger_id: &str) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE webhook_activations SET active = FALSE \
             WHERE org_id = $1 AND workspace_id = $2 AND trigger_id = $3",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
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
        // All-zeros means "no token assigned": no row is stored under it.
        if token_hash == &NO_TOKEN {
            return Ok(None);
        }
        let sql = format!(
            "SELECT {ACTIVATION_COLUMNS} FROM webhook_activations \
             WHERE token_hash = $1 AND {ROUTABLE}"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(token_hash.as_slice())
            .fetch_optional(&self.pool)
            .await
            .map_err(storage_error)?
            .as_ref()
            .map(decode_activation)
            .transpose()
    }

    /// SYSTEM-SURFACE: cross-tenant enumeration for bootstrap map population.
    async fn list_all_active(&self) -> Result<Vec<WebhookActivationRecord>, StorageError> {
        let sql = format!(
            "SELECT {ACTIVATION_COLUMNS} FROM webhook_activations WHERE {ROUTABLE} \
             ORDER BY org_id COLLATE \"C\", workspace_id COLLATE \"C\", slug COLLATE \"C\""
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .fetch_all(&self.pool)
            .await
            .map_err(storage_error)?
            .iter()
            .map(decode_activation)
            .collect()
    }
}
