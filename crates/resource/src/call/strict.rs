//! Strict per-attempt credential admission for the managed call facade, and
//! the unit's credential pin.
//!
//! # Per-attempt read
//!
//! On a manager with a credential observer, every attempt on a
//! credential-bound row reads each bound credential's availability after
//! every wait of the attempt (the quota), before it is registered (Design
//! CONTRACT.md:40-41, QUOTA-DX.md:33). The two phases are the acquire
//! path's (`manager::strict_admission`, invariant I7):
//!
//! 1. [`ManagedLease::read_credentials`] reads outside every lock, through
//!    the manager's join-next reads, bounded by the unit's deadline and the
//!    read timeout, raced against the lease's generation and the unit's
//!    cancel.
//! 2. [`ManagedLease::register`] takes `Manager.admission` (through the
//!    row's [`AdmissionLink`](crate::manager::AdmissionLink)), re-checks
//!    taint and shutdown, applies the reading to the row's gate, checks the
//!    row's suspension, the lease's generation and the unit's pin, and
//!    grants under the lock. Lock order: `Manager.admission`, the row's
//!    gate, each slot's writer lock.
//!
//! Interim managers and rows with no bound slot read nothing and register
//! lock-free. An outage refuses `CheckUnavailable` and changes no gate
//! state (CONTRACT.md:77). Readmission at the installed material with an
//! advanced use revision publishes a fresh generation without closing the
//! lease, so the lease's later attempts are admitted too (the residual of
//! I6).
//!
//! # Pin
//!
//! [`PinSlots::pin_slots`] loads slots one by one, so a rotation can land
//! while it runs. [`capture_pin`] brackets the pin with the slot generations
//! read before and after it (a seqlock): equal generations mean the pinned
//! material is exactly what those generations installed. A pin that raced a
//! rotation is retried up to [`PIN_CAPTURE_TRIES`] times, then kept marked
//! unstable. The pin is captured after the first attempt's read, and
//! [`pin_is_current`] compares its generations against the slots under
//! `Manager.admission` before every strict grant: the first attempt runs on
//! the binding its read validated, and a later attempt whose pin a rotation
//! superseded is refused `Rebinding` without being sent (DX-API.md:136).
//! There is no re-pin mid-unit: whether the unit is retried is the settled
//! outcome's call (a `Write` whose earlier attempt was sent is
//! `OutcomeUnknown`).

use tokio_util::sync::CancellationToken;

use super::{
    error::OperationError,
    managed::{ManagedLease, UnitShared, cancelled_before_grant, generation_refusal},
    pin::PinSlots,
};
use crate::{
    error::{CredentialUnavailableReason, ErrorKind},
    manager::StrictReading,
    resource::Provider,
    runtime::{admission::AdmissionGeneration, managed::ManagedResource},
};

/// Bound on [`capture_pin`]'s retries when a rotation races the pin.
pub(crate) const PIN_CAPTURE_TRIES: usize = 3;

/// The slots one unit runs on, with the generations they were pinned at.
pub(crate) struct UnitPin<P> {
    pinned: P,
    /// Each declared slot's generation, in declaration order; `None` when a
    /// slot has no projection port (the row cannot be read strictly
    /// either).
    generations: Option<Vec<(&'static str, u64)>>,
    /// Whether the generations read before and after the pin matched.
    stable: bool,
    /// The resource's credential slot epoch, read inside the same bracket:
    /// a connection-bound session runs only on an instance built at it.
    slot_epoch: u64,
}

impl<P> UnitPin<P> {
    /// The snapshot every attempt of the unit reads.
    pub(crate) fn pinned(&self) -> &P {
        &self.pinned
    }

    /// The credential slot epoch the unit was pinned at.
    pub(crate) fn slot_epoch(&self) -> u64 {
        self.slot_epoch
    }

