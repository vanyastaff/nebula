//! Credential slot rotation / revoke: the refresh + two-phase
//! (synchronous-taint then cancellation-safe drain+hook) revoke surface,
//! their shared post-resolution dispatch, and the type-erased `(key, scope)`
//! row resolution helpers the rotation entry points use.
//!
//! This module keeps the public outcome types, the hook settlement shared
//! by both directions, and row lookup; [`refresh`] holds the install and
//! refresh entry points and [`revoke`] the taint / drain / revoke ones.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use nebula_core::{ResourceKey, ScopeLevel};
use nebula_credential::SecretFreeMessage;

use super::Manager;
use crate::{
    error::Error,
    events::ResourceEvent,
    metrics::SlotDispatchMetricOutcome,
    runtime::acquire_loop::{
        RetiredCleanupObservation, SlotHookAdmission, SlotHookDeferral, SlotHookObservation,
        SlotHookSettlement,
    },
};

#[derive(Debug, Clone, Copy)]
enum SlotHookDirection {
    Refresh,
    Revoke,
}

const MAX_SLOT_HOOK_OBSERVATION_HORIZON: Duration = Duration::from_hours(1);

fn slot_hook_observation_deadline(timeout: Duration) -> tokio::time::Instant {
    tokio::time::Instant::from_std(crate::deadline::deadline_after(
        tokio::time::Instant::now().into_std(),
        timeout,
        MAX_SLOT_HOOK_OBSERVATION_HORIZON,
    ))
}

/// Result of the in-flight drain that precedes a slot hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "the preceding drain outcome carries revoke safety information"]
#[non_exhaustive]
pub enum SlotDrainOutcome {
    /// The operation has no drain phase (credential refresh).
    NotRequired,
    /// All in-flight leases drained before the revoke hook was submitted.
    Drained,
    /// The best-effort drain timed out before the revoke hook was submitted.
    /// The hook still ran or became queue-owned, so this is not permission to
    /// retry it.
    #[non_exhaustive]
    TimedOut {
        /// Leases still outstanding when the drain budget elapsed.
        outstanding_leases: u64,
    },
}

/// Accepted outcome of a credential-slot hook dispatch.
///
/// `Err` from a manager slot method means the work was rejected before queue
/// acceptance or an observed hook failed. [`Deferred`](Self::Deferred) means
/// an admitted hook remains queue-owned. [`Abandoned`](Self::Abandoned) is a
/// distinct terminal loss: ownership settled without a hook result and retry
/// is unsafe because execution may already have produced external effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "slot dispatch outcome carries execution, observation, and drain state that must be handled"]
#[non_exhaustive]
pub enum SlotDispatchOutcome {
    /// The hook completed successfully and its result was observed.
    #[non_exhaustive]
    Completed {
        /// Outcome of the revoke drain, or [`SlotDrainOutcome::NotRequired`]
        /// for refresh.
        drain: SlotDrainOutcome,
    },
    /// The admitted hook exceeded its framework-owned execution budget.
    /// This terminal outcome is non-retryable because the hook may already
    /// have produced external effects before cancellation.
    #[non_exhaustive]
    TimedOut {
        /// Outcome of the revoke drain, or [`SlotDrainOutcome::NotRequired`]
        /// for refresh.
        drain: SlotDrainOutcome,
    },
    /// The queue owns an accepted hook whose completion was not observable
    /// by this caller. Queue execution is bounded and best-effort; callers
    /// must not retry this accepted work.
    #[non_exhaustive]
    Deferred {
        /// Outcome of the revoke drain, or [`SlotDrainOutcome::NotRequired`]
        /// for refresh.
        drain: SlotDrainOutcome,
        /// Why the admitted task could not be observed to a terminal result.
        reason: SlotDeferralReason,
    },
    /// The admitted queue task terminated without producing a hook result.
    /// This is a terminal, non-retryable loss rather than observer deferral.
    #[non_exhaustive]
    Abandoned {
        /// Outcome of the revoke drain, or [`SlotDrainOutcome::NotRequired`]
        /// for refresh.
        drain: SlotDrainOutcome,
    },
}

