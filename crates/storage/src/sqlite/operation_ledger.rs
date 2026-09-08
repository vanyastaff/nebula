//! SQLite operation ledger over ordered migration 0045.
//!
//! Every operation runs under `BEGIN IMMEDIATE`, so the read that decides and
//! the write that follows are one linearized operation against the single
//! writer. A deferred transaction would let another writer replace the row
//! between the decision and the write, which for this table means deciding
//! "not prepared" against a slot another worker just prepared — and preparing
//! it a second time under a second operation identity.
//!
//! Driver detail and request payloads never cross the port boundary. A failure
//! before commit is [`OperationLedgerError::Unavailable`] (the operation
//! definitely did not commit); a failed commit is
//! [`OperationLedgerError::AcknowledgementUnknown`], which authorizes **zero**
//! provider calls until a database-only read confirms the durable binding.

use nebula_core::{OperationCallId, OperationId};
use nebula_storage_port::store::{OperationLedger, OperationLedgerAdjudicator};
use nebula_storage_port::{
    AttemptGeneration, DestinationCapability, EffectOccurrenceKey, EffectSlotBinding, EffectSlotId,
    FencingToken, OperationLedgerError, OperationRecord, OperationState, PrepareOutcome,
    RequestFingerprint, Scope,
};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use crate::operation_ledger::{
    compose_record, decide_prepare, prepare_label, read_label, state_from_text, state_text,
    write_label,
};

/// SQLite-backed durable operation ledger.
///
/// Wrap a pool whose schema was installed via [`super::init_schema`].
#[derive(Clone, Debug)]
pub struct SqliteOperationLedger {
    pool: SqlitePool,
}

