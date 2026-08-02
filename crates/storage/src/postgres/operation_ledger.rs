//! PostgreSQL operation ledger over ordered migration 0045.
//!
//! Prepare relies on the natural-key unique index rather than on a lock: the
//! slot a caller wants may not exist yet, and `SELECT … FOR UPDATE` cannot lock
//! a row that is absent, so two workers preparing the same slot would both read
//! "not prepared". The insert is therefore `ON CONFLICT DO NOTHING`, and a
//! loser re-reads the winner's row inside the same transaction — so both
//! callers leave with one operation identity instead of one of them losing on
//! the index.
//!
//! Mutating paths lock the row they decide on with `FOR UPDATE`, so a fenced
//! commit cannot be invalidated between its decision and its write.
//!
//! Driver detail and request payloads never cross the port boundary. A failure
//! before commit is [`OperationLedgerError::Unavailable`] (the operation
//! definitely did not commit); a failed commit is
//! [`OperationLedgerError::AcknowledgementUnknown`], which authorizes **zero**
//! provider calls until a database-only read confirms the durable binding.

use nebula_storage_port::store::{OperationLedger, OperationLedgerAdjudicator};
use nebula_storage_port::{
    AttemptGeneration, EffectSlotBinding, EffectSlotId, KnownOutcome, OperationId,
    OperationLedgerError, OperationRecord, OperationState, PrepareOutcome, RequestFingerprint,
    Scope,
};
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::operation_ledger::{
    CommitDecision, compose_record, decide_adjudication, decide_commit, decide_prepare,
    destination_from_text, destination_text, prepare_label, read_label, state_from_text,
    state_text, write_label,
};

/// PostgreSQL-backed durable operation ledger.
///
/// Wrap a pool whose schema was installed via [`super::init_schema`].
#[derive(Clone, Debug)]
pub struct PgOperationLedger {
    pool: PgPool,
}

impl PgOperationLedger {
    /// Wrap an existing pool. The caller installs the port schema (see
    /// [`super::init_schema`]).
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Open the transaction every ledger operation runs in.
    async fn begin(&self) -> Result<Transaction<'_, Postgres>, OperationLedgerError> {
        self.pool.begin().await.map_err(driver_did_not_commit)
    }
}

/// A driver failure reached before commit definitely did not commit.
fn driver_did_not_commit(_error: sqlx::Error) -> OperationLedgerError {
    OperationLedgerError::Unavailable
}

/// A failed commit leaves the caller unable to prove whether the write landed.
///
/// This is deliberately **not** `Unavailable`: after a prepare it authorizes no
/// provider call at all, whereas `Unavailable` means the caller may safely try
/// again.
fn commit_acknowledgement_unknown(_error: sqlx::Error) -> OperationLedgerError {
    OperationLedgerError::AcknowledgementUnknown
}

/// Interpret one durable ledger row.
fn decode_row(row: &sqlx::postgres::PgRow) -> Result<OperationRecord, OperationLedgerError> {
    let slot_bytes: Vec<u8> = row.try_get("slot_id").map_err(driver_did_not_commit)?;
    let slot_id = EffectSlotId::from_storage_bytes(
        <[u8; 16]>::try_from(slot_bytes).map_err(|_width| OperationLedgerError::Unavailable)?,
    );
    let corrupt = |_reason| OperationLedgerError::CorruptRecord { slot_id };

    let operation_bytes: Vec<u8> = row.try_get("operation_id").map_err(corrupt)?;
    let operation_id = OperationId::from_storage_bytes(
        <[u8; 16]>::try_from(operation_bytes)
            .map_err(|_width| OperationLedgerError::CorruptRecord { slot_id })?,
    );
    let generation: i64 = row.try_get("attempt_generation").map_err(corrupt)?;
    let destination: String = row.try_get("destination").map_err(corrupt)?;
    let fingerprint_version: i32 = row.try_get("fingerprint_version").map_err(corrupt)?;
    let fingerprint_bytes: Vec<u8> = row.try_get("fingerprint").map_err(corrupt)?;
    let state: String = row.try_get("state").map_err(corrupt)?;

    let destination = destination_from_text(&destination)
        .ok_or(OperationLedgerError::CorruptRecord { slot_id })?;
    let state = state_from_text(&state).ok_or(OperationLedgerError::CorruptRecord { slot_id })?;
    let fingerprint = RequestFingerprint::new(
        u16::try_from(fingerprint_version)
            .map_err(|_range| OperationLedgerError::CorruptRecord { slot_id })?,
        <[u8; 32]>::try_from(fingerprint_bytes)
            .map_err(|_width| OperationLedgerError::CorruptRecord { slot_id })?,
    );

    Ok(compose_record(
        slot_id,
        operation_id,
        AttemptGeneration::new(
            u64::try_from(generation)
                .map_err(|_range| OperationLedgerError::CorruptRecord { slot_id })?,
        ),
        destination,
        fingerprint,
        state,
    ))
}