/// Result of an epoch-ordered guard installation and refresh-hook dispatch.
#[derive(Debug)]
#[must_use = "refresh installation and hook outcomes must be observed"]
#[non_exhaustive]
pub enum EpochRefreshOutcome {
    /// A newer guard was installed before the hook was dispatched.
    Applied(SlotDispatchOutcome),
    /// The refresh was stale; neither the slot nor the hook was changed.
    Stale {
        /// Highest material epoch already accepted by the slot.
        current_material_epoch: u64,
    },
}

/// Result of an epoch-ordered slot revoke and its drain/hook tail.
#[derive(Debug)]
#[must_use = "revoke installation and hook outcomes must be observed"]
#[non_exhaustive]
pub enum EpochRevokeOutcome {
    /// The slot was cleared before the resource was tainted and drained.
    Applied(RevokeTail),
    /// The slot was already terminally revoked; no duplicate hook ran.
    AlreadyRevoked,
}

/// Why an admitted credential hook returned [`SlotDispatchOutcome::Deferred`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "deferral reason determines whether the observation budget elapsed"]
#[non_exhaustive]
pub enum SlotDeferralReason {
    /// Dispatch originated inside a cleanup queue where awaiting another
    /// queue receipt could deadlock.
    CleanupContext,
    /// The accepted hook had not started when the caller's post-admission
    /// observation budget elapsed.
    ObservationTimedOut,
}

impl From<SlotHookDeferral> for SlotDeferralReason {
    fn from(value: SlotHookDeferral) -> Self {
        match value {
            SlotHookDeferral::CleanupContext => Self::CleanupContext,
            SlotHookDeferral::ObservationTimedOut => Self::ObservationTimedOut,
        }
    }
}

/// A resource registry row whose credential slot has been **synchronously
/// tainted** by [`Manager::taint_slot`](Manager::taint_slot) /
/// [`Manager::taint_slot_for_identity`](Manager::taint_slot_for_identity) —
/// phase 1 of the
/// two-phase revoke (see the [`manager`](crate::manager) module docs for the
/// canonical invariant and why the taint is synchronous-before-the-tail).
///
/// Holding one is proof the taint already ran to completion: new acquires on
/// this row's credential are already rejected. It is consumed by
/// [`Manager::drain_and_revoke`](Manager::drain_and_revoke) to run the
/// cancellation-safe drain + revoke-hook tail.
///
/// Opaque by design: the only valid use is to pass it to
/// [`drain_and_revoke`](Manager::drain_and_revoke). It is **not** `Clone` —
/// one taint maps to exactly one drain/revoke tail.
#[must_use = "a TaintedSlot only completes the revoke when passed to Manager::drain_and_revoke"]
pub struct TaintedSlot {
    /// Structural key of the tainted resource registry row (span/event
    /// label only — no credential material).
    pub(super) key: ResourceKey,
    /// The credential slot on that row that was revoked.
    pub(super) slot: String,
    /// The resolved row whose taint flag was already set synchronously.
    pub(super) managed: Arc<dyn crate::registry::ManagedHandle>,
    /// When the synchronous taint was applied — the drain/revoke duration
    /// metric spans from here so it covers the whole revoke, not just the
    /// awaited tail.
    pub(super) tainted_at: Instant,
}

impl std::fmt::Debug for TaintedSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately omits `managed` (not `Debug`, and an internal
        // erased handle); only the credential-free routing labels.
        f.debug_struct("TaintedSlot")
            .field("key", &self.key)
            .field("slot", &self.slot)
            .finish_non_exhaustive()
    }
}

