//! In-memory [`StartAcceptanceStore`] over the shared execution-store core.
//!
//! The reservation, the execution row, and the Start control row are written
//! inside one `parking_lot` mutex critical section — the in-memory equivalent
//! of the SQL backends' single transaction.
//!
//! The mutex is not a transaction: there is no rollback. Write order therefore
//! carries the atomicity, exactly as `claim_and_materialize_start` does — the
//! reservation is only recorded once the execution row has been inserted, so a
//! duplicate execution id can never leave a key reserved for an execution that
//! does not exist. The materialized path validates revision admission and both
//! id collisions *before* any mutation, so a rejected admission or a duplicate
//! leaves the shared state untouched.

use std::time::Duration;

use nebula_core::id::ExecutionId;
use nebula_storage_port::StorageError;
use nebula_storage_port::dto::ControlMsg;
use nebula_storage_port::store::{
    KeyedStart, MaterializedKeyedStart, StartAcceptance, StartAcceptanceStore,
    StartMaterialization, StartRevisionRejection,
};

use super::execution::{QueuedMsg, SharedState, StartKeyReservation, insert_created_row};
use super::plan_flavor_catalog::{
    InternalRevisionError, RetainDecision, RevisionReference, RevisionReferenceOwner,
    retain_exact_locked,
};

/// In-memory owner of keyed start acceptance.
#[derive(Clone, Debug)]
pub struct InMemoryStartAcceptanceStore {
    inner: SharedState,
}

impl InMemoryStartAcceptanceStore {
    /// Build a start-acceptance store over an execution store's shared core.
    #[must_use]
    pub fn new(store: &super::InMemoryExecutionStore) -> Self {
        Self {
            inner: store.shared(),
        }
    }
}

