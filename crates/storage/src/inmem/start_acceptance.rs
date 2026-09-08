//! In-memory [`StartAcceptanceStore`] over the shared execution-store core.
//!
//! The reservation, execution, immutable contract bundle, Start command, and
//! live revision reference are written inside one `parking_lot` mutex critical
//! section, the in-memory equivalent of the SQL backends' single transaction.
//!
//! The mutex is not a transaction: there is no rollback. Write order therefore
//! carries the atomicity. Revision admission and every identity collision are
//! validated before mutation, so rejection leaves the shared state untouched.

use std::time::Duration;

use nebula_core::id::ExecutionId;
use nebula_storage_port::StorageError;
use nebula_storage_port::store::{
    StartAcceptanceStore, StartMaterialization, StartReservationMaintenance, StartRevisionRejection,
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
    #[tracing::instrument(skip_all, err)]
    async fn lookup_trigger_start(
        &self,
        scope: &nebula_storage_port::Scope,
        key: &nebula_storage_port::dto::TriggerStartKey<'_>,
    ) -> Result<Option<String>, StorageError> {
        Ok(self
            .inner
            .lock()
            .dedup
            .get(&(
                scope.workspace_id.clone(),
                scope.org_id.clone(),
                key.trigger_id().to_owned(),
                key.event_id().to_owned(),
            ))
            .cloned())
    }

    #[tracing::instrument(skip_all, fields(execution_id = %start.execution_id()), err)]
    async fn materialize_start(
        &self,
        start: &nebula_storage_port::dto::MaterializedStart<'_>,
    ) -> Result<StartMaterialization, nebula_storage_port::store::StartMaterializationError> {
        use nebula_storage_port::store::StartMaterializationError;
        let mut state = self.inner.lock();
        let trigger = start.trigger().map(|key| {
            (
                start.scope().workspace_id.clone(),
                start.scope().org_id.clone(),
                key.trigger_id().to_owned(),
                key.event_id().to_owned(),
            )
        });
        if let Some(execution_id) = trigger.as_ref().and_then(|key| state.dedup.get(key)) {
            return Ok(StartMaterialization::Replayed {
                execution_id: execution_id.clone(),
            });
        }
        let key = start.idempotency().map(|key| {
            (
                start.scope().workspace_id.clone(),
                start.scope().org_id.clone(),
                key.key().to_owned(),
            )
        });
        if let Some(existing) = key.as_ref().and_then(|key| state.start_keys.get(key)) {
            return Ok(
                if start.idempotency().is_some_and(|key| {
                    key.fingerprint().version() == existing.fingerprint_version
                        && key.fingerprint().digest() == &existing.fingerprint
                }) {
                    StartMaterialization::Replayed {
                        execution_id: existing.execution_id.clone(),
                    }
                } else {
                    StartMaterialization::FingerprintMismatch
                },
            );
        }
        let header = crate::start_materialization::validate_envelope(start)?;
        let commitment = crate::start_materialization::commitment(start)?;
        if let Some(existing) = state.materialized_starts.get(start.execution_id()) {
            return if existing.bundle.scope() == start.scope() && existing.commitment == commitment
            {
                Ok(StartMaterialization::Replayed {
                    execution_id: start.execution_id().to_owned(),
                })
            } else {
                Err(StartMaterializationError::MaterializationConflict)
            };
        }
        if state.rows.contains_key(start.execution_id())
            || state.queue.contains_key(&start.command().id)
            || state
                .materialized_bundle_owners
                .contains_key(&start.bundle().identity().bundle_id())
        {
            return Err(StartMaterializationError::MaterializationConflict);
        }
        let ids = start.bundle().identity().revisions();
        if let Err(error) =
            super::plan_flavor_catalog::require_active_pair(&state.revision_catalog, ids)
        {
            let rejection = match error {
                InternalRevisionError::PlanUnavailable => StartRevisionRejection::PlanUnavailable,
                InternalRevisionError::WorkerFlavorUnavailable => {
                    StartRevisionRejection::WorkerFlavorUnavailable
                },
                _ => StartRevisionRejection::PairNotAdmitted,
            };
            return Ok(StartMaterialization::RevisionRejected(rejection));
        }
        let plan = super::plan_flavor_catalog::load_pair(&state.revision_catalog, ids)
            .map_err(|_| StartMaterializationError::InvalidEnvelope)?;
        crate::start_materialization::validate_catalog_header(start, &header, plan.plan_bytes())?;
        let execution_id = start
            .execution_id()
            .parse::<ExecutionId>()
            .map_err(|_| StartMaterializationError::InvalidEnvelope)?;
        let reference = RevisionReference::new(
            RevisionReferenceOwner::for_execution(execution_id),
            start.bundle().identity().bundle_id(),
            ids,
        );
        crate::execution_state::ensure_execution_state_size(start.execution().initial_state)?;
        if !matches!(
            retain_exact_locked(&mut state, reference),
            Ok(RetainDecision::Retained)
        ) {
            return Err(StartMaterializationError::MaterializationConflict);
        }
        insert_created_row(
            &mut state,
            start.scope(),
            start.execution_id(),
            start.execution().workflow_id,
            start.execution().initial_state,
        )?;
        state.queue.insert(
            start.command().id,
            QueuedMsg {
                msg: start.command().clone(),
                status: "Pending".into(),
                processed_by: None,
                processed_at: None,
                reclaim_count: 0,
                error_message: None,
                claim_generation: 0,
            },
        );
        state.materialized_bundle_owners.insert(
            start.bundle().identity().bundle_id(),
            start.execution_id().to_owned(),
        );
        state.materialized_starts.insert(
            start.execution_id().to_owned(),
            crate::start_materialization::StoredStart {
                bundle: nebula_storage_port::dto::StoredContractBundle::new(
                    start.scope().clone(),
                    start.execution_id().to_owned(),
                    start.bundle().clone(),
                ),
                commitment,
            },
        );
        if let (Some(key), Some(identity)) = (key, start.idempotency()) {
            state.start_keys.insert(
                key,
                StartKeyReservation {
                    fingerprint_version: identity.fingerprint().version(),
                    fingerprint: *identity.fingerprint().digest(),
                    execution_id: start.execution_id().to_owned(),
                    created_at: chrono::Utc::now(),
                },
            );
        }
        if let Some(trigger) = trigger {
            state.dedup.insert(trigger, start.execution_id().to_owned());
        }
        Ok(StartMaterialization::Accepted {
            execution_id: start.execution_id().to_owned(),
        })
    }

    async fn lookup_start(
        &self,
        scope: &nebula_storage_port::Scope,
        key: &str,
    ) -> Result<Option<nebula_storage_port::dto::StartReservation>, StorageError> {
        Ok(self
            .inner
            .lock()
            .start_keys
            .get(&(
                scope.workspace_id.clone(),
                scope.org_id.clone(),
                key.to_owned(),
            ))
            .map(|stored| {
                nebula_storage_port::dto::StartReservation::new(
                    nebula_storage_port::store::StartFingerprint::new(
                        stored.fingerprint_version,
                        stored.fingerprint,
                    ),
                    stored.execution_id.clone(),
                )
            }))
    }

    async fn read_contract_bundle(
        &self,
        scope: &nebula_storage_port::Scope,
        execution_id: &str,
    ) -> Result<Option<nebula_storage_port::dto::StoredContractBundle>, StorageError> {
        Ok(self
            .inner
            .lock()
            .materialized_starts
            .get(execution_id)
            .filter(|stored| stored.bundle.scope() == scope)
            .map(|stored| stored.bundle.clone()))
    }
}

