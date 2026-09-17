use std::sync::Arc;
use std::time::Instant;

use nebula_storage_port::store::{ClaimToken, RefreshClaimStore as RefreshClaimRepo};
use tokio_util::sync::CancellationToken;

use crate::runtime::refresh::l1::{L1Completion, L1RefreshCoalescer};
use crate::runtime::refresh::metrics::RefreshCoordMetrics;

use super::errors::ClaimFinalization;

pub(super) struct L1RefreshLease {
    l1: Arc<L1RefreshCoalescer>,
    credential_id: Option<String>,
    completion: L1Completion,
    _permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl L1RefreshLease {
    pub(super) fn new(l1: Arc<L1RefreshCoalescer>, credential_id: String) -> Self {
        Self {
            l1,
            credential_id: Some(credential_id),
            completion: L1Completion::NoStateChange,
            _permit: None,
        }
    }

    pub(super) fn attach_permit(&mut self, permit: tokio::sync::OwnedSemaphorePermit) {
        self._permit = Some(permit);
    }

    pub(super) fn set_completion(&mut self, completion: L1Completion) {
        self.completion = completion;
    }
}

impl Drop for L1RefreshLease {
    fn drop(&mut self) {
        if let Some(credential_id) = self.credential_id.take() {
            self.l1.complete(&credential_id, self.completion);
        }
    }
}

/// Owned L2 lease transferred atomically into the provider/persistence task.
///
/// Before transfer, dropping the outer coordination future stops heartbeat and
/// best-effort releases the claim because no provider request has started.
/// After transfer, the detached task owns this guard, so caller cancellation or
/// timeout cannot release the claim before the critical section reports an
/// exact disposition.
pub(super) struct RefreshLease {
    repo: Arc<dyn RefreshClaimRepo>,
    token: Option<ClaimToken>,
    heartbeat_stop: CancellationToken,
    heartbeat_task: Option<tokio::task::JoinHandle<()>>,
    metrics: RefreshCoordMetrics,
    hold_start: Instant,
    release_on_drop: bool,
    _l1: Option<L1RefreshLease>,
}

impl RefreshLease {
    pub(super) fn new(
        repo: Arc<dyn RefreshClaimRepo>,
        token: ClaimToken,
        heartbeat_stop: CancellationToken,
        heartbeat_task: tokio::task::JoinHandle<()>,
        metrics: RefreshCoordMetrics,
        hold_start: Instant,
        l1: L1RefreshLease,
    ) -> Self {
        Self {
            repo,
            token: Some(token),
            heartbeat_stop,
            heartbeat_task: Some(heartbeat_task),
            metrics,
            hold_start,
            release_on_drop: true,
            _l1: Some(l1),
        }
    }

    pub(super) fn enter_provider_critical_section(&mut self) {
        self.release_on_drop = false;
        if let Some(l1) = &mut self._l1 {
            // From the sentinel acknowledgement until an exact disposition,
            // any panic/runtime teardown must wake waiters as genuinely
            // outcome-unknown.
            l1.set_completion(L1Completion::OutcomeUnknown);
        }
    }

    pub(super) async fn finish(
        mut self,
        finalization: ClaimFinalization,
        l1_completion: L1Completion,
    ) {
        if let Some(l1) = &mut self._l1 {
            l1.set_completion(l1_completion);
        }
        self.heartbeat_stop.cancel();
        if let Some(task) = self.heartbeat_task.take() {
            task.abort();
            let _ = task.await;
        }
        self.metrics
            .hold_duration
            .observe(self.hold_start.elapsed().as_secs_f64());

        // The provider/persistence section has an exact disposition. Wake L1
        // waiters and return the global permit *before* touching the L2 release
        // path: a wedged database/pool must not permanently poison the local
        // single-flight entry or consume one global refresh slot.
        drop(self._l1.take());

        let Some(token) = self.token.take() else {
            return;
        };
        if finalization == ClaimFinalization::Release {
            let repo = Arc::clone(&self.repo);
            // Release is best-effort and deliberately detached. The L2 row
            // continues to coalesce other replicas until this completes. If
            // it remains through expiry, storage fails closed instead of
            // treating the stale sentinel as replay authorization, while the exact
            // provider/persistence result can return without a hung release
            // wedging local progress.
            tokio::spawn(async move {
                if let Err(error) = repo.release(token).await {
                    // A release failure never changes an already-confirmed
                    // outcome. This branch is `finalization ==
                    // ClaimFinalization::Release`, which both the pre-provider
                    // cleanup sites and the post-provider state-disposition
                    // sites choose, so the error carries no provider outcome of
                    // its own: only `ReleaseRefused` means the sweep already
                    // accounted the claim's incident. That refusal leaves the
                    // row poison until `adjudicate` records the provider
                    // outcome, and waiting for claim expiry cannot clear it
                    // because the retained row outlives its expiry.
                    tracing::warn!(
                        ?error,
                        "L2 claim release after exact refresh disposition failed"
                    );
                }
            });
        } else {
            tracing::warn!("refresh disposition forbids replay; retaining claim as durable poison");
        }
    }
}

impl Drop for RefreshLease {
    fn drop(&mut self) {
        let Some(token) = self.token.take() else {
            return;
        };
        self.heartbeat_stop.cancel();
        if let Some(task) = self.heartbeat_task.take() {
            task.abort();
        }
        self.metrics
            .hold_duration
            .observe(self.hold_start.elapsed().as_secs_f64());

        if !self.release_on_drop {
            // Panic/runtime cancellation after the sentinel boundary has no
            // trustworthy commit disposition. Releasing here would allow an
            // immediate blind replay, so retain the row exactly like an
            // `OutcomeUnknown` disposition does, leaving the claim durable
            // poison until `adjudicate` records the provider outcome.
            tracing::warn!(
                "provider/persistence task dropped without an exact disposition; \
                 retaining refresh claim as durable poison"
            );
            return;
        }

        let repo = Arc::clone(&self.repo);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    if let Err(error) = repo.release(token).await {
                        tracing::warn!(
                            ?error,
                            "L2 claim release after pre-provider cancellation or task failure failed"
                        );
                    }
                });
            },
            Err(error) => {
                // There is no executor on which an async release can run. The
                // stopped heartbeat guarantees the row expires naturally.
                tracing::warn!(
                    ?error,
                    "no Tokio runtime available for L2 claim release; claim will expire by TTL"
                );
            },
        }
    }
}
