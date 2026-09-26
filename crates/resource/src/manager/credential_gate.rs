//! Credential suspension of credential-bound rows.
//!
//! A credential can deny use without changing its material: it needs
//! reauthentication, or an operation (a revoke in flight, one awaiting
//! reconciliation) blocks use. A row bound to it must then stop admitting
//! work at once, yet keep its physical owners for reuse when the credential
//! is usable again at the same material (Design CONTRACT "same-material
//! block": cooperative cancellation of every admitted unit; retained
//! physical owners remain; a new admission generation is required).
//!
//! [`Manager::suspend_credential_row`] records the denying slot and closes
//! every admission generation of the row's current span (see
//! `runtime::admission`). While suspended the row refuses acquires with
//! [`ErrorKind::CredentialUnavailable`](crate::ErrorKind::CredentialUnavailable),
//! builds no instance (warmup, refill), skips idle health probes (a probe
//! authenticates), and still evicts stale or expired idle entries.
//! [`Manager::reopen_credential_row`] clears a slot when the caller's
//! [`CredentialGateTicket`] — captured before it observed the credential —
//! is still current; clearing the last slot publishes a fresh generation.
//! A material advance instead goes through the ordinary refresh install.
//!
//! Only suspensions advance the ticket counter, so a deny observed late
//! always lands while an admit observed before a later deny is refused.
//! Taint wins: a tainted row reports [`CredentialSuspendOutcome::Tainted`]
//! and [`CredentialReopenOutcome::Tainted`] and never reopens.

use nebula_core::{ResourceKey, ScopeLevel};

use super::Manager;
use crate::{
    dedup::SlotIdentity,
    error::{CredentialUnavailableReason, Error},
    events::ResourceEvent,
    registry::ManagedHandle,
    runtime::admission::{ReopenTransition, SuspendTransition, SuspensionFloor, UseMark},
};

/// The credential gate's state when a caller began observing a credential.
///
/// Capture it with [`Manager::credential_gate_ticket`] **before** reading the
/// credential and present it to [`Manager::reopen_credential_row`]; a
/// suspension recorded in between makes the reopen
/// [`Superseded`](CredentialReopenOutcome::Superseded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialGateTicket(u64);

impl CredentialGateTicket {
    pub(crate) fn new(epoch: u64) -> Self {
        Self(epoch)
    }

    pub(crate) fn epoch(self) -> u64 {
        self.0
    }
}

/// Where a credential observation was made: the material epoch and, when the
/// observation carried it, the credential's use revision (admission epoch).
///
/// Build it from what was read: `From<&CredentialGuardMetadata>` for a
/// projected guard (material and use revision),
/// `From<&CredentialAvailabilityObservation>` for a head-only observation
/// (the use revision only from an `Open` status), or [`new`](Self::new) when
/// only the material epoch is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CredentialObservedAt {
    material_epoch: u64,
    admission_epoch: Option<u64>,
}

impl CredentialObservedAt {
    /// An observation at `material_epoch`, without a use revision.
    #[must_use]
    pub const fn new(material_epoch: u64) -> Self {
        Self {
            material_epoch,
            admission_epoch: None,
        }
    }

    /// The same observation, read together with use revision
    /// `admission_epoch`.
    #[must_use]
    pub const fn with_admission_epoch(self, admission_epoch: u64) -> Self {
        Self {
            material_epoch: self.material_epoch,
            admission_epoch: Some(admission_epoch),
        }
    }

    /// The material epoch observed.
    #[must_use]
    pub const fn material_epoch(self) -> u64 {
        self.material_epoch
    }

    /// The use revision observed, when the observation carried one.
    #[must_use]
    pub const fn admission_epoch(self) -> Option<u64> {
        self.admission_epoch
    }
}

impl From<&nebula_credential::CredentialGuardMetadata> for CredentialObservedAt {
    fn from(metadata: &nebula_credential::CredentialGuardMetadata) -> Self {
        Self::new(metadata.material_epoch()).with_admission_epoch(metadata.admission_epoch())
    }
}

impl From<&nebula_credential::CredentialAvailabilityObservation> for CredentialObservedAt {
    fn from(observation: &nebula_credential::CredentialAvailabilityObservation) -> Self {
        let observed = Self::new(observation.material_epoch());
        match observation.admission_epoch() {
            Some(admission_epoch) => observed.with_admission_epoch(admission_epoch),
            None => observed,
        }
    }
}