impl SqliteOperationLedger {
    /// Wrap an existing pool. The caller installs the port schema (see
    /// [`super::init_schema`]).
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Open the single-writer transaction every ledger operation runs in.
    async fn begin_write(&self) -> Result<Transaction<'_, Sqlite>, OperationLedgerError> {
        self.pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(driver_did_not_commit)
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
fn decode_row(row: &sqlx::sqlite::SqliteRow) -> Result<OperationRecord, OperationLedgerError> {
    let slot_bytes: Vec<u8> = row.try_get("slot_id").map_err(driver_did_not_commit)?;
    let slot_id = EffectSlotId::from_storage_bytes(
        <[u8; 16]>::try_from(slot_bytes).map_err(|_width| OperationLedgerError::Unavailable)?,
    );
    let corrupt = |_reason| OperationLedgerError::CorruptRecord { slot_id };

    let operation_bytes: Vec<u8> = row.try_get("operation_id").map_err(corrupt)?;
    let operation_id = OperationId::from_bytes(
        <[u8; 16]>::try_from(operation_bytes)
            .map_err(|_width| OperationLedgerError::CorruptRecord { slot_id })?,
    );
    let generation: i64 = row.try_get("attempt_generation").map_err(corrupt)?;
    let destination: String = row.try_get("destination").map_err(corrupt)?;
    let fingerprint_version: i64 = row.try_get("fingerprint_version").map_err(corrupt)?;
    let fingerprint_bytes: Vec<u8> = row.try_get("fingerprint").map_err(corrupt)?;
    let state: String = row.try_get("state").map_err(corrupt)?;

    let destination = DestinationCapability::try_from(destination.as_str())
        .map_err(|_| OperationLedgerError::CorruptRecord { slot_id })?;
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
    tx: &mut Transaction<'_, Sqlite>,
    key: &EffectOccurrenceKey<'_>,
) -> Result<Option<OperationRecord>, OperationLedgerError> {
    let row = sqlx::query(
        "SELECT slot_id, operation_id, attempt_generation, destination, \
                fingerprint_version, fingerprint, state \
         FROM port_operation_ledger \
         WHERE workspace_id = ? AND org_id = ? AND execution_id = ? \
           AND node_key = ? AND occurrence = ?",
    )
    .bind(&key.scope().workspace_id)
    .bind(&key.scope().org_id)
    .bind(key.execution_id())
    .bind(key.node_key())
    .bind(key.occurrence())
    .fetch_optional(&mut **tx)
    .await
    .map_err(driver_did_not_commit)?;

    match row.as_ref().map(decode_row).transpose()? {
        Some(record) => attach_protocol(tx, key.scope(), record).await.map(Some),
        None => Ok(None),
    }
}

/// Read one slot a tenant is allowed to see.
///
/// The scope predicate is part of the query, so a slot owned by another tenant
/// is indistinguishable from an absent one — a caller cannot use a guessed
/// identity to learn that some other tenant holds it.
async fn load_visible(
    tx: &mut Transaction<'_, Sqlite>,
    scope: &Scope,
    slot_id: EffectSlotId,
) -> Result<OperationRecord, OperationLedgerError> {
    let row = sqlx::query(
        "SELECT slot_id, operation_id, attempt_generation, destination, \
                fingerprint_version, fingerprint, state \
         FROM port_operation_ledger \
         WHERE slot_id = ? AND workspace_id = ? AND org_id = ?",
    )
    .bind(slot_id.as_bytes().as_slice())
    .bind(&scope.workspace_id)
    .bind(&scope.org_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(driver_did_not_commit)?
    .ok_or(OperationLedgerError::SlotUnprepared { slot_id })?;

    attach_protocol(tx, scope, decode_row(&row)?).await
}

async fn backend_now(tx: &mut Transaction<'_, Sqlite>) -> Result<i64, OperationLedgerError> {
    sqlx::query_scalar("SELECT CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)")
        .fetch_one(&mut **tx)
        .await
        .map_err(driver_did_not_commit)
}

async fn attach_protocol(
    tx: &mut Transaction<'_, Sqlite>,
    scope: &Scope,
    record: OperationRecord,
) -> Result<OperationRecord, OperationLedgerError> {
    let payload: Option<String> = sqlx::query_scalar("SELECT payload FROM port_operation_protocol WHERE slot_id = ? AND workspace_id = ? AND org_id = ?")
        .bind(record.operation().slot_id().as_bytes().as_slice()).bind(&scope.workspace_id).bind(&scope.org_id)
        .fetch_optional(&mut **tx).await.map_err(driver_did_not_commit)?;
    match crate::operation_ledger::decode_protocol(payload.as_deref())? {
        Some(protocol) => {
            let record = record.with_protocol(protocol);
            crate::operation_ledger::validate_record(&record)?;
            Ok(record)
        },
        None => Ok(record),
    }
}

async fn insert_protocol(
    tx: &mut Transaction<'_, Sqlite>,
    binding: &EffectSlotBinding<'_>,
    slot: EffectSlotId,
    protocol: &nebula_storage_port::dto::OperationProtocolRecord,
) -> Result<(), OperationLedgerError> {
    let payload =
        serde_json::to_string(protocol).map_err(|_| OperationLedgerError::InvalidProtocol)?;
    sqlx::query("INSERT INTO port_operation_protocol(slot_id, workspace_id, org_id, execution_id, payload) VALUES(?,?,?,?,?)")
        .bind(slot.as_bytes().as_slice()).bind(&binding.scope.workspace_id).bind(&binding.scope.org_id).bind(binding.execution_id).bind(payload)
        .execute(&mut **tx).await.map_err(driver_did_not_commit)?;
    Ok(())
}

async fn persist_decision(
    tx: &mut Transaction<'_, Sqlite>,
    scope: &Scope,
    slot: EffectSlotId,
    decision: &crate::operation_ledger::ProtocolDecision,
    now_ms: i64,
) -> Result<(), OperationLedgerError> {
    let protocol = decision
        .record
        .protocol()
        .ok_or(OperationLedgerError::InvalidProtocol)?;
    let payload =
        serde_json::to_string(protocol).map_err(|_| OperationLedgerError::InvalidProtocol)?;
    sqlx::query("UPDATE port_operation_protocol SET payload = ? WHERE slot_id = ? AND workspace_id = ? AND org_id = ?")
        .bind(payload).bind(slot.as_bytes().as_slice()).bind(&scope.workspace_id).bind(&scope.org_id).execute(&mut **tx).await.map_err(driver_did_not_commit)?;
    if decision.journal.is_some() && decision.record.state() != OperationState::Prepared {
        write_state(tx, scope, slot, decision.record.state(), None, now_ms).await?;
    }
    if let Some(journal) = &decision.journal {
        let execution: String = sqlx::query_scalar("SELECT execution_id FROM port_operation_ledger WHERE slot_id = ? AND workspace_id = ? AND org_id = ?")
            .bind(slot.as_bytes().as_slice()).bind(&scope.workspace_id).bind(&scope.org_id).fetch_one(&mut **tx).await.map_err(driver_did_not_commit)?;
        let sequence: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM port_execution_journal WHERE execution_id = ?",
        )
        .bind(&execution)
        .fetch_one(&mut **tx)
        .await
        .map_err(driver_did_not_commit)?;
        sqlx::query("INSERT INTO port_execution_journal(execution_id, seq, payload) VALUES(?,?,?)")
            .bind(execution)
            .bind(sequence)
            .bind(
                serde_json::to_string(journal)
                    .map_err(|_| OperationLedgerError::InvalidProtocol)?,
            )
            .execute(&mut **tx)
            .await
            .map_err(driver_did_not_commit)?;
    }
    Ok(())
}

/// Apply a resolved state to one slot under the caller's scope.
async fn write_state(
    tx: &mut Transaction<'_, Sqlite>,
    scope: &Scope,
    slot_id: EffectSlotId,
    state: OperationState,
    evidence: Option<&str>,
    now_ms: i64,
) -> Result<(), OperationLedgerError> {
    sqlx::query(
        "UPDATE port_operation_ledger \
         SET state = ?, outcome_at_ms = ?, adjudication_evidence = ?, adjudicated_at_ms = ? \
         WHERE slot_id = ? AND workspace_id = ? AND org_id = ?",
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
impl OperationLedger for SqliteOperationLedger {
    #[tracing::instrument(level = "debug", skip_all, name = "operation_ledger.read_occurrence", fields(backend = "sqlite", outcome = tracing::field::Empty))]
    async fn read_occurrence(
        &self,
        key: &EffectOccurrenceKey<'_>,
    ) -> Result<Option<OperationRecord>, OperationLedgerError> {
        let result = async {
            let mut tx = self.pool.begin().await.map_err(driver_did_not_commit)?;
            load_by_natural_key(&mut tx, key).await
        }
        .await;
        let outcome = crate::operation_ledger::occurrence_read_label(&result);
        tracing::Span::current().record("outcome", outcome);
        result
    }

    #[tracing::instrument(
        level = "debug",
        name = "operation_ledger.prepare",
        skip(self, binding),
        fields(
            backend = "sqlite",
            execution_id = binding.execution_id,
            node_key = binding.node_key,
            outcome = tracing::field::Empty,
        )
    )]
    async fn prepare(
        &self,
        binding: &EffectSlotBinding<'_>,
        fencing: FencingToken,
    ) -> Result<PrepareOutcome, OperationLedgerError> {
        let result = async {
            let generation =
                crate::operation_ledger::stored_attempt_generation(binding.attempt_generation)?;
            let mut tx = self.begin_write().await?;
            lock_execution(&mut tx, binding.scope, binding.execution_id, Some(fencing)).await?;
            let now_ms = backend_now(&mut tx).await?;
            let protocol = crate::operation_ledger::initial_protocol(binding, now_ms)?;

            if let Some(stored) = load_by_natural_key(&mut tx, &binding.occurrence_key()).await? {
                let replayed = decide_prepare(stored.operation().slot_id(), &stored, binding);
                // Nothing was written on either path; the commit only releases
                // the transaction, so failing to release cannot make a
                // rejection ambiguous.
                drop(tx.commit().await);
                return replayed;
            }

            let slot_id = EffectSlotId::from_storage_bytes(*uuid::Uuid::new_v4().as_bytes());
            let operation_id = OperationId::from_bytes(*uuid::Uuid::new_v4().as_bytes());
            sqlx::query(
                "INSERT INTO port_operation_ledger \
                 (slot_id, workspace_id, org_id, execution_id, node_key, occurrence, \
                  attempt_generation, fingerprint_version, fingerprint, destination, \
                  operation_id, state, prepared_at_ms) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'prepared', ?)",
            )
            .bind(slot_id.as_bytes().as_slice())
            .bind(&binding.scope.workspace_id)
            .bind(&binding.scope.org_id)
            .bind(binding.execution_id)
            .bind(binding.node_key)
            .bind(binding.occurrence)
            .bind(generation)
            .bind(i64::from(binding.fingerprint.version()))
            .bind(binding.fingerprint.digest().as_slice())
            .bind(<&'static str>::from(binding.destination))
            .bind(operation_id.as_bytes().as_slice())
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(driver_did_not_commit)?;

            insert_protocol(&mut tx, binding, slot_id, &protocol).await?;
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
        tracing::debug!(target: "nebula_storage::sqlite", outcome, "operation ledger prepare");
        result
    }

    #[tracing::instrument(
        level = "debug",
        name = "operation_ledger.read_exact",
        skip(self),
        fields(backend = "sqlite", outcome = tracing::field::Empty)
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
            let mut tx = self.pool.begin().await.map_err(driver_did_not_commit)?;
            let record = load_visible(&mut tx, scope, slot_id).await;
            drop(tx.commit().await);
            record
        }
        .await;

        let outcome = read_label(&result);
        tracing::Span::current().record("outcome", outcome);
        tracing::debug!(target: "nebula_storage::sqlite", outcome, "operation ledger read");
        result
    }

    #[tracing::instrument(level = "debug", name = "operation_ledger.advance", skip_all, fields(backend = "sqlite", outcome = tracing::field::Empty))]
    async fn advance(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
        fencing: FencingToken,
        command: &nebula_storage_port::dto::OperationCommand,
    ) -> Result<nebula_storage_port::dto::OperationAdvance, OperationLedgerError> {
        let result = async {
            let mut tx = self.begin_write().await?;
            lock_slot_owner(&mut tx, scope, slot_id, Some(fencing)).await?;
            let stored = load_visible(&mut tx, scope, slot_id).await?;
            let now_ms = backend_now(&mut tx).await?;
            let fresh = OperationCallId::from_bytes(*uuid::Uuid::new_v4().as_bytes());
            let decision =
                crate::operation_ledger::decide_advance(&stored, command, now_ms, fresh)?;
            if decision.changed {
                persist_decision(&mut tx, scope, slot_id, &decision, now_ms).await?;
                tx.commit().await.map_err(commit_acknowledgement_unknown)?;
            }
            Ok(decision.response)
        }
        .await;
        tracing::Span::current().record("outcome", crate::operation_ledger::advance_label(&result));
        result
    }
}

#[async_trait::async_trait]
impl OperationLedgerAdjudicator for SqliteOperationLedger {
    #[tracing::instrument(
        level = "debug",
        name = "operation_ledger.adjudicate",
        // `evidence` is operator prose, persisted for review rather than
        // broadcast to every trace consumer.
        skip_all,
        fields(backend = "sqlite", outcome = tracing::field::Empty)
    )]
    async fn adjudicate(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
        outcome: &nebula_storage_port::dto::FrozenOutcomeEvidence,
        evidence: &str,
    ) -> Result<(), OperationLedgerError> {
        let result = async {
            let mut tx = self.begin_write().await?;
            lock_slot_owner(&mut tx, scope, slot_id, None).await?;
            let stored = load_visible(&mut tx, scope, slot_id).await?;
            let decision =
                crate::operation_ledger::decide_adjudicate_protocol(&stored, outcome, evidence)?;
            if decision.changed {
                let now_ms = backend_now(&mut tx).await?;
                persist_decision(&mut tx, scope, slot_id, &decision, now_ms).await?;
                write_state(
                    &mut tx,
                    scope,
                    slot_id,
                    decision.record.state(),
                    Some(evidence),
                    now_ms,
                )
                .await?;
                tx.commit().await.map_err(commit_acknowledgement_unknown)?;
            }
            Ok(())
        }
        .await;
        tracing::Span::current().record("outcome", write_label(&result));
        result
    }
}

