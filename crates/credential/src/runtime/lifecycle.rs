//! Process-lifetime owner for credential lifecycle orchestration.
//!
//! Deployment composition builds the concrete adapters, then transfers the
//! service and every background lifecycle task into this owner. Keeping their
//! shutdown authority together prevents a composition root from accidentally
//! detaching refresh maintenance from the service it protects.

use std::sync::Arc;

use nebula_core::accessor::MetricsEmitter;
use nebula_eventbus::EventBus;

use crate::{CredentialService, LeaseEvent};

use super::{LeaseLifecycle, LeaseLifecycleConfig, ReclaimSweepHandle};

/// Owns the credential service and its process-local lifecycle tasks.
///
/// Durable claims remain the source of truth across process failure. This type
/// owns only process lifetime: dropping it cancels the lease scheduler and
/// aborts the periodic reclaim sweep.
pub struct CredentialLifecycleRuntime {
    service: Arc<CredentialService>,
    lease: LeaseLifecycle,
    reclaim_sweep: ReclaimSweepHandle,
}

impl std::fmt::Debug for CredentialLifecycleRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialLifecycleRuntime")
            .field("reclaim_sweep", &self.reclaim_sweep)
            .finish_non_exhaustive()
    }
}

impl CredentialLifecycleRuntime {
    /// Build one service with the lease scheduler owned by this runtime.
    #[must_use]
    pub fn compose(
        reclaim_sweep: ReclaimSweepHandle,
        lease_config: LeaseLifecycleConfig,
        lease_bus: Option<Arc<EventBus<LeaseEvent>>>,
        metrics: Option<Arc<dyn MetricsEmitter>>,
        build_service: impl FnOnce(LeaseLifecycle) -> Arc<CredentialService>,
    ) -> Self {
        let lease = LeaseLifecycle::spawn(
            lease_config,
            lease_bus,
            metrics,
            tokio_util::sync::CancellationToken::new(),
        );
        let service = build_service(lease.clone());
        Self {
            service,
            lease,
            reclaim_sweep,
        }
    }

    /// Clone the semantic command and projection service handle.
    #[must_use]
    pub fn service(&self) -> Arc<CredentialService> {
        Arc::clone(&self.service)
    }

    /// Stop process-local lifecycle work.
    ///
    /// Durable refresh claims are deliberately left to their storage-defined
    /// expiry and reclaim semantics when work was already past provider egress.
    pub async fn shutdown(&mut self) {
        self.lease.shutdown().await;
        self.reclaim_sweep.shutdown().await;
    }

    /// Whether the periodic reclaim task has stopped.
    #[doc(hidden)]
    #[must_use]
    pub fn reclaim_sweep_is_finished(&self) -> bool {
        self.reclaim_sweep.is_finished()
    }
}

impl Drop for CredentialLifecycleRuntime {
    fn drop(&mut self) {
        self.reclaim_sweep.abort();
    }
}
