//! PostgreSQL iteration-checkpoint store over ordered migration 0062.
//!
//! A save locks the execution row (`SELECT … FOR UPDATE`, the fence the
//! operation ledger shares) before it reads the stored checkpoint, so every
//! save of one execution is serialized behind its owner's row and a lease
//! takeover cannot slip between the fence and the write. A failure before
//! commit is [`IterationCheckpointError::Unavailable`] (nothing was written);
//! a failed commit is [`IterationCheckpointError::AcknowledgementUnknown`].
//!
//! State bytes never cross into an error or a span.

use nebula_storage_port::store::CheckpointStore;
use nebula_storage_port::{
    CheckpointSaved, FencingToken, IterationCheckpoint, IterationCheckpointError,
    IterationCheckpointKey,
};
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::iteration_checkpoint::{
    SaveDecision, decide_save, durable_integer, load_label, require_stored_version, save_label,
    stored_checkpoint,
};

/// PostgreSQL-backed fenced iteration-checkpoint store.
///
/// Wrap a pool whose schema was installed via [`super::init_schema`].
#[derive(Clone, Debug)]
pub struct PgCheckpointStore {
    pool: PgPool,
}

impl PgCheckpointStore {
    /// Wrap an existing pool. The caller installs the port schema (see
    /// [`super::init_schema`]).
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// A driver failure reached before commit definitely did not commit.
fn unavailable(_error: sqlx::Error) -> IterationCheckpointError {
    IterationCheckpointError::Unavailable
}

/// A failed commit leaves the caller unable to prove whether the write landed.
fn acknowledgement_unknown(_error: sqlx::Error) -> IterationCheckpointError {
    IterationCheckpointError::AcknowledgementUnknown
}

async fn load(
    pool: &PgPool,
    key: &IterationCheckpointKey<'_>,
) -> Result<Option<IterationCheckpoint>, IterationCheckpointError> {
    let row = sqlx::query(
        "SELECT action_version, iteration::bigint AS iteration, state, state_digest, \
                resume_delay_ms, attested_positions::bigint AS attested_positions, \
                attempt_generation, fencing_generation, written_at_ms \
         FROM port_iteration_checkpoints \
         WHERE workspace_id = $1 AND org_id = $2 AND execution_id = $3 \
           AND node_key = $4 AND action_key = $5 AND action_version_digest = $6",
    )
    .bind(key.scope().workspace_id.as_str())
    .bind(key.scope().org_id.as_str())
    .bind(key.execution_id())
    .bind(key.node_key())
    .bind(key.action_key())
    .bind(key.action_version_digest().as_slice())
    .fetch_optional(pool)
    .await
    .map_err(unavailable)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let corrupt = |_error: sqlx::Error| IterationCheckpointError::InvalidRecord;
    require_stored_version(
        &row.try_get::<String, _>("action_version")
            .map_err(corrupt)?,
        key,
    )?;
    stored_checkpoint(
        row.try_get("iteration").map_err(corrupt)?,
        row.try_get("state").map_err(corrupt)?,
        row.try_get("state_digest").map_err(corrupt)?,
        row.try_get("resume_delay_ms").map_err(corrupt)?,
        row.try_get("attested_positions").map_err(corrupt)?,
        row.try_get("attempt_generation").map_err(corrupt)?,
        row.try_get("fencing_generation").map_err(corrupt)?,
        row.try_get("written_at_ms").map_err(corrupt)?,
    )
    .map(Some)
}

async fn stored_identity(
    tx: &mut Transaction<'_, Postgres>,
    key: &IterationCheckpointKey<'_>,
) -> Result<Option<(u32, [u8; 32])>, IterationCheckpointError> {
    let row = sqlx::query(
        "SELECT action_version, iteration::bigint AS iteration, state_digest \
         FROM port_iteration_checkpoints \
         WHERE workspace_id = $1 AND org_id = $2 AND execution_id = $3 \
           AND node_key = $4 AND action_key = $5 AND action_version_digest = $6 \
         FOR UPDATE",
    )
    .bind(key.scope().workspace_id.as_str())
    .bind(key.scope().org_id.as_str())
    .bind(key.execution_id())
    .bind(key.node_key())
    .bind(key.action_key())
    .bind(key.action_version_digest().as_slice())
    .fetch_optional(&mut **tx)
    .await
    .map_err(unavailable)?;
    row.map(|row| {
        let corrupt = |_error| IterationCheckpointError::InvalidRecord;
        let version: String = row.try_get("action_version").map_err(corrupt)?;
        require_stored_version(&version, key)?;
        let iteration: i64 = row.try_get("iteration").map_err(corrupt)?;
        let digest: Vec<u8> = row.try_get("state_digest").map_err(corrupt)?;
        Ok((
            u32::try_from(iteration).map_err(|_| IterationCheckpointError::InvalidRecord)?,
            <[u8; 32]>::try_from(digest).map_err(|_| IterationCheckpointError::InvalidRecord)?,
        ))
    })
    .transpose()
}

async fn save(
    pool: &PgPool,
    key: &IterationCheckpointKey<'_>,
    checkpoint: &IterationCheckpoint,
    fencing: FencingToken,
) -> Result<CheckpointSaved, IterationCheckpointError> {
    let mut tx = pool.begin().await.map_err(unavailable)?;
    super::execution_fence::lock_execution(&mut tx, key.scope(), key.execution_id(), Some(fencing))
        .await?;
    let stored = stored_identity(&mut tx, key).await?;
    let decision = decide_save(
        stored
            .as_ref()
            .map(|(iteration, digest)| (*iteration, digest)),
        checkpoint,
    )?;
    if decision == SaveDecision::AlreadyRecorded {
        // Nothing was written; the commit only releases the row locks.
        drop(tx.commit().await);
        return Ok(decision.saved());
    }
    let delay = checkpoint
        .resume_delay_ms()
        .map(durable_integer)
        .transpose()?;
    sqlx::query(
        "INSERT INTO port_iteration_checkpoints \
         (workspace_id, org_id, execution_id, node_key, action_key, action_version, \
          action_version_digest, iteration, state, state_digest, resume_delay_ms, \
          attested_positions, attempt_generation, fencing_generation, written_at_ms) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, \
                 (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint) \
         ON CONFLICT (workspace_id, org_id, execution_id, node_key, action_key, \
                      action_version_digest) \
         DO UPDATE SET iteration = excluded.iteration, state = excluded.state, \
           state_digest = excluded.state_digest, resume_delay_ms = excluded.resume_delay_ms, \
           attested_positions = excluded.attested_positions, \
           attempt_generation = excluded.attempt_generation, \
           fencing_generation = excluded.fencing_generation, \
           written_at_ms = excluded.written_at_ms",
    )
    .bind(key.scope().workspace_id.as_str())
    .bind(key.scope().org_id.as_str())
    .bind(key.execution_id())
    .bind(key.node_key())
    .bind(key.action_key())
    .bind(key.action_version())
    .bind(key.action_version_digest().as_slice())
    .bind(
        i32::try_from(checkpoint.iteration())
            .map_err(|_| IterationCheckpointError::InvalidRecord)?,
    )
    .bind(checkpoint.state())
    .bind(checkpoint.state_digest().as_slice())
    .bind(delay)
    .bind(
        i32::try_from(checkpoint.attested_positions())
            .map_err(|_| IterationCheckpointError::InvalidRecord)?,
    )
    .bind(durable_integer(checkpoint.attempt_generation())?)
    .bind(durable_integer(fencing.generation())?)
    .execute(&mut *tx)
    .await
    .map_err(unavailable)?;
    tx.commit().await.map_err(acknowledgement_unknown)?;
    Ok(decision.saved())
}

#[async_trait::async_trait]
impl CheckpointStore for PgCheckpointStore {
    #[tracing::instrument(
        level = "debug",
        name = "iteration_checkpoint.load",
        skip_all,
        fields(backend = "postgres", outcome = tracing::field::Empty)
    )]
    async fn load_iteration_checkpoint(
        &self,
        key: &IterationCheckpointKey<'_>,
    ) -> Result<Option<IterationCheckpoint>, IterationCheckpointError> {
        let result = load(&self.pool, key).await;
        tracing::Span::current().record("outcome", load_label(&result));
        result
    }

    #[tracing::instrument(
        level = "debug",
        name = "iteration_checkpoint.save",
        skip_all,
        fields(backend = "postgres", iteration = checkpoint.iteration(), outcome = tracing::field::Empty)
    )]
    async fn save_iteration_checkpoint(
        &self,
        key: &IterationCheckpointKey<'_>,
        checkpoint: &IterationCheckpoint,
        fencing: FencingToken,
    ) -> Result<CheckpointSaved, IterationCheckpointError> {
        let result = save(&self.pool, key, checkpoint, fencing).await;
        tracing::Span::current().record("outcome", save_label(result));
        result
    }
}