/// Read the slot addressed by the natural key a caller can rebuild.
async fn load_by_natural_key(
    tx: &mut Transaction<'_, Postgres>,
    binding: &EffectSlotBinding<'_>,
) -> Result<Option<OperationRecord>, OperationLedgerError> {
    let row = sqlx::query(
        "SELECT slot_id, operation_id, attempt_generation, destination, \
                fingerprint_version, fingerprint, state \
         FROM port_operation_ledger \
         WHERE workspace_id = $1 AND org_id = $2 AND execution_id = $3 \
           AND node_key = $4 AND occurrence = $5",
    )
    .bind(&binding.scope.workspace_id)
    .bind(&binding.scope.org_id)
    .bind(binding.execution_id)
    .bind(binding.node_key)
    .bind(binding.occurrence)
    .fetch_optional(&mut **tx)
    .await
    .map_err(driver_did_not_commit)?;

    row.as_ref().map(decode_row).transpose()
}

/// Read one slot a tenant is allowed to see.
///
/// The scope predicate is part of the query, so a slot owned by another tenant
/// is indistinguishable from an absent one — a caller cannot use a guessed
/// identity to learn that some other tenant holds it.
async fn load_visible(
    tx: &mut Transaction<'_, Postgres>,
    scope: &Scope,
    slot_id: EffectSlotId,
) -> Result<OperationRecord, OperationLedgerError> {
    let row = sqlx::query(
        "SELECT slot_id, operation_id, attempt_generation, destination, \
                fingerprint_version, fingerprint, state \
         FROM port_operation_ledger \
         WHERE slot_id = $1 AND workspace_id = $2 AND org_id = $3 FOR UPDATE",
    )
    .bind(slot_id.as_bytes().as_slice())
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(driver_did_not_commit)?
    .ok_or(OperationLedgerError::SlotUnprepared { slot_id })?;

    decode_row(&row)
}

/// Apply a resolved state to one slot under the caller's scope.
async fn write_state(
    tx: &mut Transaction<'_, Postgres>,
    scope: &Scope,
    slot_id: EffectSlotId,
    state: OperationState,
    evidence: Option<&str>,
    now_ms: i64,
) -> Result<(), OperationLedgerError> {
    sqlx::query(
        "UPDATE port_operation_ledger \
         SET state = $1, outcome_at_ms = $2, adjudication_evidence = $3, adjudicated_at_ms = $4 \
         WHERE slot_id = $5 AND workspace_id = $6 AND org_id = $7",
    )
    .bind(state_text(state))
    .bind(now_ms)
    .bind(evidence)
    .bind(evidence.map(|_present| now_ms))
    .bind(slot_id.as_bytes().as_slice())
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .execute(&mut **tx)
    .await
    .map_err(driver_did_not_commit)
    .map(|_applied| ())
}

