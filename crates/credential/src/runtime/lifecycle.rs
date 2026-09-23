//! Process-lifetime owner for credential lifecycle orchestration.
//!
//! Deployment composition builds the concrete adapters, then transfers the
//! service and every background lifecycle task into this owner. Keeping their
//! shutdown authority together prevents a composition root from accidentally
//! detaching refresh maintenance from the service it protects.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::CredentialService;

use super::ReclaimSweepHandle;

/// Owns the credential service and its process-local lifecycle tasks.
///
/// Durable claims remain the source of truth across process failure. This type
/// owns only process lifetime: dropping it cancels the lease scheduler and
/// aborts the periodic reclaim sweep.
pub struct CredentialLifecycleRuntime {
    service: Arc<CredentialService>,
    reclaim_sweep: ReclaimSweepHandle,
    shutdown: CancellationToken,
}

impl std::fmt::Debug for CredentialLifecycleRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialLifecycleRuntime")
            .field("reclaim_sweep", &self.reclaim_sweep)
            .field("shutdown_requested", &self.shutdown.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl CredentialLifecycleRuntime {
    /// Take ownership of one fully composed service and its maintenance task.
    #[must_use]
    pub fn new(
        service: Arc<CredentialService>,
        reclaim_sweep: ReclaimSweepHandle,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            service,
            reclaim_sweep,
            shutdown,
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
    pub fn shutdown(&self) {
        self.shutdown.cancel();
        self.reclaim_sweep.abort();
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
        self.shutdown();
    }
}