/// Outcome of the cancellation-safe revoke tail
/// ([`Manager::drain_and_revoke`]).
///
/// The tail has exactly one owner of the per-resource time budget (the
/// `drain_timeout` argument): the drain wait is bounded by it
/// (best-effort — a timed-out drain still proceeds to the hook), and the
/// revoke hook is *separately* bounded by it. There is **no** caller-side
/// `tokio::time::timeout` wrapping the whole tail; the hook states
/// are reported here rather than inferred from a dropped outer future. See
/// the [`manager`](crate::manager) module docs for why an outer timeout
/// wrapper would be unsafe (it could drop the future before the hook ran).
/// Four variants are terminal hook results; [`Deferred`](Self::Deferred) is
/// the sole non-terminal observer result:
///
/// - [`Done`](Self::Done) — the revoke hook completed `Ok`.
/// - [`HookFailed`](Self::HookFailed) — the hook returned `Err` (carried
///   verbatim).
/// - [`HookTimedOut`](Self::HookTimedOut) — the hook itself did not
///   complete within the budget. The row stays tainted (the taint ran in
///   the synchronous phase-1); only a *hung hook* is bounded, never the
///   taint, and never at the cost of skipping a hook after a slow drain.
/// - [`Deferred`](Self::Deferred) — the queue accepted the hook work, but
///   completion cannot be observed from the current cleanup context.
/// - [`Abandoned`](Self::Abandoned) — admitted ownership settled without a
///   hook result; retry is unsafe.
#[derive(Debug)]
#[must_use = "the revoke tail outcome must be recorded (it is not a silent success)"]
#[non_exhaustive]
pub enum RevokeTail {
    /// Drain + revoke hook completed; the hook returned `Ok`. (A
    /// best-effort drain timeout that still reached a successful hook is
    /// still `Done` — the drain timeout is non-fatal.)
    #[non_exhaustive]
    Done {
        /// Outcome of the best-effort drain that preceded the hook.
        drain: SlotDrainOutcome,
    },
    /// The revoke hook returned an error. The row stays tainted; the
    /// inner error is preserved for the caller's outcome accounting.
    #[non_exhaustive]
    HookFailed {
        /// Observed provider or topology hook error.
        error: Error,
        /// Outcome of the best-effort drain that preceded the hook.
        drain: SlotDrainOutcome,
    },
    /// The revoke hook did not complete within the per-resource budget
    /// (a wedged `on_credential_revoke`). The row stays tainted; this is
    /// the only thing the budget bounds.
    #[non_exhaustive]
    HookTimedOut {
        /// Outcome of the best-effort drain that preceded the hook.
        drain: SlotDrainOutcome,
    },
    /// The queue accepted the revoke hook, but it remains queue-owned and
    /// has not completed yet. This is neither hook success nor failure and
    /// must not be retried.
    #[non_exhaustive]
    Deferred {
        /// Outcome of the best-effort drain that preceded queue acceptance.
        drain: SlotDrainOutcome,
        /// Why the admitted hook was not observed to a terminal result.
        reason: SlotDeferralReason,
    },
    /// The admitted queue task terminated without producing a hook result.
    #[non_exhaustive]
    Abandoned {
        /// Outcome of the best-effort drain that preceded the hook.
        drain: SlotDrainOutcome,
    },
}

