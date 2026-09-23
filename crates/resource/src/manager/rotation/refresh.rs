//! Credential slot refresh: guard install plus refresh-hook dispatch.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use nebula_core::{ResourceKey, ScopeLevel};
use nebula_credential::SecretFreeMessage;

use super::{
    EpochRefreshOutcome, Manager, SlotDispatchOutcome, SlotDrainOutcome, SlotHookDirection,
    slot_hook_observation_deadline,
};
use crate::{
    error::Error, events::ResourceEvent, metrics::SlotDispatchMetricOutcome,
    runtime::acquire_loop::SlotHookWaitOutcome,
};

impl Manager {
    /// Installs a projected guard into one identity-pinned row, then dispatches
    /// the refresh hook. The install completes synchronously before the first
    /// await, so author code cannot observe the old guard after dispatch.
    ///
    /// A stale epoch is a successful no-op and does not invoke the hook.
    /// Type mismatch or unknown slot leaves the previous guard untouched.
    ///
    /// # Errors
    ///
    /// Returns an error when the identity-pinned resource cannot be resolved,
    /// the slot rejects the guard, or refresh dispatch fails.
    pub async fn install_and_refresh_slot_for_identity(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        slot_identity: &crate::dedup::SlotIdentity,
        guard: nebula_credential::ErasedCredentialGuard,
    ) -> Result<EpochRefreshOutcome, Error> {
        let managed = self.lookup_any_for_slot_identity_structural(key, &scope, slot_identity)?;
        let update = managed
            .install_credential_slot(slot, guard)
            .map_err(|source| {
                Error::permanent("credential slot installation failed")
                    .with_source(source)
                    .with_resource_key(key.clone())
            })?;
        match update {
            crate::SlotUpdate::Installed => self
                .refresh_resolved(
                    key,
                    slot,
                    managed,
                    crate::hook_guard::MAX_ROTATION_DISPATCH_CEILING,
                    crate::hook_guard::MAX_ROTATION_DISPATCH_CEILING,
                )
                .await
                .map(EpochRefreshOutcome::Applied),
            crate::SlotUpdate::Stale {
                current_material_epoch,
            } => Ok(EpochRefreshOutcome::Stale {
                current_material_epoch,
            }),
            crate::SlotUpdate::Revoked | crate::SlotUpdate::AlreadyRevoked => Err(
                Error::permanent("credential slot install returned an invalid revoke outcome")
                    .with_resource_key(key.clone()),
            ),
        }
    }