/// Result of [`Manager::suspend_credential_row`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSuspendOutcome {
    /// The row was admitting and is now suspended; every lease admitted
    /// before observes closing.
    Suspended,
    /// The row was already suspended; the slot's reason is recorded.
    AlreadySuspended,
    /// The observation predates the material the row already installed for
    /// the slot; nothing changed.
    StaleObservation,
    /// A credential revoke already tainted the row; nothing changed.
    Tainted,
}

/// Result of [`Manager::reopen_credential_row`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialReopenOutcome {
    /// The last suspended slot cleared; the row admits work again under a
    /// fresh admission generation.
    Reopened,
    /// The row was not suspended, but the credential's use revision advanced
    /// at the same material past what the row admitted: use was denied and
    /// allowed again unobserved. The row admits new work under a fresh
    /// admission generation; leases admitted before stay open, and nothing is
    /// rebuilt.
    Readmitted,
    /// The slot cleared but another bound slot still suspends the row.
    StillSuspended,
    /// The row was not suspended; nothing changed.
    NotSuspended,
    /// The observation is older than what the row already admitted or was
    /// denied at, or is at another material than the one installed; nothing
    /// changed.
    StaleObservation,
    /// A suspension was recorded after the ticket was captured; nothing
    /// changed. Observe the credential again with a fresh ticket.
    Superseded,
    /// A credential revoke tainted the row; it never reopens.
    Tainted,
}

impl Manager {
    /// Captures the credential gate ticket of row
    /// `(key, scope, slot_identity)`; take it before observing the credential
    /// whose result may reopen the row.
    ///
    /// # Errors
    ///
    /// [`NotFound`](crate::ErrorKind::NotFound) when no such row exists;
    /// [`Cancelled`](crate::ErrorKind::Cancelled) while shutting down.
    pub fn credential_gate_ticket(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: &SlotIdentity,
    ) -> Result<CredentialGateTicket, Error> {
        let _admission = self.lock_admission();
        let managed = self.lookup_any_for_slot_identity_structural(key, scope, slot_identity)?;
        Ok(CredentialGateTicket::new(managed.credential_gate_epoch()))
    }

    /// Suspends row `(key, scope, slot_identity)` because its credential slot
    /// `slot` denies use for `reason`.
    ///
    /// `observed` is where the denial was observed, when known. An
    /// observation older than the material the row installed for `slot`, or
    /// (with a use revision) older than the revision the slot already
    /// admitted at, is ignored
    /// ([`StaleObservation`](CredentialSuspendOutcome::StaleObservation)).
    /// A denial read with its use revision must be beaten by a strictly
    /// newer revision to reopen; one read without (an operation in flight,
    /// a resolver error) by the revision the slot had admitted or newer.
    ///
    /// # Errors
    ///
    /// [`NotFound`](crate::ErrorKind::NotFound) when no such row exists,
    /// [`Permanent`](crate::ErrorKind::Permanent) when `slot` is not one of
    /// the row's declared credential slots, and
    /// [`Cancelled`](crate::ErrorKind::Cancelled) while shutting down.
    pub fn suspend_credential_row(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: &SlotIdentity,
        slot: &str,
        reason: CredentialUnavailableReason,
        observed: Option<CredentialObservedAt>,
    ) -> Result<CredentialSuspendOutcome, Error> {
        let _admission = self.lock_admission();
        let managed = self.lookup_any_for_slot_identity_structural(key, scope, slot_identity)?;
        self.suspend_under_admission(key, slot, reason, observed, &*managed)
    }

    /// Records that row `(key, scope, slot_identity)`'s credential slot
    /// `slot` is usable at `observed`.
    ///
    /// On a suspended row this clears the slot's suspension, provided
    /// `ticket` is still current and `observed` is newer than the denial
    /// (see [`suspend_credential_row`](Self::suspend_credential_row)).
    /// Clearing the last suspended slot reopens the row without rebuilding
    /// anything: idle and retained instances are reused under a fresh
    /// admission generation, while leases admitted before the suspension stay
    /// closed.
    ///
    /// On an admitting row an observation at a newer use revision than the
    /// row admitted at (same material) means use was denied and allowed again
    /// without this row observing it: the row
    /// [`Readmitted`](CredentialReopenOutcome::Readmitted) new work under a
    /// fresh admission generation, leaving earlier leases open.
    ///
    /// An observation at another material than the one installed changes
    /// nothing ([`StaleObservation`](CredentialReopenOutcome::StaleObservation)):
    /// new material reaches the row through its install.
    ///
    /// # Errors
    ///
    /// As [`suspend_credential_row`](Self::suspend_credential_row).
    pub fn reopen_credential_row(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: &SlotIdentity,
        slot: &str,
        ticket: CredentialGateTicket,
        observed: CredentialObservedAt,
    ) -> Result<CredentialReopenOutcome, Error> {
        let _admission = self.lock_admission();
        let managed = self.lookup_any_for_slot_identity_structural(key, scope, slot_identity)?;
        self.reopen_under_admission(key, slot, ticket, observed, &*managed)
    }