impl Manager {
    fn slot_hook_settlement(
        &self,
        key: ResourceKey,
        slot: String,
        direction: SlotHookDirection,
    ) -> (SlotHookSettlement, SlotHookAdmission) {
        let hook_metrics = self.metrics.clone();
        let hook_event_bus = Arc::clone(&self.event_bus);
        let cleanup_metrics = self.metrics.clone();
        let cleanup_event_bus = Arc::clone(&self.event_bus);
        let cleanup_key = key.clone();
        let cleanup_slot = slot.clone();
        let (settlement, admission) = SlotHookSettlement::new(Box::new(move |observation| {
            let metric_outcome = match &observation {
                SlotHookObservation::Completed => SlotDispatchMetricOutcome::Success,
                SlotHookObservation::Failed(_) => SlotDispatchMetricOutcome::Failed,
                SlotHookObservation::TimedOut(_) => SlotDispatchMetricOutcome::TimedOut,
                SlotHookObservation::Abandoned => SlotDispatchMetricOutcome::Abandoned,
            };
            if let Some(metrics) = &hook_metrics {
                match direction {
                    SlotHookDirection::Refresh => {
                        metrics.record_slot_refresh_outcome(metric_outcome);
                    },
                    SlotHookDirection::Revoke => {
                        metrics.record_slot_revoke_outcome(metric_outcome);
                    },
                }
            }

            let event = match (direction, observation) {
                (SlotHookDirection::Refresh, SlotHookObservation::Completed) => {
                    Some(ResourceEvent::SlotRefreshed { key, slot })
                },
                (SlotHookDirection::Revoke, SlotHookObservation::Completed) => {
                    Some(ResourceEvent::SlotRevoked { key, slot })
                },
                (SlotHookDirection::Refresh, SlotHookObservation::Failed(kind)) => {
                    Some(ResourceEvent::SlotRefreshFailed {
                        key,
                        slot,
                        kind,
                        message: SecretFreeMessage::new("credential refresh hook failed"),
                    })
                },
                (SlotHookDirection::Refresh, SlotHookObservation::TimedOut(kind)) => {
                    Some(ResourceEvent::SlotRefreshFailed {
                        key,
                        slot,
                        kind,
                        message: SecretFreeMessage::new("credential refresh hook timed out"),
                    })
                },
                (SlotHookDirection::Revoke, SlotHookObservation::Failed(kind)) => {
                    Some(ResourceEvent::SlotRevokeFailed {
                        key,
                        slot,
                        kind,
                        message: SecretFreeMessage::new("credential revoke hook failed"),
                    })
                },
                (SlotHookDirection::Revoke, SlotHookObservation::TimedOut(kind)) => {
                    Some(ResourceEvent::SlotRevokeFailed {
                        key,
                        slot,
                        kind,
                        message: SecretFreeMessage::new("credential revoke hook timed out"),
                    })
                },
                (SlotHookDirection::Refresh, SlotHookObservation::Abandoned) => {
                    tracing::warn!(
                        resource.key = %key,
                        slot,
                        ?direction,
                        "admitted credential hook was abandoned before terminal settlement"
                    );
                    Some(ResourceEvent::SlotRefreshFailed {
                        key,
                        slot,
                        kind: crate::ErrorKind::Cancelled,
                        message: SecretFreeMessage::new(
                            "admitted credential hook was abandoned before producing a result",
                        ),
                    })
                },
                (SlotHookDirection::Revoke, SlotHookObservation::Abandoned) => {
                    tracing::warn!(
                        resource.key = %key,
                        slot,
                        ?direction,
                        "admitted credential hook was abandoned before terminal settlement"
                    );
                    Some(ResourceEvent::SlotRevokeFailed {
                        key,
                        slot,
                        kind: crate::ErrorKind::Cancelled,
                        message: SecretFreeMessage::new(
                            "admitted credential hook was abandoned before producing a result",
                        ),
                    })
                },
            };
            if let Some(event) = event {
                let _ = hook_event_bus.emit(event);
            }
        }));
        let settlement = settlement.with_cleanup_observer(Box::new(move |cleanup| {
            if let Some(metrics) = &cleanup_metrics {
                metrics.record_release_error();
            }
            let kind = match cleanup {
                RetiredCleanupObservation::Failed(kind) => {
                    tracing::warn!(
                        resource.key = %cleanup_key,
                        slot = cleanup_slot,
                        ?direction,
                        error.kind = ?kind,
                        "framework-owned retired cleanup failed after credential hook settlement"
                    );
                    kind
                },
                RetiredCleanupObservation::TimedOut => {
                    tracing::warn!(
                        resource.key = %cleanup_key,
                        slot = cleanup_slot,
                        ?direction,
                        "framework-owned retired cleanup timed out after credential hook settlement"
                    );
                    crate::ErrorKind::Backpressure
                },
                RetiredCleanupObservation::Abandoned => {
                    tracing::warn!(
                        resource.key = %cleanup_key,
                        slot = cleanup_slot,
                        ?direction,
                        "framework-owned retired cleanup was abandoned"
                    );
                    crate::ErrorKind::Cancelled
                },
            };
            let _ = cleanup_event_bus.emit(ResourceEvent::RetiredCleanupFailed {
                key: cleanup_key,
                slot: cleanup_slot,
                kind,
            });
        }));
        (settlement, admission)
    }