#[async_trait::async_trait]
impl StartReservationMaintenance for InMemoryStartAcceptanceStore {
    async fn evict_reservations_older_than(
        &self,
        retention: Duration,
    ) -> Result<u64, StorageError> {
        let retention = chrono::Duration::from_std(retention).unwrap_or(chrono::Duration::MAX);
        let cutoff = chrono::Utc::now()
            .checked_sub_signed(retention)
            .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC);
        let mut state = self.inner.lock();
        let before = state.start_keys.len();
        state
            .start_keys
            .retain(|_, reservation| reservation.created_at >= cutoff);
        Ok((before - state.start_keys.len()) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn maximal_retention_keeps_existing_reservations_without_panicking() {
        let executions = super::super::InMemoryExecutionStore::new();
        let reservations = InMemoryStartAcceptanceStore::new(&executions);
        reservations.inner.lock().start_keys.insert(
            (
                "workspace".to_owned(),
                "organization".to_owned(),
                "key".to_owned(),
            ),
            StartKeyReservation {
                fingerprint_version: 1,
                fingerprint: [7; 32],
                execution_id: "execution".to_owned(),
                created_at: chrono::Utc::now(),
            },
        );

        let removed = reservations
            .evict_reservations_older_than(Duration::MAX)
            .await
            .unwrap();

        assert_eq!(removed, 0);
        assert_eq!(reservations.inner.lock().start_keys.len(), 1);
    }
}