    /// Suspends the exact row `pinned` that the fan-out resolved for
    /// `binding`, revalidating reverse-index ownership in the same
    /// lifecycle-admission critical section. A replacement row with
    /// identical routing keys never receives this result.
    #[cfg(feature = "rotation")]
    pub(crate) fn suspend_published_credential_binding(
        &self,
        index: &crate::ResourceFanoutIndex,
        credential_id: &nebula_credential::CredentialId,
        binding: &crate::Bind,
        pinned: &std::sync::Arc<dyn ManagedHandle>,
        reason: CredentialUnavailableReason,
        observed: Option<CredentialObservedAt>,
    ) -> Result<CredentialSuspendOutcome, Error> {
        let _admission = self.lock_admission();
        let managed = self.revalidate_published_binding(index, credential_id, binding, pinned)?;
        self.suspend_under_admission(
            &binding.resource_key,
            &binding.slot_name,
            reason,
            observed,
            &*managed,
        )
    }

    /// Reopens the exact row `pinned`; see
    /// [`suspend_published_credential_binding`](Self::suspend_published_credential_binding).
    #[cfg(feature = "rotation")]
    pub(crate) fn reopen_published_credential_binding(
        &self,
        index: &crate::ResourceFanoutIndex,
        credential_id: &nebula_credential::CredentialId,
        binding: &crate::Bind,
        pinned: &std::sync::Arc<dyn ManagedHandle>,
        ticket: CredentialGateTicket,
        observed: CredentialObservedAt,
    ) -> Result<CredentialReopenOutcome, Error> {
        let _admission = self.lock_admission();
        let managed = self.revalidate_published_binding(index, credential_id, binding, pinned)?;
        self.reopen_under_admission(
            &binding.resource_key,
            &binding.slot_name,
            ticket,
            observed,
            &*managed,
        )
    }

    /// Caller holds `Manager.admission`.
    #[cfg(feature = "rotation")]
    fn revalidate_published_binding(
        &self,
        index: &crate::ResourceFanoutIndex,
        credential_id: &nebula_credential::CredentialId,
        binding: &crate::Bind,
        pinned: &std::sync::Arc<dyn ManagedHandle>,
    ) -> Result<std::sync::Arc<dyn ManagedHandle>, Error> {
        self.shutdown_guard()?;
        if !index.contains_published_binding(credential_id, binding) {
            return Err(Error::not_found(&binding.resource_key));
        }
        let managed = self.lookup_any_for_slot_identity_structural(
            &binding.resource_key,
            &binding.scope,
            &binding.slot_identity,
        )?;
        if !std::sync::Arc::ptr_eq(&managed, pinned) {
            return Err(Error::not_found(&binding.resource_key));
        }
        Ok(managed)
    }

    fn lock_admission(&self) -> std::sync::MutexGuard<'_, ()> {
        self.admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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

    /// Reopens (or readmits) `slot` after a credential install proved it
    /// usable at `observed`, when the installer captured a ticket. Best
    /// effort: the install already succeeded, so a refused or failed reopen
    /// is only logged. Caller holds `Manager.admission`.
    pub(crate) fn reopen_after_install(
        &self,
        key: &ResourceKey,
        slot: &str,
        ticket: Option<CredentialGateTicket>,
        observed: CredentialObservedAt,
        managed: &dyn ManagedHandle,
    ) {
        let Some(ticket) = ticket else {
            return;
        };
        match self.reopen_under_admission(key, slot, ticket, observed, managed) {
            Ok(outcome) => {
                tracing::debug!(resource.key = %key, slot, ?outcome, "reopen after credential install");
            },
            Err(error) => {
                tracing::debug!(
                    resource.key = %key,
                    slot,
                    error.kind = ?error.kind(),
                    "reopen after credential install skipped"
                );
            },
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
}

/// The use revision of the projection installed in `slot`, if any.
fn installed_mark(managed: &dyn ManagedHandle, slot: &str) -> Option<UseMark> {
    managed
        .credential_slot_projection(slot)
        .and_then(|(_, metadata)| metadata)
        .map(|metadata| UseMark::installed(&metadata))
}
