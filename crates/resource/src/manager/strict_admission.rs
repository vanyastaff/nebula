//! Strict per-acquire credential admission.
//!
//! On a manager configured with a credential availability observer, every
//! new unit of work on a credential-bound row — an acquire, each create of an
//! explicit pool warmup, each background create — first reads the
//! availability of each bound credential (Design CONTRACT: every new
//! credentialed unit reads
//! availability first; no cached admission; an outage denies). The protocol
//! is two-phase so no lock is held across the read:
//!
//! 1. **Read, outside every lock** ([`ManagedResource::read_credentials_strict`]):
//!    capture the row's credential gate ticket, snapshot the bound slots'
//!    installed projections ([`ManagedResource::credential_targets`]), then
//!    read every slot concurrently through the manager's join-next
//!    [`CredentialReads`] — slots bound to the same credential lane share
//!    one read — each bounded by the
//!    caller's deadline and [`CREDENTIAL_READ_TIMEOUT`]. A refresh in flight
//!    is joined for a bounded wait (the credential crate's
//!    `REFRESH_JOIN_*` bounds), each pause jittered over the upper half of
//!    its step so a fleet that met the same refresh does not re-read in
//!    lockstep.
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

impl StrictReading {
    /// The gate ticket captured before the read.
    pub(crate) fn ticket(&self) -> super::CredentialGateTicket {
        self.ticket
    }

    /// One outcome per bound slot read.
    pub(crate) fn slots(&self) -> &[(&'static str, SlotOutcome)] {
        &self.slots
    }
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
        let slots = if targets.len() == 1 {
            // The common single-slot row: no join set to allocate.
            let mut targets = targets;
            let mut slots = Vec::with_capacity(1);
            match targets.pop() {
                Some(Ok(target)) => {
                    let outcome = tracing::Instrument::instrument(
                        read_slot(reads, &target, started, deadline),
                        span,
                    )
                    .await;
                    slots.push((target.slot, outcome));
                },
                Some(Err(slot)) => slots.push((slot, SlotOutcome::Unobservable)),
                None => {},
            }
            slots
        } else {
            // Slots bound to the same credential lane share one read: two
            // reads of one lane from one unit would run one after the other
            // (join-next) under the same deadline and could refuse falsely.
            let (lanes, mut slots) = group_by_lane(targets);
            let outcomes = tracing::Instrument::instrument(
                futures::future::join_all(
                    lanes
                        .iter()
                        .map(|(target, _)| read_slot(reads, target, started, deadline)),
                ),
                span,
            )
            .await;
            for ((_, lane_slots), outcome) in lanes.iter().zip(outcomes) {
                slots.extend(lane_slots.iter().map(|&slot| (slot, outcome)));
            }
            slots
        };
        Some(StrictReading { ticket, slots })
    }

    /// Whether a background create (the registration warmup, the
    /// maintenance refill) may build one instance now: one strict read per
    /// create, issued immediately before it, admitting only when every bound
    /// slot is usable at the installed material. It changes no gate state —
    /// a blocked or unreadable credential just builds nothing; the next
    /// acquire's read applies what it sees. A row that accepts no new
    /// instances (tainted, suspended, not ready) reads nothing and refuses.
    /// Always `true` for an accepting row that is not strict.
    pub(crate) async fn credentials_admit_creation(&self) -> bool {
        if !self.accepts_new_instances() {
            return false;
        }
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

/// One credential lane to read and the slots it answers for.
type LaneRead = (CredentialTarget, Vec<&'static str>);

/// Groups `targets` by credential lane — `(credential id, owner scope,
/// contract key)` — so each lane is read once per unit. Unobservable slots
/// are returned as outcomes already.
fn group_by_lane(
    targets: Vec<Result<CredentialTarget, &'static str>>,
) -> (Vec<LaneRead>, Vec<(&'static str, SlotOutcome)>) {
    let mut lanes: Vec<LaneRead> = Vec::with_capacity(targets.len());
    let mut slots = Vec::with_capacity(targets.len());
    for target in targets {
        match target {
            Ok(target) => {
                let same_lane = lanes.iter_mut().find(|(lane, _)| {
                    lane.credential_id == target.credential_id
                        && lane.scope == target.scope
                        && lane.key == target.key
                });
                if let Some((_, lane_slots)) = same_lane {
                    lane_slots.push(target.slot);
                } else {
                    let slot = target.slot;
                    lanes.push((target, vec![slot]));
                }
            },
            Err(slot) => slots.push((slot, SlotOutcome::Unobservable)),
        }
    }
    (lanes, slots)
}

/// A refresh-join pause drawn uniformly from `[pause / 2, pause]` by `unit`
/// in `[0, 1]` ("equal jitter"): never longer than the schedule's bound.
pub(crate) fn jittered(pause: Duration, unit: f64) -> Duration {
    let half = pause / 2;
    half + pause.saturating_sub(half).mul_f64(unit.clamp(0.0, 1.0))
}

/// Reads one slot, joining a refresh in flight for a bounded wait. The
/// pauses between re-reads double from `REFRESH_JOIN_FIRST_PAUSE` to
/// `REFRESH_JOIN_MAX_PAUSE`, each jittered below its bound.
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
    // The refresh last observed in flight, once the join has started: a
    // re-read never outlives the join, and one cut off by it answers with
    // this observation (still busy) rather than as an unreadable store.
    let mut joined: Option<CredentialAvailabilityObservation> = None;
    loop {
        let bound = if joined.is_some() {
            read_deadline().min(join_deadline)
        } else {
            read_deadline()
        };
        let result: ReadResult = reads
            .read_after_arrival(&target.scope, target.credential_id, &target.key, bound)
            .await;
        // Jittered below its bound: acquires that met the same refresh do not
        // re-read it in lockstep across the fleet.
        let wait = if reads.jitters_join_pauses() {
            jittered(pause, fastrand::f64())
        } else {
            pause
        };
        match result {
            Ok(observation)
                if observation.availability() == CredentialAvailability::RefreshInFlight
                    && tokio::time::Instant::now() + wait <= join_deadline =>
            {
                joined = Some(observation);
                tokio::time::sleep(wait).await;
                pause = (pause * 2).min(REFRESH_JOIN_MAX_PAUSE);
            },
            Ok(observation) => return SlotOutcome::Observed(observation),
            Err(ReadFailure::TimedOut) if bound == join_deadline => {
                if let Some(observation) = joined {
                    return SlotOutcome::Observed(observation);
                }
                return SlotOutcome::Failed(ReadFailure::TimedOut);
            },
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
        self.link
            .apply_strict_reading_under_admission(key, managed, reading, reads)
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