    /// Whether the pin saw no rotation while it loaded the slots.
    #[cfg(test)]
    pub(crate) fn is_stable(&self) -> bool {
        self.stable
    }
}

/// Every declared slot's generation, or `None` when a slot does not expose
/// its projection.
fn slot_generations<R: Provider>(resource: &R) -> Option<Vec<(&'static str, u64)>> {
    R::credential_slot_names()
        .iter()
        .map(|&slot| {
            resource
                .credential_slot_projection(slot)
                .map(|(generation, _)| (slot, generation))
        })
        .collect()
}

/// Pins the row's slots for one unit, bracketed by their generations (see
/// the module docs). Never blocks: a pin that keeps racing rotations is
/// returned unstable after [`PIN_CAPTURE_TRIES`] tries.
pub(crate) fn capture_pin<R: Provider + PinSlots>(
    managed: &ManagedResource<R>,
) -> UnitPin<R::Pinned> {
    let resource = &managed.resource;
    let mut tries = 1;
    loop {
        let before = slot_generations(resource);
        let pinned = resource.pin_slots();
        let slot_epoch = resource.credential_slot_epoch();
        let after = slot_generations(resource);
        let stable = before == after;
        if stable || tries >= PIN_CAPTURE_TRIES {
            if !stable {
                tracing::debug!(
                    resource.key = %R::key(),
                    tries,
                    "credential slots kept rotating while a unit pinned them"
                );
            }
            return UnitPin {
                pinned,
                generations: after,
                stable,
                slot_epoch,
            };
        }
        tries += 1;
    }
}

/// Whether `pin` still names the slots the row holds: stable, and every
/// slot at the generation it was pinned at. Called under
/// `Manager.admission` (lock order: admission, then each slot's writer
/// lock).
pub(crate) fn pin_is_current<R: Provider, P>(
    managed: &ManagedResource<R>,
    pin: &UnitPin<P>,
) -> bool {
    pin.stable && slot_generations(&managed.resource) == pin.generations
}

impl<R: Provider + PinSlots> ManagedLease<R> {
    /// Step 4 of an attempt: the strict per-attempt credential read, outside
    /// every lock, raced against the lease's generation (see
    /// [`read_credentials`]).
    pub(super) async fn read_credentials(
        &self,
        deadline: tokio::time::Instant,
        cancel: Option<&CancellationToken>,
    ) -> Result<Option<StrictReading>, OperationError> {
        read_credentials(&self.managed, &self.generation, deadline, cancel).await
    }

    /// Step 6 of an attempt: registers it and grants it under the lease's
    /// generation (see [`register_grant`]). Strict when the attempt read.
    pub(super) fn register(
        &self,
        reading: Option<&StrictReading>,
        pin: &UnitPin<R::Pinned>,
        shared: &UnitShared,
    ) -> Result<(), OperationError> {
        register_grant(
            &self.managed,
            reading.is_some(),
            reading,
            &self.generation,
            pin,
            shared,
        )
    }
}

/// The strict per-attempt credential read of `managed`, outside every lock.
/// `None` — zero reads — on an interim manager and for a row with no bound
/// slot.
///
/// The read is bounded by `deadline` (and by the read's own timeout) and
/// raced, closing first, against `generation` (the unit's) and, until the
/// unit's first grant, against [`Submission::cancel`](super::Submission::cancel).
///
/// # Cancel safety
///
/// Dropping the future drops the read; nothing is changed.
pub(super) async fn read_credentials<R: Provider>(
    managed: &ManagedResource<R>,
    generation: &AdmissionGeneration,
    deadline: tokio::time::Instant,
    cancel: Option<&CancellationToken>,
) -> Result<Option<StrictReading>, OperationError> {
    if managed.credential_reads.is_none() {
        return Ok(None);
    }
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    let read = async { Ok(managed.read_credentials_strict(Some(remaining)).await) };
    let Ok(reading) = managed
        .rate_limiter
        .wait_under(generation, cancel, read)
        .await
    else {
        // The unit's generation closed or the unit was cancelled first.
        generation_refusal(managed, generation)?;
        return Err(cancelled_before_grant());
    };
    Ok(reading)
}

/// The last step of an attempt: registers it and grants it. Synchronous.
///
/// Not `strict` (an attempt that read nothing), this is the local path:
/// `generation`'s admission, then the grant, lock-free. Strict, it runs
/// under `Manager.admission`: taint (`Revoked`), shutdown (`Cancelled`),
/// the `reading` (if any) applied to the row's gate (suspend, reopen,
/// readmit, or the read's refusal), the row's suspension, `generation`'s
/// admission, then the pin — a pin a rotation superseded is refused
/// `Rebinding` — and the grant. Every refusal is `NotSent`.
pub(super) fn register_grant<R: Provider, P>(
    managed: &ManagedResource<R>,
    strict: bool,
    reading: Option<&StrictReading>,
    generation: &AdmissionGeneration,
    pin: &UnitPin<P>,
    shared: &UnitShared,
) -> Result<(), OperationError> {
    let (true, Some(reads)) = (strict, managed.credential_reads.as_deref()) else {
        generation_refusal(managed, generation)?;
        return shared.grant();
    };
    let key = R::key();
    let link = reads.link();
    let _admission = link.lock();
    if managed.is_tainted() {
        return Err(OperationError::new(
            ErrorKind::Revoked,
            "resource tainted by a credential revoke; new attempts refused",
        ));
    }
    link.shutdown_guard().map_err(|_| {
        OperationError::new(
            ErrorKind::Cancelled,
            "manager shutting down; attempt refused",
        )
    })?;
    if let Some(reading) = reading {
        link.apply_strict_reading_under_admission(&key, managed, reading, reads)
            .map_err(OperationError::from)?;
    }
    if let Some(suspension) = managed.admission.suspension() {
        return Err(OperationError::new(
            ErrorKind::CredentialUnavailable {
                reason: suspension.reason(),
            },
            "bound credential unavailable; new attempts refused",
        ));
    }
    generation_refusal(managed, generation)?;
    if !pin_is_current(managed, pin) {
        if let Some(metrics) = reads.metrics() {
            metrics.record_denied(CredentialUnavailableReason::Rebinding);
        }
        tracing::debug!(
            resource.key = %key,
            "credential slots rotated since the unit pinned them; attempt refused"
        );
        return Err(OperationError::new(
            ErrorKind::CredentialUnavailable {
                reason: CredentialUnavailableReason::Rebinding,
            },
            "credential slots rotated since the unit pinned them; attempt refused",
        ));
    }
    shared.grant()
}

#[cfg(test)]
#[path = "../call_strict_tests.rs"]
mod attempt_tests;

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{PIN_CAPTURE_TRIES, capture_pin, pin_is_current};
    use crate::{
        Manager, Resident, ResidentConfig,
        manager::strict_fixtures::{RotatingPin, bind, credential_id, register, row},
    };

