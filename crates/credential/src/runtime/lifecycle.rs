//! Process-lifetime owner for credential lifecycle orchestration.
//!
//! Deployment composition builds the concrete adapters, then transfers the
//! service and every background lifecycle task into this owner. Keeping their
//! shutdown authority together prevents a composition root from accidentally
//! detaching refresh maintenance from the service it protects.

use std::sync::Arc;

use nebula_core::accessor::MetricsEmitter;
use nebula_eventbus::EventBus;
use nebula_storage_port::CredentialRefreshSchedule;

use crate::{CredentialService, LeaseEvent};

use super::{
    CredentialRefreshSchedulerConfig, CredentialRefreshSchedulerConfigError, LeaseLifecycle,
    LeaseLifecycleConfig, ReclaimSweepHandle,
    lease::LeaseLifecycleTask,
    refresh::{CredentialRefreshSchedulerTask, ScheduledRefreshExecutor},
};

/// Owns the credential service and its process-local lifecycle tasks.
///
/// Durable claims remain the source of truth across process failure. This type
/// owns only process lifetime: dropping it cancels the lease scheduler and
/// aborts the periodic reclaim sweep.
pub struct CredentialLifecycleRuntime {
    service: Arc<CredentialService>,
    lease_task: LeaseLifecycleTask,
    reclaim_sweep: ReclaimSweepHandle,
    refresh_scheduler: Option<CredentialRefreshSchedulerTask>,
}

impl std::fmt::Debug for CredentialLifecycleRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialLifecycleRuntime")
            .field("reclaim_sweep", &self.reclaim_sweep)
            .field("refresh_scheduler", &self.refresh_scheduler)
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
        let (lease, lease_task) = LeaseLifecycle::spawn_owned(lease_config, lease_bus, metrics);
        let service = build_service(lease);
        Self {
            service,
            lease_task,
            reclaim_sweep,
            refresh_scheduler: None,
        }
    }

    /// Build one service and own its due-refresh, lease, and reclaim tasks.
    ///
    /// The scheduler scans against backend time and may deliver a candidate
    /// more than once. The service rechecks current state before the existing
    /// durable refresh claim admits provider egress.
    pub fn compose_with_refresh_schedule(
        refresh_schedule: Arc<dyn CredentialRefreshSchedule>,
        scheduler_config: CredentialRefreshSchedulerConfig,
        reclaim_sweep: ReclaimSweepHandle,
        lease_config: LeaseLifecycleConfig,
        lease_bus: Option<Arc<EventBus<LeaseEvent>>>,
        metrics: Option<Arc<dyn MetricsEmitter>>,
        build_service: impl FnOnce(LeaseLifecycle) -> Arc<CredentialService>,
    ) -> Result<Self, CredentialRefreshSchedulerConfigError> {
        let (lease, lease_task) = LeaseLifecycle::spawn_owned(lease_config, lease_bus, metrics);
        let service = build_service(lease);
        let scheduler_config =
            scheduler_config.cover_refresh_horizon(service.maximum_refresh_horizon())?;
        let executor: Arc<dyn ScheduledRefreshExecutor> = service.clone();
        let scheduler_metrics = reclaim_sweep.scheduler_metrics();
        let refresh_scheduler = CredentialRefreshSchedulerTask::spawn(
            refresh_schedule,
            executor,
            scheduler_config,
            scheduler_metrics,
        )?;
        Ok(Self {
            service,
            lease_task,
            reclaim_sweep,
            refresh_scheduler: Some(refresh_scheduler),
        })
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
        if let Some(refresh_scheduler) = &mut self.refresh_scheduler {
            refresh_scheduler.shutdown().await;
        }
        self.lease_task.shutdown().await;
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
