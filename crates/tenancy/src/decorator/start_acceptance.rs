//! Scope-enforcing [`StartAcceptanceStore`] decorator.

use std::sync::Arc;

use nebula_storage_port::dto::{MaterializedStart, StartReservation, StoredContractBundle};
use nebula_storage_port::store::{
    StartAcceptanceStore, StartMaterialization, StartMaterializationError,
};
use nebula_storage_port::{Scope, StorageError};

/// Forces start materialization and reads into one bound tenant.
#[derive(Clone)]
pub struct ScopedStartAcceptanceStore {
    inner: Arc<dyn StartAcceptanceStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedStartAcceptanceStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScopedStartAcceptanceStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedStartAcceptanceStore {
    /// Bind `inner` to `scope`.
    #[must_use]
    pub fn new(inner: Arc<dyn StartAcceptanceStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl StartAcceptanceStore for ScopedStartAcceptanceStore {
    async fn lookup_trigger_start(
        &self,
        _scope: &Scope,
        key: &nebula_storage_port::dto::TriggerStartKey<'_>,
    ) -> Result<Option<String>, StorageError> {
        self.inner.lookup_trigger_start(&self.bound, key).await
    }

    async fn materialize_start(
        &self,
        start: &MaterializedStart<'_>,
    ) -> Result<StartMaterialization, StartMaterializationError> {
        let mut command = start.command().clone();
        command.scope = self.bound.clone();
        let scoped_start = if let Some(trigger) = start.trigger() {
            MaterializedStart::for_trigger(
                &self.bound,
                trigger,
                start.execution_id(),
                start.execution(),
                &command,
                start.bundle(),
            )
        } else {
            MaterializedStart::new(
                &self.bound,
                start.idempotency(),
                start.execution_id(),
                start.execution(),
                &command,
                start.bundle(),
            )
        };
        self.inner.materialize_start(&scoped_start).await
    }

    async fn lookup_start(
        &self,
        _scope: &Scope,
        key: &str,
    ) -> Result<Option<StartReservation>, StorageError> {
        self.inner.lookup_start(&self.bound, key).await
    }

    async fn read_contract_bundle(
        &self,
        _scope: &Scope,
        execution_id: &str,
    ) -> Result<Option<StoredContractBundle>, StorageError> {
        self.inner
            .read_contract_bundle(&self.bound, execution_id)
            .await
    }
}