    fn rotating(manager: &Manager) -> RotatingPin {
        let resource = register(
            manager,
            RotatingPin::new(),
            Resident::new(ResidentConfig::default()),
        )
        .expect("register");
        bind(&resource.db, credential_id(), 4, 1);
        resource
    }

    #[tokio::test]
    async fn a_quiet_pin_is_stable_and_current_until_the_slot_moves() {
        let manager = Manager::new();
        let resource = rotating(&manager);
        let managed = row::<RotatingPin>(&manager);

        let pin = capture_pin(&managed);
        assert!(pin.is_stable());
        assert_eq!(pin.pinned(), &vec![("db", Some(4))]);
        assert_eq!(resource.pins.load(Ordering::SeqCst), 1);
        assert!(pin_is_current(&managed, &pin));
        assert_eq!(
            pin.slot_epoch(),
            resource.db.generation(),
            "the slot epoch is read in the same bracket"
        );

        bind(&resource.db, credential_id(), 5, 1);
        assert!(
            !pin_is_current(&managed, &pin),
            "a new install supersedes it"
        );
    }

    #[tokio::test]
    async fn a_pin_that_raced_one_rotation_is_retaken_stable() {
        let manager = Manager::new();
        let resource = rotating(&manager);
        let managed = row::<RotatingPin>(&manager);
        resource.pin_rotations.store(1, Ordering::SeqCst);

        let pin = capture_pin(&managed);
        assert!(pin.is_stable());
        assert_eq!(resource.pins.load(Ordering::SeqCst), 2);
        assert!(pin_is_current(&managed, &pin));
    }

    #[tokio::test]
    async fn a_pin_that_keeps_racing_rotations_is_unstable_and_never_current() {
        let manager = Manager::new();
        let resource = rotating(&manager);
        let managed = row::<RotatingPin>(&manager);
        resource.pin_rotations.store(usize::MAX, Ordering::SeqCst);

        let pin = capture_pin(&managed);
        assert!(!pin.is_stable());
        assert_eq!(resource.pins.load(Ordering::SeqCst), PIN_CAPTURE_TRIES);
        resource.pin_rotations.store(0, Ordering::SeqCst);
        assert!(!pin_is_current(&managed, &pin));
    }
}
