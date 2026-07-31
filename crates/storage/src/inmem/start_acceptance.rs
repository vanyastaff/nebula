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
//! does not exist.

use std::time::Duration;

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::ControlMsg;
use nebula_storage_port::store::{KeyedStart, StartAcceptance, StartAcceptanceStore};

use super::execution::{QueuedMsg, SharedState, StartKeyReservation, insert_created_row};

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