#[async_trait::async_trait]
impl StartAcceptanceStore for InMemoryStartAcceptanceStore {
    #[tracing::instrument(
        level = "debug",
        skip(self, start),
        fields(
            execution_id = start.execution_id,
            fingerprint_version = start.fingerprint.version(),
        )
    )]
    async fn accept_keyed_start(
        &self,
        start: &KeyedStart<'_>,
    ) -> Result<StartAcceptance, StorageError> {
        let key = (
            start.scope.workspace_id.clone(),
            start.scope.org_id.clone(),
            start.start_key.to_owned(),
        );
        let mut state = self.inner.lock();

        if let Some(existing) = state.start_keys.get(&key) {
            return Ok(crate::start_acceptance::replay_outcome(
                start,
                i64::from(existing.fingerprint_version),
                &existing.fingerprint,
                existing.execution_id.clone(),
            ));
        }

        // Validate *both* collisions before mutating anything.
        //
        // The mutex is not a transaction: there is no rollback, so any write
        // made before a later check fails is permanent. Inserting the
        // execution row first and only then rejecting a duplicate command id
        // left the execution behind while reporting an error — the SQL
        // backends roll the whole transaction back, so the in-memory adapter
        // would have been the one backend where a failed acceptance still
        // created an execution.
        if state.rows.contains_key(start.execution_id) {
            return Err(StorageError::Duplicate {
                entity: "execution",
                detail: format!("execution {} already exists", start.execution_id),
            });
        }
        if state.queue.contains_key(&start.command.id) {
            return Err(StorageError::Duplicate {
                entity: "control_queue",
                detail: format!("control command id {:?} already queued", start.command.id),
            });
        }

        // Past this point every remaining step is infallible, so the three
        // writes land together or not at all.
        insert_created_row(
            &mut state,
            start.scope,
            start.execution_id,
            start.execution.workflow_id,
            start.execution.initial_state,
        )?;
        state.queue.insert(
            start.command.id,
            QueuedMsg {
                msg: ControlMsg::clone(start.command),
                status: "Pending".to_owned(),
                processed_by: None,
                processed_at: None,
                reclaim_count: 0,
                error_message: None,
                claim_generation: 0,
            },
        );

        state.start_keys.insert(
            key,
            StartKeyReservation {
                fingerprint_version: start.fingerprint.version(),
                fingerprint: *start.fingerprint.digest(),
                execution_id: start.execution_id.to_owned(),
                created_at: chrono::Utc::now(),
            },
        );
        tracing::debug!(
            target: "nebula_storage::inmem",
            "start_acceptance: reserved key, created execution, enqueued Start"
        );
        Ok(StartAcceptance::Accepted {
            execution_id: start.execution_id.to_owned(),
        })
    }

    #[tracing::instrument(
        level = "debug",
        skip(self, start),
        fields(
            execution_id = start.keyed.execution_id,
            fingerprint_version = start.keyed.fingerprint.version(),
        )
    )]
    async fn materialize_keyed_start(
        &self,
        start: &MaterializedKeyedStart<'_>,
    ) -> Result<StartMaterialization, StorageError> {
        let keyed = &start.keyed;
        crate::start_acceptance::validate_materialized_start(start)?;
        let key = (
            keyed.scope.workspace_id.clone(),
            keyed.scope.org_id.clone(),
            keyed.start_key.to_owned(),
        );
        let execution_id = keyed.execution_id.parse::<ExecutionId>().map_err(|_| {
            StorageError::Internal(
                "materialize_keyed_start: execution id is not a typed ExecutionId".to_owned(),
            )
        })?;
        let mut state = self.inner.lock();

        if let Some(existing) = state.start_keys.get(&key) {
            return Ok(
                match crate::start_acceptance::replay_outcome(
                    keyed,
                    i64::from(existing.fingerprint_version),
                    &existing.fingerprint,
                    existing.execution_id.clone(),
                ) {
                    StartAcceptance::Replayed { execution_id } => {
                        StartMaterialization::Replayed { execution_id }
                    },
                    StartAcceptance::FingerprintMismatch => {
                        StartMaterialization::FingerprintMismatch
                    },
                    // replay_outcome only ever reports the incumbent's receipt.
                    StartAcceptance::Accepted { .. } => {
                        unreachable!("a stored reservation can never replay as a fresh acceptance")
                    },
                },
            );
        }

        // Validate every collision before mutating anything: the mutex is not
        // a transaction, so a write made before a later check fails would be
        // permanent while the SQL backends roll the whole transaction back.
        if state.rows.contains_key(keyed.execution_id) {
            return Err(StorageError::Duplicate {
                entity: "execution",
                detail: format!("execution {} already exists", keyed.execution_id),
            });
        }
        if state.queue.contains_key(&keyed.command.id) {
            return Err(StorageError::Duplicate {
                entity: "control_queue",
                detail: format!("control command id {:?} already queued", keyed.command.id),
            });
        }

        // Admit the exact pair and insert the live reference. Admission runs
        // before any aggregate write so a rejected revision leaves no trace;
        // `retain_exact_locked` itself validates before it inserts.
        let reference = RevisionReference::new(
            RevisionReferenceOwner::for_execution(execution_id),
            start.identity.bundle_id(),
            start.identity.revisions(),
        );
        match retain_exact_locked(&mut state, reference) {
            Ok(RetainDecision::Retained) => {},
            // The owner is a freshly parsed execution id no row exists for yet;
            // an existing reference here would be an internal invariant break.
            Ok(RetainDecision::AlreadyRetained) => {
                return Err(StorageError::Internal(
                    "materialize_keyed_start: reference already exists for a new execution"
                        .to_owned(),
                ));
            },
            Err(InternalRevisionError::PlanUnavailable) => {
                return Ok(StartMaterialization::RevisionRejected(
                    StartRevisionRejection::PlanUnavailable,
                ));
            },
            Err(InternalRevisionError::WorkerFlavorUnavailable) => {
                return Ok(StartMaterialization::RevisionRejected(
                    StartRevisionRejection::WorkerFlavorUnavailable,
                ));
            },
            Err(InternalRevisionError::PairNotAdmitted) => {
                return Ok(StartMaterialization::RevisionRejected(
                    StartRevisionRejection::PairNotAdmitted,
                ));
            },
            Err(InternalRevisionError::Draining | InternalRevisionError::Deleted) => {
                return Ok(StartMaterialization::RevisionRejected(
                    StartRevisionRejection::PairNotAdmitted,
                ));
            },
            Err(other) => {
                return Err(StorageError::Internal(format!(
                    "materialize_keyed_start: {other}"
                )));
            },
        }

        // Past this point every remaining step is infallible, so the
        // reservation, the execution row, the Start command, and the live
        // reference land together or not at all.
        insert_created_row(
            &mut state,
            keyed.scope,
            keyed.execution_id,
            keyed.execution.workflow_id,
            keyed.execution.initial_state,
        )?;
        state.queue.insert(
            keyed.command.id,
            QueuedMsg {
                msg: ControlMsg::clone(keyed.command),
                status: "Pending".to_owned(),
                processed_by: None,
                processed_at: None,
                reclaim_count: 0,
                error_message: None,
                claim_generation: 0,
            },
        );

        state.start_keys.insert(
            key,
            StartKeyReservation {
                fingerprint_version: keyed.fingerprint.version(),
                fingerprint: *keyed.fingerprint.digest(),
                execution_id: keyed.execution_id.to_owned(),
                created_at: chrono::Utc::now(),
            },
        );
        tracing::debug!(
            target: "nebula_storage::inmem",
            "start_acceptance: materialized start with a live revision reference"
        );
        Ok(StartMaterialization::Accepted {
            execution_id: keyed.execution_id.to_owned(),
        })
    }

    async fn evict_reservations_older_than(
        &self,
        retention: Duration,
    ) -> Result<u64, StorageError> {
        let cutoff = chrono::Utc::now()
            - chrono::Duration::from_std(retention).unwrap_or(chrono::Duration::MAX);
        let mut state = self.inner.lock();
        let before = state.start_keys.len();
        state
            .start_keys
            .retain(|_, reservation| reservation.created_at >= cutoff);
        Ok((before - state.start_keys.len()) as u64)
    }
}
