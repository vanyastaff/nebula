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

use nebula_core::{OperationCallId, OperationId};
use nebula_storage_port::store::{OperationLedger, OperationLedgerAdjudicator};
use nebula_storage_port::{
    AttemptGeneration, DestinationCapability, EffectOccurrenceKey, EffectSlotBinding, EffectSlotId,
    FencingToken, OperationLedgerError, OperationRecord, OperationState, PrepareOutcome,
    RequestFingerprint, Scope,
};
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::operation_ledger::{
    compose_record, decide_prepare, prepare_label, read_label, state_from_text, state_text,
    write_label,
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
    let operation_id = OperationId::from_bytes(
        <[u8; 16]>::try_from(operation_bytes)
            .map_err(|_width| OperationLedgerError::CorruptRecord { slot_id })?,
    );
    let generation: i64 = row.try_get("attempt_generation").map_err(corrupt)?;
    let destination: String = row.try_get("destination").map_err(corrupt)?;
    let fingerprint_version: i32 = row.try_get("fingerprint_version").map_err(corrupt)?;
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
///
/// Preparation already holds the execution advisory lock. Recovery reads must
/// remain non-blocking with respect to an uncommitted row update, so this query
/// deliberately takes no row lock.
async fn load_by_natural_key(
    tx: &mut Transaction<'_, Postgres>,
    key: &EffectOccurrenceKey<'_>,
) -> Result<Option<OperationRecord>, OperationLedgerError> {
    let row = sqlx::query(
        "SELECT l.slot_id, l.operation_id, l.attempt_generation, l.destination, \
                l.fingerprint_version, l.fingerprint, l.state, \
                (SELECT p.payload FROM port_operation_protocol p \
                 WHERE p.slot_id = l.slot_id AND p.workspace_id = l.workspace_id \
                   AND p.org_id = l.org_id) AS protocol_payload \
         FROM port_operation_ledger l \
         WHERE l.workspace_id = $1 AND l.org_id = $2 AND l.execution_id = $3 \
           AND l.node_key = $4 AND l.occurrence = $5",
    )
    .bind(&key.scope().workspace_id)
    .bind(&key.scope().org_id)
    .bind(key.execution_id())
    .bind(key.node_key())
    .bind(key.occurrence())
    .fetch_optional(&mut **tx)
    .await
    .map_err(driver_did_not_commit)?;

    let Some(row) = row else {
        return Ok(None);
    };
    let payload = row
        .try_get::<Option<String>, _>("protocol_payload")
        .map_err(driver_did_not_commit)?;
    attach_decoded_protocol(decode_row(&row)?, payload.as_deref()).map(Some)
}

/// Read one slot a tenant is allowed to see.
///
/// The scope predicate is part of the query, so a slot owned by another tenant
/// is indistinguishable from an absent one — a caller cannot use a guessed
/// identity to learn that some other tenant holds it.
#[derive(Debug, Clone, Copy)]
enum LedgerRowAccess {
    Read,
    Write,
}

async fn load_visible(
    tx: &mut Transaction<'_, Postgres>,
    scope: &Scope,
    slot_id: EffectSlotId,
    access: LedgerRowAccess,
) -> Result<OperationRecord, OperationLedgerError> {
    let statement = match access {
        LedgerRowAccess::Read => {
            "SELECT l.slot_id, l.operation_id, l.attempt_generation, l.destination, \
                    l.fingerprint_version, l.fingerprint, l.state, \
                    (SELECT p.payload FROM port_operation_protocol p \
                     WHERE p.slot_id = l.slot_id AND p.workspace_id = l.workspace_id \
                       AND p.org_id = l.org_id) AS protocol_payload \
             FROM port_operation_ledger l \
             WHERE l.slot_id = $1 AND l.workspace_id = $2 AND l.org_id = $3"
        },
        LedgerRowAccess::Write => {
            "SELECT l.slot_id, l.operation_id, l.attempt_generation, l.destination, \
                    l.fingerprint_version, l.fingerprint, l.state, \
                    (SELECT p.payload FROM port_operation_protocol p \
                     WHERE p.slot_id = l.slot_id AND p.workspace_id = l.workspace_id \
                       AND p.org_id = l.org_id) AS protocol_payload \
             FROM port_operation_ledger l \
             WHERE l.slot_id = $1 AND l.workspace_id = $2 AND l.org_id = $3 FOR UPDATE OF l"
        },
    };
    let row = sqlx::query(statement)
        .bind(slot_id.as_bytes().as_slice())
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(driver_did_not_commit)?
        .ok_or(OperationLedgerError::SlotUnprepared { slot_id })?;

    let payload = row
        .try_get::<Option<String>, _>("protocol_payload")
        .map_err(driver_did_not_commit)?;
    attach_decoded_protocol(decode_row(&row)?, payload.as_deref())
}

async fn backend_now(tx: &mut Transaction<'_, Postgres>) -> Result<i64, OperationLedgerError> {
    sqlx::query_scalar("SELECT (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint")
        .fetch_one(&mut **tx)
        .await
        .map_err(driver_did_not_commit)
}

fn attach_decoded_protocol(
    record: OperationRecord,
    payload: Option<&str>,
) -> Result<OperationRecord, OperationLedgerError> {
    match crate::operation_ledger::decode_protocol(payload)? {
        Some(protocol) => {
            let record = record.with_protocol(protocol);
            crate::operation_ledger::validate_record(&record)?;
            Ok(record)
        },
        None => Ok(record),
    }
}

async fn insert_protocol(
    tx: &mut Transaction<'_, Postgres>,
    binding: &EffectSlotBinding<'_>,
    slot: EffectSlotId,
    protocol: &nebula_storage_port::dto::OperationProtocolRecord,
) -> Result<(), OperationLedgerError> {
    let payload =
        serde_json::to_string(protocol).map_err(|_| OperationLedgerError::InvalidProtocol)?;
    sqlx::query("INSERT INTO port_operation_protocol(slot_id, workspace_id, org_id, execution_id, payload) VALUES($1,$2,$3,$4,$5)")
        .bind(slot.as_bytes().as_slice()).bind(&binding.scope.workspace_id).bind(&binding.scope.org_id).bind(binding.execution_id).bind(payload)
        .execute(&mut **tx).await.map_err(driver_did_not_commit)?;
    Ok(())
}

async fn persist_decision(
    tx: &mut Transaction<'_, Postgres>,
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
    sqlx::query("UPDATE port_operation_protocol SET payload = $1 WHERE slot_id = $2 AND workspace_id = $3 AND org_id = $4")
        .bind(payload).bind(slot.as_bytes().as_slice()).bind(&scope.workspace_id).bind(&scope.org_id).execute(&mut **tx).await.map_err(driver_did_not_commit)?;
    if decision.journal.is_some() && decision.record.state() != OperationState::Prepared {
        write_state(tx, scope, slot, decision.record.state(), None, now_ms).await?;
    }
    if let Some(journal) = &decision.journal {
        let execution: String = sqlx::query_scalar("SELECT execution_id FROM port_operation_ledger WHERE slot_id = $1 AND workspace_id = $2 AND org_id = $3")
            .bind(slot.as_bytes().as_slice()).bind(&scope.workspace_id).bind(&scope.org_id).fetch_one(&mut **tx).await.map_err(driver_did_not_commit)?;
        let sequence: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM port_execution_journal WHERE execution_id = $1",
        )
        .bind(&execution)
        .fetch_one(&mut **tx)
        .await
        .map_err(driver_did_not_commit)?;
        sqlx::query(
            "INSERT INTO port_execution_journal(execution_id, seq, payload) VALUES($1,$2,$3)",
        )
        .bind(execution)
        .bind(sequence)
        .bind(journal)
        .execute(&mut **tx)
        .await
        .map_err(driver_did_not_commit)?;
    }
    Ok(())
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
    #[tracing::instrument(level = "debug", skip_all, name = "operation_ledger.read_occurrence", fields(backend = "postgres", outcome = tracing::field::Empty))]
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
            backend = "postgres",
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
            let mut tx = self.begin().await?;
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
            .bind(generation)
            .bind(i32::from(binding.fingerprint.version()))
            .bind(binding.fingerprint.digest().as_slice())
            .bind(<&'static str>::from(binding.destination))
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
                let stored = load_by_natural_key(&mut tx, &binding.occurrence_key())
                    .await?
                    .ok_or(OperationLedgerError::Unavailable)?;
                let replayed = decide_prepare(stored.operation().slot_id(), &stored, binding);
                drop(tx.commit().await);
                return replayed;
            }

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
            let record = load_visible(&mut tx, scope, slot_id, LedgerRowAccess::Read).await;
            drop(tx.commit().await);
            record
        }
        .await;

        let outcome = read_label(&result);
        tracing::Span::current().record("outcome", outcome);
        tracing::debug!(target: "nebula_storage::postgres", outcome, "operation ledger read");
        result
    }

    #[tracing::instrument(level = "debug", name = "operation_ledger.advance", skip_all, fields(backend = "postgres", outcome = tracing::field::Empty))]
    async fn advance(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
        fencing: FencingToken,
        command: &nebula_storage_port::dto::OperationCommand,
    ) -> Result<nebula_storage_port::dto::OperationAdvance, OperationLedgerError> {
        let result = async {
            let mut tx = self.begin().await?;
            lock_slot_owner(&mut tx, scope, slot_id, Some(fencing)).await?;
            let stored = load_visible(&mut tx, scope, slot_id, LedgerRowAccess::Write).await?;
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
impl OperationLedgerAdjudicator for PgOperationLedger {
    #[tracing::instrument(
        level = "debug",
        name = "operation_ledger.adjudicate",
        // `evidence` is operator prose, persisted for review rather than
        // broadcast to every trace consumer.
        skip_all,
        fields(backend = "postgres", outcome = tracing::field::Empty)
    )]
    async fn adjudicate(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
        outcome: &nebula_storage_port::dto::FrozenOutcomeEvidence,
        evidence: &str,
    ) -> Result<(), OperationLedgerError> {
        let result = async {
            let mut tx = self.begin().await?;
            lock_slot_owner(&mut tx, scope, slot_id, None).await?;
            let stored = load_visible(&mut tx, scope, slot_id, LedgerRowAccess::Write).await?;
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
    tx: &mut Transaction<'_, Postgres>,
    scope: &Scope,
    execution_id: &str,
    fencing: Option<FencingToken>,
) -> Result<(), OperationLedgerError> {
    let row = sqlx::query("SELECT fencing_generation, lease_holder, lease_expires_at_ms FROM port_executions WHERE id = $1 AND workspace_id = $2 AND org_id = $3 FOR UPDATE")
        .bind(execution_id).bind(&scope.workspace_id).bind(&scope.org_id).fetch_optional(&mut **tx).await.map_err(driver_did_not_commit)?
        .ok_or(OperationLedgerError::ExecutionLeaseRejected)?;
    if let Some(fencing) = fencing {
        let now: i64 =
            sqlx::query_scalar("SELECT (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint")
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
    tx: &mut Transaction<'_, Postgres>,
    scope: &Scope,
    slot_id: EffectSlotId,
    fencing: Option<FencingToken>,
) -> Result<(), OperationLedgerError> {
    let execution: String = sqlx::query_scalar("SELECT execution_id FROM port_operation_ledger WHERE slot_id = $1 AND workspace_id = $2 AND org_id = $3")
        .bind(slot_id.as_bytes().as_slice()).bind(&scope.workspace_id).bind(&scope.org_id).fetch_optional(&mut **tx).await.map_err(driver_did_not_commit)?
        .ok_or(OperationLedgerError::SlotUnprepared { slot_id })?;
    lock_execution(tx, scope, &execution, fencing).await
}

#[cfg(test)]
mod snapshot_query_tests {
    #[test]
    fn occurrence_snapshot_is_fetched_in_one_database_round_trip() {
        let source = include_str!("operation_ledger.rs");
        let start = source
            .find("async fn load_by_natural_key(")
            .expect("natural-key loader remains present");
        let function = &source[start..];
        let end = function
            .find("\n/// Read one slot")
            .expect("the following loader documentation remains present");
        let function = &function[..end];

        assert_eq!(
            function.matches(".fetch_").count(),
            1,
            "ledger state and protocol payload must use one PostgreSQL snapshot"
        );
        assert!(
            function.contains("port_operation_ledger")
                && function.contains("port_operation_protocol")
                && function.contains("protocol_payload"),
            "the single query must project both ledger state and protocol payload"
        );
    }
}