    /// Notifies a registered resource that one of its `#[credential]`
    /// slots was rotated, after the engine has installed the fresh guard.
    ///
    /// Resolves `(key, scope)` to the live [`ManagedResource`](crate::ManagedResource) via the same
    /// registry lookup the `acquire_*` family uses, then borrows the live
    /// `Instance` per topology and invokes
    /// [`Provider::on_credential_refresh`](crate::resource::Provider::on_credential_refresh)
    /// for `slot`. The slot cell itself
    /// lives on the author's resource struct and is populated/rotated by
    /// the engine through `&self` (`SlotCell::store`) — this method does
    /// **not** own a slot map; it only drives the per-resource hook.
    ///
    /// Emits [`ResourceEvent::SlotRefreshed`] only after terminal completion or
    /// [`ResourceEvent::SlotRefreshFailed`] (with a typed error kind and fixed
    /// credential-free message) on failure, and records the corresponding
    /// slot-refresh metric. [`SlotDispatchOutcome::Deferred`] means the hook
    /// was accepted by the queue but cannot be observed to completion here;
    /// it must not be retried and this call emits neither a false success nor
    /// a false failure event.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource is registered for
    ///   `key` at `scope`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - Whatever the resource's `on_credential_refresh` hook maps into [`Error`].
    ///
    /// # Cancel safety
    ///
    /// Admission happens synchronously on the first poll. Before admission,
    /// dropping the future has no effect. After admission, the release queue
    /// owns the hook and its settlement: dropping this observer cannot cancel
    /// the hook, and the queue-owned settlement still records exactly one
    /// terminal metric and emits the matching terminal event.
    #[tracing::instrument(
        level = "debug",
        name = "nebula.resource.slot_refresh",
        skip(self),
        fields(key = %key, slot = %slot, topology, duration_ms)
    )]
    pub async fn refresh_slot(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
    ) -> Result<SlotDispatchOutcome, Error> {
        let managed = self.lookup_any_for_slot(key, &scope)?;
        self.refresh_resolved(
            key,
            slot,
            managed,
            crate::hook_guard::MAX_ROTATION_DISPATCH_CEILING,
            crate::hook_guard::MAX_ROTATION_DISPATCH_CEILING,
        )
        .await
    }

    /// [`refresh_slot`](Self::refresh_slot) pinned to the **collision-free
    /// structural** resolved per-slot credential identity.
    ///
    /// Resolves the registry row whose `slot_identity` matches (via the same
    /// unambiguous-by-construction path [`get_for`](crate::registry::Registry::get_for)
    /// backs), so a multi-tenant `(key, scope)` routes the rotation to the
    /// *specific* resolved row instead of failing closed with
    /// [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous). This is
    /// the entry point the engine per-slot rotation fan-out drives once it
    /// has resolved a node's slot bindings; identity-agnostic
    /// [`refresh_slot`](Self::refresh_slot) stays fail-closed for the
    /// no-identity caller. The engine rotation fan-out records the
    /// structural [`SlotIdentity`](crate::dedup::SlotIdentity) at bind time,
    /// so routing is by exact string equality (no digest aliasing).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of `key` at `scope`
    ///   matches `slot_identity`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - Whatever the resource's `on_credential_refresh` hook maps into [`Error`].
    ///
    /// # Cancel safety
    ///
    /// Cancel safe — same contract as [`refresh_slot`](Self::refresh_slot).
    #[tracing::instrument(
        level = "debug",
        name = "nebula.resource.slot_refresh",
        skip(self, slot_identity),
        fields(key = %key, slot = %slot, topology, duration_ms)
    )]
    pub async fn refresh_slot_for_identity(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<SlotDispatchOutcome, Error> {
        self.refresh_slot_for_identity_with_timeout(
            key,
            scope,
            slot,
            slot_identity,
            crate::hook_guard::MAX_ROTATION_DISPATCH_CEILING,
            crate::hook_guard::MAX_ROTATION_DISPATCH_CEILING,
        )
        .await
    }

    pub(crate) async fn refresh_slot_for_identity_with_timeout(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        slot_identity: &crate::dedup::SlotIdentity,
        hook_timeout: Duration,
        observation_timeout: Duration,
    ) -> Result<SlotDispatchOutcome, Error> {
        let managed = self.lookup_any_for_slot_identity_structural(key, &scope, slot_identity)?;
        self.refresh_resolved(key, slot, managed, hook_timeout, observation_timeout)
            .await
    }

    /// Post-resolution refresh dispatch shared by
    /// [`refresh_slot`](Self::refresh_slot) (identity-agnostic) and
    /// [`refresh_slot_for_identity`](Self::refresh_slot_for_identity)
    /// (slot-identity-pinned).
    ///
    /// The two public entry points differ only in how they resolve the row;
    /// the hook dispatch, metric (exactly one outcome per dispatch), and
    /// event emission are identical and live here.
    async fn refresh_resolved(
        &self,
        key: &ResourceKey,
        slot: &str,
        managed: Arc<dyn crate::registry::ManagedHandle>,
        hook_timeout: Duration,
        observation_timeout: Duration,
    ) -> Result<SlotDispatchOutcome, Error> {
        let started = Instant::now();
        tracing::Span::current().record("topology", managed.topology_tag().as_str());

        // Unknown-slot validation: reject a slot name the resource type does
        // not declare, before the author's `on_credential_refresh` hook ever
        // dispatches. See `ManagedHandle::accepts_credential_slot_name` — a
        // type that declares no credential slots at all rejects every slot
        // name (fail closed: nothing to rotate). Still recorded as a real
        // `Failed` dispatch outcome and `SlotRefreshFailed` event — a
        // rejected call is observably a failed refresh to any subscriber
        // counting attempts, not a silent no-op that would undercount
        // `attempts` relative to `success + failed + timed_out`.
        if !managed.accepts_credential_slot_name(slot) {
            let err = Error::unknown_credential_slot(key.clone(), slot);
            if let Some(m) = &self.metrics {
                m.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Failed);
            }
            self.emit(ResourceEvent::SlotRefreshFailed {
                key: key.clone(),
                slot: slot.to_owned(),
                kind: err.kind().clone(),
                message: SecretFreeMessage::new(
                    "credential refresh rejected: unknown credential slot",
                ),
            });
            tracing::warn!(error = %err, "slot refresh rejected: unknown credential slot");
            return Err(err);
        }

        // Submission is the ownership boundary. A rejection below happens
        // before admission and is returned as a retryable error. Once accepted,
        // the queue task owns the hook and terminal settlement. The caller only
        // observes its receipt for `observation_timeout` while the hook is
        // queued. Expiry before execution starts returns `Deferred` without
        // cancelling or double-accounting the admitted task. Once execution
        // starts, the caller awaits the hook's independently bounded terminal
        // result.
        let (settlement, admission) =
            self.slot_hook_settlement(key.clone(), slot.to_owned(), SlotHookDirection::Refresh);
        let accepted =
            match Arc::clone(&managed).submit_on_refresh(slot, hook_timeout, settlement, admission)
            {
                Ok(accepted) => accepted,
                Err(error) => {
                    if let Some(metrics) = &self.metrics {
                        metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Failed);
                    }
                    self.emit(ResourceEvent::SlotRefreshFailed {
                        key: key.clone(),
                        slot: slot.to_owned(),
                        kind: error.kind().clone(),
                        message: SecretFreeMessage::new("credential refresh hook admission failed"),
                    });
                    return Err(error);
                },
            };
        // Start the observer budget only after synchronous admission. The
        // provider execution budget starts when the queue dequeues the hook;
        // stamping this deadline before admission would make an equal hook
        // timeout unreachable under queue delay. Receipt polling is biased,
        // so a terminal hook result wins an exact deadline tie.
        let observation_deadline = slot_hook_observation_deadline(observation_timeout);
        let observed = accepted.wait_until(observation_deadline).await;
        tracing::Span::current().record("duration_ms", started.elapsed().as_millis() as u64);
        match observed {
            SlotHookWaitOutcome::Completed => Ok(SlotDispatchOutcome::Completed {
                drain: SlotDrainOutcome::NotRequired,
            }),
            SlotHookWaitOutcome::Deferred(reason) => {
                if let Some(metrics) = &self.metrics {
                    metrics.record_slot_refresh_deferred();
                }
                tracing::debug!(
                    outcome = "deferred",
                    "slot refresh hook is admitted and queue-owned; caller stops observing"
                );
                Ok(SlotDispatchOutcome::Deferred {
                    drain: SlotDrainOutcome::NotRequired,
                    reason: reason.into(),
                })
            },
            SlotHookWaitOutcome::Abandoned => Ok(SlotDispatchOutcome::Abandoned {
                drain: SlotDrainOutcome::NotRequired,
            }),
            SlotHookWaitOutcome::TimedOut(_error) => Ok(SlotDispatchOutcome::TimedOut {
                drain: SlotDrainOutcome::NotRequired,
            }),
            SlotHookWaitOutcome::Failed(error) => Err(error),
        }
    }
}