#[async_trait::async_trait]
impl OperationLedger for PgOperationLedger {
    #[tracing::instrument(
        level = "debug",
        name = "operation_ledger.prepare",
        skip(self, binding),
        fields(
            backend = "postgres",
            execution_id = binding.execution_id,
            node_key = binding.node_key,
            outcome = tracing::field::Empty,
        )
    )]
    async fn prepare(
        &self,
        binding: &EffectSlotBinding<'_>,
    ) -> Result<PrepareOutcome, OperationLedgerError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let result = async {
            let mut tx = self.begin().await?;

            if let Some(stored) = load_by_natural_key(&mut tx, binding).await? {
                let replayed = decide_prepare(stored.operation().slot_id(), &stored, binding);
                // Nothing was written on either path; the commit only releases
                // the transaction, so failing to release cannot make a
                // rejection ambiguous.
                drop(tx.commit().await);
                return replayed;
            }

            let slot_id = EffectSlotId::from_storage_bytes(*uuid::Uuid::new_v4().as_bytes());
            let operation_id = OperationId::from_storage_bytes(*uuid::Uuid::new_v4().as_bytes());
            let inserted = sqlx::query(
                "INSERT INTO port_operation_ledger \
                 (slot_id, workspace_id, org_id, execution_id, node_key, occurrence, \
                  attempt_generation, fingerprint_version, fingerprint, destination, \
                  operation_id, state, prepared_at_ms) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'prepared', $12) \
                 ON CONFLICT (workspace_id, org_id, execution_id, node_key, occurrence) \
                 DO NOTHING",
            )
            .bind(slot_id.as_bytes().as_slice())
            .bind(&binding.scope.workspace_id)
            .bind(&binding.scope.org_id)
            .bind(binding.execution_id)
            .bind(binding.node_key)
            .bind(binding.occurrence)
            .bind(i64::try_from(binding.attempt_generation.get()).unwrap_or(i64::MAX))
            .bind(i32::from(binding.fingerprint.version()))
            .bind(binding.fingerprint.digest().as_slice())
            .bind(destination_text(binding.destination))
            .bind(operation_id.as_bytes().as_slice())
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(driver_did_not_commit)?
            .rows_affected();

            if inserted == 0 {
                // Another worker won the natural-key race. Re-read its row in
                // this transaction and replay against it, so both callers leave
                // holding one operation identity rather than one of them
                // surfacing a driver conflict.
                let stored = load_by_natural_key(&mut tx, binding)
                    .await?
                    .ok_or(OperationLedgerError::Unavailable)?;
                let replayed = decide_prepare(stored.operation().slot_id(), &stored, binding);
                drop(tx.commit().await);
                return replayed;
            }

            tx.commit().await.map_err(commit_acknowledgement_unknown)?;
            Ok(PrepareOutcome::Prepared(
                compose_record(
                    slot_id,
                    operation_id,
                    binding.attempt_generation,
                    binding.destination,
                    binding.fingerprint,
                    OperationState::Prepared,
                )
                .operation(),
            ))
        }
        .await;

        let outcome = prepare_label(&result);
        tracing::Span::current().record("outcome", outcome);
        tracing::debug!(target: "nebula_storage::postgres", outcome, "operation ledger prepare");
        result
    }

    #[tracing::instrument(
        level = "debug",
        name = "operation_ledger.read_exact",
        skip(self),
        fields(backend = "postgres", outcome = tracing::field::Empty)
    )]
    async fn read_exact(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
    ) -> Result<OperationRecord, OperationLedgerError> {
        let result = async {
            // A read decides nothing it then writes, so it does not take the
            // write lock: a database-only reconciliation must stay available
            // exactly when writers are contending.
            let mut tx = self.begin().await?;
            let record = load_visible(&mut tx, scope, slot_id).await;
            drop(tx.commit().await);
            record
        }
        .await;

        let outcome = read_label(&result);
        tracing::Span::current().record("outcome", outcome);
        tracing::debug!(target: "nebula_storage::postgres", outcome, "operation ledger read");
        result
    }

    #[tracing::instrument(
        level = "debug",
        name = "operation_ledger.commit_outcome",
        skip(self),
        fields(backend = "postgres", outcome = tracing::field::Empty)
    )]
    async fn commit_outcome(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
        attempt_generation: AttemptGeneration,
        outcome: KnownOutcome,
    ) -> Result<(), OperationLedgerError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let result = async {
            let mut tx = self.begin().await?;
            let stored = load_visible(&mut tx, scope, slot_id).await?;
            match decide_commit(slot_id, &stored, attempt_generation, outcome)? {
                CommitDecision::AlreadyRecorded => {
                    drop(tx.commit().await);
                    Ok(())
                },
                CommitDecision::Apply(state) => {
                    write_state(&mut tx, scope, slot_id, state, None, now_ms).await?;
                    tx.commit().await.map_err(commit_acknowledgement_unknown)
                },
            }
        }
        .await;

        let label = write_label(&result);
        tracing::Span::current().record("outcome", label);
        tracing::debug!(
            target: "nebula_storage::postgres",
            outcome = label,
            "operation ledger fenced commit"
        );
        result
    }
}

#[async_trait::async_trait]
impl OperationLedgerAdjudicator for PgOperationLedger {
    #[tracing::instrument(
        level = "debug",
        name = "operation_ledger.adjudicate",
        // `evidence` is operator prose, persisted for review rather than
        // broadcast to every trace consumer.
        skip(self, evidence),
        fields(backend = "postgres", outcome = tracing::field::Empty)
    )]
    async fn adjudicate(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
        outcome: KnownOutcome,
        evidence: &str,
    ) -> Result<(), OperationLedgerError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let result = async {
            if evidence.trim().is_empty() {
                // An adjudication without a reason is not reviewable, and
                // reviewability is the only thing that makes a hand-decided
                // outcome acceptable.
                return Err(OperationLedgerError::CorruptRecord { slot_id });
            }
            let mut tx = self.begin().await?;
            let stored = load_visible(&mut tx, scope, slot_id).await?;
            let state = decide_adjudication(slot_id, &stored, outcome)?;
            write_state(&mut tx, scope, slot_id, state, Some(evidence), now_ms).await?;
            tx.commit().await.map_err(commit_acknowledgement_unknown)
        }
        .await;

        let label = write_label(&result);
        tracing::Span::current().record("outcome", label);
        tracing::debug!(
            target: "nebula_storage::postgres",
            outcome = label,
            "operation ledger adjudication"
        );
        result
    }
}
