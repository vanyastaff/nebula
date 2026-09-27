//! The manager's admission state as a lease can reach it.
//!
//! A managed call facade admits each provider attempt long after its lease
//! was acquired, without a `Manager` borrow. [`AdmissionLink`] is the part
//! of the manager an attempt needs for the strict per-attempt credential
//! read's second phase: `Manager.admission` (the same mutex), the shutdown
//! fence, the manager's cancellation, and the event bus the gate reports
//! on. It is reached through the row's
//! [`CredentialReads`], so only strict rows carry it.
//!
//! The gate changes a strict reading applies — suspend, reopen, readmit —
//! live here, so the acquire pipeline and a facade attempt apply them
//! through one body. Every `*_under_admission` function requires its caller
//! to hold [`lock`](AdmissionLink::lock); none of them awaits.

use std::sync::{
    Arc, Mutex, MutexGuard, PoisonError,
    atomic::{AtomicBool, Ordering},
};

use nebula_core::ResourceKey;
use nebula_eventbus::EventBus;
use tokio_util::sync::CancellationToken;

use super::{
    Manager,
    credential_gate::{
        CredentialGateTicket, CredentialObservedAt, CredentialReopenOutcome,
        CredentialSuspendOutcome, installed_mark,
    },
    credential_reads::CredentialReads,
    strict_admission::{StrictReading, decide},
};
use crate::{
    error::{CredentialUnavailableReason, Error},
    events::ResourceEvent,
    registry::ManagedHandle,
    runtime::admission::{ReopenTransition, SuspendTransition, SuspensionFloor, UseMark},
};

/// The manager's admission lock, shutdown fence, cancellation and event bus,
/// shared with the rows that admit work after acquire. See the module docs.
#[derive(Clone)]
pub(crate) struct AdmissionLink {
    lock: Arc<Mutex<()>>,
    shutting_down: Arc<AtomicBool>,
    cancel: CancellationToken,
    events: Arc<EventBus<ResourceEvent>>,
}