async fn lock_execution(
    tx: &mut Transaction<'_, Sqlite>,
    scope: &Scope,
    execution_id: &str,
    fencing: Option<FencingToken>,
) -> Result<(), OperationLedgerError> {
    let row = sqlx::query("SELECT fencing_generation, lease_holder, lease_expires_at_ms FROM port_executions WHERE id = ? AND workspace_id = ? AND org_id = ?")
        .bind(execution_id).bind(&scope.workspace_id).bind(&scope.org_id).fetch_optional(&mut **tx).await.map_err(driver_did_not_commit)?
        .ok_or(OperationLedgerError::ExecutionLeaseRejected)?;
    if let Some(fencing) = fencing {
        let now: i64 = sqlx::query_scalar(
            "SELECT CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)",
        )
        .fetch_one(&mut **tx)
        .await
        .map_err(driver_did_not_commit)?;
        let generation: i64 = row
            .try_get("fencing_generation")
            .map_err(driver_did_not_commit)?;
        let holder: Option<String> = row.try_get("lease_holder").map_err(driver_did_not_commit)?;
        let expires: Option<i64> = row
            .try_get("lease_expires_at_ms")
            .map_err(driver_did_not_commit)?;
        crate::operation_ledger::require_live_lease(
            fencing,
            u64::try_from(generation).map_err(|_| OperationLedgerError::ExecutionLeaseRejected)?,
            holder.is_some() && expires.is_some_and(|deadline| deadline > now),
        )?;
    }
    Ok(())
}

async fn lock_slot_owner(
    tx: &mut Transaction<'_, Sqlite>,
    scope: &Scope,
    slot_id: EffectSlotId,
    fencing: Option<FencingToken>,
) -> Result<(), OperationLedgerError> {
    let execution: String = sqlx::query_scalar("SELECT execution_id FROM port_operation_ledger WHERE slot_id = ? AND workspace_id = ? AND org_id = ?")
        .bind(slot_id.as_bytes().as_slice()).bind(&scope.workspace_id).bind(&scope.org_id).fetch_optional(&mut **tx).await.map_err(driver_did_not_commit)?
        .ok_or(OperationLedgerError::SlotUnprepared { slot_id })?;
    lock_execution(tx, scope, &execution, fencing).await
}
