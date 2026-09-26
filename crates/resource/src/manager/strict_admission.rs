//! Strict per-acquire credential admission.
//!
//! On a manager configured with a credential availability observer, every
//! new unit of work on a credential-bound row — an acquire, an explicit pool
//! warmup, a background create — first reads the availability of each bound
//! credential (Design CONTRACT: every new credentialed unit reads
//! availability first; no cached admission; an outage denies). The protocol
//! is two-phase so no lock is held across the read:
//!
//! 1. **Read, outside every lock** ([`ManagedResource::read_credentials_strict`]):
//!    capture the row's credential gate ticket, snapshot the bound slots'
//!    installed projections ([`ManagedResource::credential_targets`]), then
//!    read every slot concurrently through the manager's join-next
//!    [`CredentialReads`](super::CredentialReads), each bounded by the
//!    caller's deadline and [`CREDENTIAL_READ_TIMEOUT`]. A refresh in flight
//!    is joined for a bounded wait (the credential crate's
//!    `REFRESH_JOIN_*` bounds).
//! 2. **Decide and apply, under `Manager.admission`**
//!    ([`Manager::apply_strict_reading_under_admission`]): re-snapshot the
//!    installed material, decide per slot ([`decide`], pure), then apply the
//!    gate changes — reopen or readmit slots read usable (only when the row is
//!    suspended or the use revision advanced), suspend slots read blocked —
//!    and refuse the unit when any slot denies.
//!
//! | Slot read | Gate change | Unit |
//! |---|---|---|
//! | available at the installed material | reopen/readmit if suspended or newer use revision | admitted (if the row admits afterwards) |
//! | available or refreshing at newer material | none | `Rebinding` |
//! | available at older material | none | `CheckUnavailable` |
//! | refresh still in flight after the join | none | `RefreshInFlight` |
//! | blocked: reauthentication | suspend (witnessed) | `ReauthRequired` |
//! | blocked: operation in flight / reconciliation | suspend | `OperationBlocked` |
//! | absent / wrong contract | none | `Absent` |
//! | store unavailable, invalid state, timeout | none | `CheckUnavailable` |
//!
//! Several denying slots report the highest-priority reason: `Absent`,
//! `ReauthRequired`, `OperationBlocked`, `Rebinding`, `RefreshInFlight`,
//! `CheckUnavailable`. A strict refusal never takes a recovery-gate ticket.
//!
//! A later unit that waits a long time for capacity after this read is not
//! re-read here; a per-call facade reads per attempt with the same two
//! functions.

use std::time::Duration;

use nebula_core::ResourceKey;
use nebula_credential::{
    CredentialAvailability, CredentialAvailabilityObservation, CredentialBlock, CredentialId,
    CredentialKey, CredentialObserveError, REFRESH_JOIN_FIRST_PAUSE, REFRESH_JOIN_MAX_PAUSE,
    REFRESH_JOIN_WAIT, TenantScope,
};

use super::{
    Manager,
    credential_gate::{CredentialObservedAt, installed_mark},
    credential_reads::{CREDENTIAL_READ_TIMEOUT, CredentialReads, ReadFailure, ReadResult},
};
use crate::{
    error::{CredentialUnavailableReason, Error},
    registry::ManagedHandle,
    resource::Provider,
    runtime::{admission::UseMark, managed::ManagedResource},
};

/// One bound slot to read: whose credential, and what the row installed.
#[derive(Debug, Clone)]
pub(crate) struct CredentialTarget {
    slot: &'static str,
    scope: TenantScope,
    credential_id: CredentialId,
    key: CredentialKey,
}

/// What reading one slot produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotOutcome {
    /// The credential answered.
    Observed(CredentialAvailabilityObservation),
    /// No observation.
    Failed(ReadFailure),
    /// The slot holds material the manager cannot observe (installed
    /// without owner-qualified metadata); fail closed.
    Unobservable,
}