impl std::fmt::Debug for AdmissionLink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdmissionLink")
            .field("shutting_down", &self.shutting_down.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl AdmissionLink {
    pub(crate) fn new(
        lock: Arc<Mutex<()>>,
        shutting_down: Arc<AtomicBool>,
        cancel: CancellationToken,
        events: Arc<EventBus<ResourceEvent>>,
    ) -> Self {
        Self {
            lock,
            shutting_down,
            cancel,
            events,
        }
    }

    /// A link to no manager: its own lock, fence, token and bus (tests).
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        Self::new(
            Arc::new(Mutex::new(())),
            Arc::new(AtomicBool::new(false)),
            CancellationToken::new(),
            Arc::new(EventBus::new(16)),
        )
    }

    /// Takes `Manager.admission`. Never hold the guard across an await.
    pub(crate) fn lock(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The manager's cancellation: fired when shutdown starts.
    pub(crate) fn cancel(&self) -> &CancellationToken {
        &self.cancel
    }

    /// The manager's shutdown fence, as [`Manager::shutdown_guard`].
    ///
    /// # Errors
    ///
    /// `Cancelled` once shutdown started.
    pub(crate) fn shutdown_guard(&self) -> Result<(), Error> {
        if self.shutting_down.load(Ordering::Acquire) || self.cancel.is_cancelled() {
            return Err(Error::cancelled());
        }
        Ok(())
    }

    /// The manager's event bus, for guards that emit on release.
    pub(crate) fn events(&self) -> &Arc<EventBus<ResourceEvent>> {
        &self.events
    }

    /// Best-effort event emission, as the manager's own.
    pub(crate) fn emit(&self, event: ResourceEvent) {
        let _ = self.events.emit(event);
    }

    /// Caller holds `Manager.admission` and resolved `managed` under it.
    pub(crate) fn suspend_under_admission(
        &self,
        key: &ResourceKey,
        slot: &str,
        reason: CredentialUnavailableReason,
        observed: Option<CredentialObservedAt>,
        managed: &dyn ManagedHandle,
    ) -> Result<CredentialSuspendOutcome, Error> {
        if !managed.accepts_credential_slot_name(slot) {
            return Err(Error::unknown_credential_slot(key.clone(), slot));
        }
        if managed.is_tainted() {
            return Ok(CredentialSuspendOutcome::Tainted);
        }
        let installed = installed_mark(managed, slot);
        if let (Some(observed), Some(installed)) = (observed, installed)
            && observed.material_epoch() < installed.material_epoch()
        {
            tracing::debug!(
                resource.key = %key,
                slot,
                observed = observed.material_epoch(),
                installed = installed.material_epoch(),
                "credential suspension ignored: observation predates installed material"
            );
            return Ok(CredentialSuspendOutcome::StaleObservation);
        }
        let admitted = managed.credential_admitted(slot, installed);
        let floor = match observed.and_then(UseMark::observed) {
            // Read with its use revision: a late read older than what the
            // slot admitted since says nothing about the current state.
            Some(mark) if admitted.is_some_and(|admitted| mark < admitted) => {
                tracing::debug!(
                    resource.key = %key,
                    slot,
                    "credential suspension ignored: observation predates the admitted use revision"
                );
                return Ok(CredentialSuspendOutcome::StaleObservation);
            },
            Some(mark) => {
                SuspensionFloor::witnessed(admitted.map_or(mark, |admitted| admitted.max(mark)))
            },
            None => SuspensionFloor::unwitnessed(admitted),
        };
        let recorded = managed
            .credential_suspension()
            .and_then(|suspension| suspension.reason_for(slot));
        match managed.suspend_credential(slot, reason, floor) {
            SuspendTransition::Suspended { closed_through } => {
                tracing::warn!(
                    resource.key = %key,
                    slot,
                    %reason,
                    closed_through,
                    "credential denies use: row suspended, admitted leases closing"
                );
                self.emit(ResourceEvent::CredentialSuspended {
                    key: key.clone(),
                    slot: slot.to_owned(),
                    reason,
                });
                Ok(CredentialSuspendOutcome::Suspended)
            },
            SuspendTransition::Updated => {
                if recorded != Some(reason) {
                    tracing::warn!(
                        resource.key = %key,
                        slot,
                        %reason,
                        "credential denies use: further slot recorded on a suspended row"
                    );
                    self.emit(ResourceEvent::CredentialSuspended {
                        key: key.clone(),
                        slot: slot.to_owned(),
                        reason,
                    });
                }
                Ok(CredentialSuspendOutcome::AlreadySuspended)
            },
            SuspendTransition::Retired => Err(Error::cancelled().with_resource_key(key.clone())),
        }
    }

    /// Caller holds `Manager.admission` and resolved `managed` under it.
    pub(crate) fn reopen_under_admission(
        &self,
        key: &ResourceKey,
        slot: &str,
        ticket: CredentialGateTicket,
        observed: CredentialObservedAt,
        managed: &dyn ManagedHandle,
    ) -> Result<CredentialReopenOutcome, Error> {
        if !managed.accepts_credential_slot_name(slot) {
            return Err(Error::unknown_credential_slot(key.clone(), slot));
        }
        if managed.is_tainted() {
            return Ok(CredentialReopenOutcome::Tainted);
        }
        // Read before the gate: the slot is never touched under its mutex.
        let installed = installed_mark(managed, slot);
        Ok(
            match managed.reopen_credential(slot, ticket.epoch(), observed, installed) {
                ReopenTransition::Reopened { seq } => {
                    tracing::info!(
                        resource.key = %key,
                        slot,
                        admission = seq,
                        "credential usable again: row reopened under a fresh admission generation"
                    );
                    self.emit(ResourceEvent::CredentialReopened { key: key.clone() });
                    CredentialReopenOutcome::Reopened
                },
                ReopenTransition::Readmitted { seq } => {
                    tracing::info!(
                        resource.key = %key,
                        slot,
                        admission = seq,
                        "credential use revision advanced unobserved: new work admitted under a \
                         fresh admission generation"
                    );
                    CredentialReopenOutcome::Readmitted
                },
                ReopenTransition::StillSuspended => CredentialReopenOutcome::StillSuspended,
                ReopenTransition::NotSuspended => CredentialReopenOutcome::NotSuspended,
                ReopenTransition::StaleObservation => {
                    tracing::debug!(
                        resource.key = %key,
                        slot,
                        "credential reopen ignored: observation predates the row's use revision"
                    );
                    CredentialReopenOutcome::StaleObservation
                },
                ReopenTransition::Superseded => {
                    tracing::debug!(
                        resource.key = %key,
                        slot,
                        "credential reopen superseded by a later suspension"
                    );
                    CredentialReopenOutcome::Superseded
                },
                ReopenTransition::Retired => {
                    return Err(Error::cancelled().with_resource_key(key.clone()));
                },
            },
        )
    }

    /// Phase 2 of strict admission: decides `reading` against the material
    /// the row has installed now and applies the gate changes, then refuses
    /// the unit when a slot denies. Caller holds `Manager.admission`, ran
    /// the post-count taint/shutdown re-check, and checks the row's
    /// suspension and phase afterwards (a reopen may leave another slot
    /// suspended).
    pub(crate) fn apply_strict_reading_under_admission(
        &self,
        key: &ResourceKey,
        managed: &dyn ManagedHandle,
        reading: &StrictReading,
        reads: &CredentialReads,
    ) -> Result<(), Error> {
        let verdict = decide(reading.slots(), |slot| installed_mark(managed, slot));
        if verdict.cancelled {
            return Err(Error::cancelled().with_resource_key(key.clone()));
        }
        // Reopens first: a suspension recorded by this same unit would
        // supersede its own ticket.
        for &(slot, observed) in &verdict.usable {
            let installed = installed_mark(managed, slot);
            let newer_revision = UseMark::observed(observed)
                .is_some_and(|mark| Some(mark) > managed.credential_admitted(slot, installed));
            if managed.credential_suspension().is_some() || newer_revision {
                let outcome =
                    self.reopen_under_admission(key, slot, reading.ticket(), observed, managed)?;
                tracing::debug!(resource.key = %key, slot, ?outcome, "strict credential read: usable");
            }
        }
        for &(slot, reason, observed) in &verdict.suspend {
            let outcome =
                self.suspend_under_admission(key, slot, reason, Some(observed), managed)?;
            tracing::debug!(resource.key = %key, slot, ?outcome, "strict credential read: blocked");
        }
        let Some(reason) = verdict.deny else {
            return Ok(());
        };
        match reason {
            CredentialUnavailableReason::Absent => tracing::warn!(
                resource.key = %key,
                "strict credential read: bound credential absent — new work refused"
            ),
            CredentialUnavailableReason::CheckUnavailable => tracing::warn!(
                resource.key = %key,
                "strict credential read: availability could not be checked — new work refused"
            ),
            _ => {
                tracing::debug!(resource.key = %key, %reason, "strict credential read refused new work");
            },
        }
        if let Some(metrics) = reads.metrics() {
            metrics.record_denied(reason);
        }
        Err(Manager::credential_unavailable_error(key, reason))
    }
}