    /// Type-erased `(key, scope)` → live `ManagedResource` resolution for
    /// the slot-rotation entry points.
    ///
    /// `refresh_slot` / `revoke_slot` take a `ResourceKey` (not a generic
    /// `R`), so they cannot use the typed `lookup::<R>`. This mirrors its
    /// shutdown-race guard (reject once `shutting_down` is observed) and
    /// resolves through the same registry the typed path uses, via the
    /// type-erased `ManagedHandle` view.
    fn lookup_any_for_slot(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
    ) -> Result<Arc<dyn crate::registry::ManagedHandle>, Error> {
        use crate::registry::HandleLookupOutcome;
        self.shutdown_guard()?;
        match self.registry.get_handle(key, scope) {
            HandleLookupOutcome::Found(any) => Ok(any),
            HandleLookupOutcome::NotFound => Err(Error::not_found(key)),
            // Fail closed: do not drive a rotation/revoke hook against an
            // arbitrarily-chosen tenant's row when several resolved-
            // credential rows share this `(key, scope)`. The engine's
            // per-slot fan-out targets the specific resolved row.
            HandleLookupOutcome::Ambiguous { rows } => Err(Error::ambiguous(format!(
                "{key}: {rows} resolved-credential registrations exist at this scope; \
                 slot rotation/revoke must target a resolved row, not an ambiguous \
                 (key, scope)"
            ))
            .with_resource_key(key.clone())),
        }
    }

    /// Returns whether a registry row exists for
    /// `(key, scope bag, slot_identity)`, keyed by the **collision-free
    /// structural** resolved-credential identity.
    ///
    /// This is the engine-facing entry: the engine records a structural
    /// [`SlotIdentity`](crate::dedup::SlotIdentity) at activation and asks
    /// the same structural identity here, so a row is visible *only* under
    /// its exact resolved binding set (no digest aliasing).
    #[must_use]
    pub fn has_registered_for_scope_identity(
        &self,
        key: &ResourceKey,
        scope: &nebula_core::Scope,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> bool {
        use crate::registry::AcquireLookupOutcome;
        if self.shutdown_guard().is_err() {
            return false;
        }
        matches!(
            self.registry.get_acquire_for(key, scope, slot_identity),
            AcquireLookupOutcome::Found { .. }
        )
    }

    /// Returns whether a registry row exists for
    /// `(key, scope level, slot_identity)`, keyed by the **collision-free
    /// structural** resolved-credential identity.
    ///
    /// Prefer
    /// [`has_registered_for_scope_identity`](Self::has_registered_for_scope_identity)
    /// when the full scope bag is available (execution + org/workspace).
    #[must_use]
    pub fn has_registered_for_identity(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> bool {
        let scope_bag = crate::context::minimal_scope_for_level(scope);
        self.has_registered_for_scope_identity(key, &scope_bag, slot_identity)
    }

    /// [`lookup_any_for_slot`](Self::lookup_any_for_slot) pinned to a
    /// resolved per-slot credential identity via
    /// [`Registry::get_for`](crate::registry::Registry::get_for).
    ///
    /// [`get_for`](crate::registry::Registry::get_for) returns the
    /// 2-variant [`PinnedLookup`](crate::registry::PinnedLookup): a
    /// resolved slot identity pins exactly one `(scope, slot_identity)` row
    /// by construction, so there is **no `Ambiguous` case to map** — the
    /// "registry invariant breach" arm the old `u64` digest path had to
    /// fabricate a fail-closed deny for is now type-unrepresentable.
    fn lookup_any_for_slot_identity_structural(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<Arc<dyn crate::registry::ManagedHandle>, Error> {
        use crate::registry::PinnedHandleLookup;
        self.shutdown_guard()?;
        match self.registry.get_handle_for(key, scope, slot_identity) {
            PinnedHandleLookup::Found(any) => Ok(any),
            PinnedHandleLookup::NotFound => Err(Error::not_found(key)),
        }
    }
}

mod refresh;
mod revoke;

#[cfg(test)]
mod tests;
