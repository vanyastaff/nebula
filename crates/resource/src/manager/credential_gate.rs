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
//! is still current and the observation is newer than the denial; clearing
//! the last slot publishes a fresh generation. A material advance instead
//! goes through the ordinary refresh install.
//!
//! Only suspensions advance the ticket counter, so a deny observed late
//! always lands while an admit observed before a later deny is refused.
//! Taint wins: a tainted row reports [`CredentialSuspendOutcome::Tainted`]
//! and [`CredentialReopenOutcome::Tainted`] and never reopens.
//!
//! # Use revision
//!
//! Every observation names where it was made, as a [`CredentialObservedAt`]:
//! the material epoch and, from an `Open` credential status, the use
//! revision (admission epoch) the backend advances on every write that
//! closes use (Design CONTRACT: "old use revision does not admit"). The gate
//! keeps, per slot, the highest revision it admitted (the installed
//! projection's, or a later accepted observation) and, while suspended, the
//! revision the denial was read at:
//!
//! - A denial read with its revision (reauthentication required) is
//!   *witnessed*: only a strictly newer revision reopens it, because the
//!   backend advances the revision when the flag clears. An `Available` read
//!   at the same revision is a lagging read and is ignored.
//! - A denial read without a revision (an operation in flight or awaiting
//!   reconciliation, a resolver error) records the admitted revision; an
//!   observation at it or newer reopens (the backend advanced the revision
//!   when that denial began).
//! - An observation older than the admitted revision is
//!   [`StaleObservation`](CredentialReopenOutcome::StaleObservation), for a
//!   suspend as for a reopen; refusals change nothing.
//! - On an admitting row a newer revision at the installed material means a
//!   denial interval nobody observed (an abandoned revoke claim). The row is
//!   [`Readmitted`](CredentialReopenOutcome::Readmitted): new work is
//!   admitted under a fresh generation, while leases admitted before stay
//!   open and nothing is rebuilt. This is a conscious relaxation of "do not
//!   revive cancelled units" for a missed true block — they were never
//!   cancelled. A strict manager narrows the gap to an interval with no
//!   acquire, create, activation or fan-out scan at all, because each
//!   acquire reads the credential first. Readmission is traced, not
//!   published as a [`ResourceEvent`](crate::ResourceEvent).
//!
//! An observation without a revision (an adapter that reports none) falls
//! back to the ticket-only rule.
//!
//! # Who observes
//!
//! On an interim manager ([`CredentialAdmissionProfile::InterimRowGate`])
//! the gate is driven from outside: engine activation, the rotation fan-out
//! and callers of [`Manager::suspend_credential_row`] /
//! [`Manager::reopen_credential_row`]. On a strict manager
//! ([`CredentialAdmissionProfile::StrictPerAcquire`]) every acquire also reads
//! its bound credentials first and applies what it saw through the same
//! `suspend_under_admission` / `reopen_under_admission` rules, with a ticket
//! captured before its read (invariant I7 in the [`manager`](super) docs).

use nebula_core::{ResourceKey, ScopeLevel};

use super::Manager;
use crate::{
    dedup::SlotIdentity,
    error::{CredentialUnavailableReason, Error},
    registry::ManagedHandle,
    runtime::admission::UseMark,
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

/// How a row admits new work against its bound credentials.
///
/// Reported per row in [`ResourceHealthSnapshot`](crate::ResourceHealthSnapshot)
/// and on [`ManagedResourceView`](crate::ManagedResourceView). Chosen at
/// registration from the resource's declared slots and whether the manager
/// was configured with a credential availability observer
/// ([`ManagerConfig::with_credential_observer`](crate::ManagerConfig::with_credential_observer)),
/// then observed: a strict row reports
/// [`StrictPerAttempt`](Self::StrictPerAttempt) once one of its leases became
/// a managed call facade, and keeps it for the row's life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CredentialAdmissionProfile {
    /// The resource declares no credential slots; nothing is read.
    Unbound,
    /// Every new unit of work — an acquire, a create — reads the bound
    /// credentials' availability first and is refused unless each is usable
    /// at the installed material. A credential store outage refuses new
    /// credentialed work.
    StrictPerAcquire,
    /// As [`StrictPerAcquire`](Self::StrictPerAcquire), and the row serves
    /// a managed call facade
    /// ([`Manager::handle`](crate::Manager::handle)):
    /// every provider attempt also reads the bound credentials after its
    /// waits and is refused unless each is usable at the material the unit
    /// pinned.
    StrictPerAttempt,
    /// No availability read before new work: the row admits until a
    /// credential denial reaches it (engine activation, the rotation fan-out
    /// or a caller suspends it). Interim — the production worker is strict;
    /// the default becomes strict before the API freeze.
    InterimRowGate,
}

impl CredentialAdmissionProfile {
    /// Whether this profile is interim surface that a later release replaces.
    #[must_use]
    pub const fn is_interim(self) -> bool {
        matches!(self, Self::InterimRowGate)
    }

    /// Stable lowercase name for logs and status views: `unbound`,
    /// `strict_per_acquire`, `strict_per_attempt` or `interim_row_gate`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unbound => "unbound",
            Self::StrictPerAcquire => "strict_per_acquire",
            Self::StrictPerAttempt => "strict_per_attempt",
            Self::InterimRowGate => "interim_row_gate",
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
        self.link.lock()
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
        self.link
            .suspend_under_admission(key, slot, reason, observed, managed)
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
        self.link
            .reopen_under_admission(key, slot, ticket, observed, managed)
    }
}

/// The use revision of the projection installed in `slot`, if any.
pub(super) fn installed_mark(managed: &dyn ManagedHandle, slot: &str) -> Option<UseMark> {
    managed
        .credential_slot_projection(slot)
        .and_then(|(_, metadata)| metadata)
        .map(|metadata| UseMark::installed(&metadata))
}