/// The strict read of a row: the gate ticket captured before it and one
/// outcome per bound slot.
#[derive(Debug, Clone)]
pub(crate) struct StrictReading {
    ticket: super::CredentialGateTicket,
    slots: Vec<(&'static str, SlotOutcome)>,
}

/// The decision for one unit, before the gate is touched.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct StrictVerdict {
    /// The highest-priority refusal, if any slot refuses.
    pub(crate) deny: Option<CredentialUnavailableReason>,
    /// Slots read blocked, to suspend.
    pub(crate) suspend: Vec<(
        &'static str,
        CredentialUnavailableReason,
        CredentialObservedAt,
    )>,
    /// Slots read usable at the installed material.
    pub(crate) usable: Vec<(&'static str, CredentialObservedAt)>,
    /// The manager is shutting down.
    pub(crate) cancelled: bool,
}

impl StrictVerdict {
    fn refuse(&mut self, reason: CredentialUnavailableReason) {
        if self.deny.is_none_or(|current| rank(reason) > rank(current)) {
            self.deny = Some(reason);
        }
    }
}

/// Priority of a refusal when several slots refuse (higher wins).
const fn rank(reason: CredentialUnavailableReason) -> u8 {
    match reason {
        CredentialUnavailableReason::Absent => 6,
        CredentialUnavailableReason::ReauthRequired => 5,
        CredentialUnavailableReason::OperationBlocked => 4,
        CredentialUnavailableReason::Rebinding => 3,
        CredentialUnavailableReason::RefreshInFlight => 2,
        CredentialUnavailableReason::CheckUnavailable => 1,
    }
}

/// Decides one unit from the slot outcomes and the material each slot has
/// installed now (`installed(slot)`, `None` when the slot no longer holds an
/// observable projection). Pure: the caller applies the gate changes.
pub(crate) fn decide(
    slots: &[(&'static str, SlotOutcome)],
    installed: impl Fn(&str) -> Option<UseMark>,
) -> StrictVerdict {
    let mut verdict = StrictVerdict::default();
    for &(slot, outcome) in slots {
        let observation = match outcome {
            SlotOutcome::Failed(ReadFailure::Cancelled) => {
                verdict.cancelled = true;
                continue;
            },
            SlotOutcome::Failed(ReadFailure::Observe(
                CredentialObserveError::Absent | CredentialObserveError::WrongCredentialKey,
            )) => {
                verdict.refuse(CredentialUnavailableReason::Absent);
                continue;
            },
            SlotOutcome::Failed(_) | SlotOutcome::Unobservable => {
                verdict.refuse(CredentialUnavailableReason::CheckUnavailable);
                continue;
            },
            SlotOutcome::Observed(observation) => observation,
        };
        let material = observation.material_epoch();
        match observation.availability() {
            CredentialAvailability::Blocked(CredentialBlock::ReauthRequired) => {
                // Read with its use revision: a witnessed denial.
                verdict.suspend.push((
                    slot,
                    CredentialUnavailableReason::ReauthRequired,
                    CredentialObservedAt::from(&observation),
                ));
                verdict.refuse(CredentialUnavailableReason::ReauthRequired);
            },
            CredentialAvailability::Blocked(_) => {
                verdict.suspend.push((
                    slot,
                    CredentialUnavailableReason::OperationBlocked,
                    CredentialObservedAt::new(material),
                ));
                verdict.refuse(CredentialUnavailableReason::OperationBlocked);
            },
            availability @ (CredentialAvailability::Available
            | CredentialAvailability::RefreshInFlight) => match installed(slot) {
                None => verdict.refuse(CredentialUnavailableReason::CheckUnavailable),
                Some(installed) if material > installed.material_epoch() => {
                    verdict.refuse(CredentialUnavailableReason::Rebinding);
                },
                Some(installed) if material < installed.material_epoch() => {
                    verdict.refuse(CredentialUnavailableReason::CheckUnavailable);
                },
                Some(_) if availability == CredentialAvailability::RefreshInFlight => {
                    verdict.refuse(CredentialUnavailableReason::RefreshInFlight);
                },
                Some(_) => verdict
                    .usable
                    .push((slot, CredentialObservedAt::from(&observation))),
            },
            _ => verdict.refuse(CredentialUnavailableReason::CheckUnavailable),
        }
    }
    verdict
}

impl<R: Provider> ManagedResource<R> {
    /// The bound slots a strict read covers, or `None` when the row is not
    /// strict. An unbound slot (never installed) is skipped; a slot whose
    /// material carries no owner-qualified metadata is unobservable.
    pub(crate) fn credential_targets(&self) -> Option<Vec<Result<CredentialTarget, &'static str>>> {
        self.credential_reads.as_ref()?;
        Some(
            R::credential_slot_names()
                .iter()
                .filter_map(
                    |&slot| match self.resource.credential_slot_projection(slot) {
                        Some((0, None)) => None,
                        Some((_, Some(metadata))) => match metadata.scope() {
                            Some(scope) => Some(Ok(CredentialTarget {
                                slot,
                                scope: scope.clone(),
                                credential_id: metadata.credential_id(),
                                key: metadata.credential_key().clone(),
                            })),
                            None => Some(Err(slot)),
                        },
                        _ => Some(Err(slot)),
                    },
                )
                .collect(),
        )
    }

    /// Phase 1 of strict admission: reads every bound slot's availability,
    /// outside every lock. `None` when the row is not strict or has no bound
    /// slot — zero reads. `remaining` is the caller's budget.
    ///
    /// Cancel safe: dropping the future drops its reads; nothing is changed.
    pub(crate) async fn read_credentials_strict(
        &self,
        remaining: Option<Duration>,
    ) -> Option<StrictReading> {
        let reads = self.credential_reads.as_deref()?;
        // Captured before reading: a suspension recorded after it supersedes
        // what this read may reopen.
        let ticket = super::CredentialGateTicket::new(self.admission.gate_epoch());
        let targets = self.credential_targets()?;
        if targets.is_empty() {
            return None;
        }
        let started = tokio::time::Instant::now();
        // A budget past the representable horizon is no budget.
        let deadline = remaining.and_then(|remaining| started.checked_add(remaining));
        let span = tracing::debug_span!(
            "resource.credential_admission",
            resource.key = %R::key(),
            slots = targets.len(),
        );
        let slots = tracing::Instrument::instrument(
            futures::future::join_all(targets.into_iter().map(|target| async move {
                match target {
                    Ok(target) => (
                        target.slot,
                        read_slot(reads, &target, started, deadline).await,
                    ),
                    Err(slot) => (slot, SlotOutcome::Unobservable),
                }
            })),
            span,
        )
        .await;
        Some(StrictReading { ticket, slots })
    }

    /// Whether a background create (the registration warmup, the
    /// maintenance refill) may build instances now: one strict read per
    /// pass, admitting only when every bound slot is usable at the installed
    /// material. It changes no gate state — a blocked or unreadable
    /// credential just builds nothing; the next acquire's read applies what
    /// it sees. Always `true` for a row that is not strict.
    pub(crate) async fn credentials_admit_creation(&self) -> bool {
        let Some(reading) = self.read_credentials_strict(None).await else {
            return true;
        };
        let verdict = decide(&reading.slots, |slot| installed_mark(self, slot));
        let Some(reason) = verdict.deny else {
            return !verdict.cancelled;
        };
        tracing::debug!(
            resource.key = %R::key(),
            %reason,
            "strict credential read: background create skipped"
        );
        if let Some(metrics) = self
            .credential_reads
            .as_deref()
            .and_then(CredentialReads::metrics)
        {
            metrics.record_denied(reason);
        }
        false
    }
}

/// Reads one slot, joining a refresh in flight for a bounded wait.
async fn read_slot(
    reads: &CredentialReads,
    target: &CredentialTarget,
    started: tokio::time::Instant,
    deadline: Option<tokio::time::Instant>,
) -> SlotOutcome {
    let read_deadline = || {
        let bound = tokio::time::Instant::now() + CREDENTIAL_READ_TIMEOUT;
        deadline.map_or(bound, |deadline| deadline.min(bound))
    };
    let join_deadline = {
        let join = started + REFRESH_JOIN_WAIT;
        deadline.map_or(join, |deadline| deadline.min(join))
    };
    let mut pause = REFRESH_JOIN_FIRST_PAUSE;
    loop {
        let result: ReadResult = reads
            .read_after_arrival(
                &target.scope,
                target.credential_id,
                &target.key,
                read_deadline(),
            )
            .await;
        match result {
            Ok(observation)
                if observation.availability() == CredentialAvailability::RefreshInFlight
                    && tokio::time::Instant::now() + pause <= join_deadline =>
            {
                tokio::time::sleep(pause).await;
                pause = (pause * 2).min(REFRESH_JOIN_MAX_PAUSE);
            },
            Ok(observation) => return SlotOutcome::Observed(observation),
            Err(failure) => return SlotOutcome::Failed(failure),
        }
    }
}

impl Manager {
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
        let verdict = decide(&reading.slots, |slot| installed_mark(managed, slot));
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
                    self.reopen_under_admission(key, slot, reading.ticket, observed, managed)?;
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
        Err(Self::credential_unavailable_error(key, reason))
    }

    /// Both phases for a caller outside the acquire pipeline (an explicit
    /// warmup): reads, then applies under a short `Manager.admission` hold
    /// and refuses while the row is suspended. `Ok` for a row that is not
    /// strict.
    pub(crate) async fn strict_credential_admission<R: Provider>(
        &self,
        managed: &std::sync::Arc<ManagedResource<R>>,
        remaining: Option<Duration>,
    ) -> Result<(), Error> {
        let Some(reads) = managed.credential_reads.as_deref() else {
            return Ok(());
        };
        let Some(reading) = managed.read_credentials_strict(remaining).await else {
            return Ok(());
        };
        let _admission = self
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.reject_if_tainted_or_shutting_down_post_count::<R>(managed)?;
        self.apply_strict_reading_under_admission(&R::key(), &**managed, &reading, reads)?;
        if let Some(suspension) = managed.admission.suspension() {
            return Err(Self::credential_unavailable_error(
                &R::key(),
                suspension.reason(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "strict_admission_tests.rs"]
mod tests;
