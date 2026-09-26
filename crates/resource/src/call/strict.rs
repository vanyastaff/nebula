//! A unit's credential pin: the slot snapshot its attempts run on, captured
//! at the unit's first grant together with the slot generations it came
//! from.
//!
//! [`PinSlots::pin_slots`] loads slots one by one, so a rotation can land
//! while it runs. [`capture_pin`] brackets the pin with the slot generations
//! read before and after it (a seqlock): equal generations mean the pinned
//! material is exactly what those generations installed. A pin that raced a
//! rotation is retried up to [`PIN_CAPTURE_TRIES`] times, then kept marked
//! unstable. [`pin_is_current`] compares the pin's generations against the
//! slots now; a strict attempt checks it under `Manager.admission` before
//! its grant, so no attempt runs on a binding its credential read did not
//! validate.

use super::pin::PinSlots;
use crate::{resource::Provider, runtime::managed::ManagedResource};

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
}

impl<P> UnitPin<P> {
    /// The snapshot every attempt of the unit reads.
    pub(crate) fn pinned(&self) -> &P {
        &self.pinned
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
            };
        }
        tries += 1;
    }
}

/// Whether `pin` still names the slots the row holds: stable, and every
/// slot at the generation it was pinned at. Called under
/// `Manager.admission` (lock order: admission, then each slot's writer
/// lock).
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the strict per-attempt registration checks it; wired in the next change"
    )
)]
pub(crate) fn pin_is_current<R: Provider, P>(
    managed: &ManagedResource<R>,
    pin: &UnitPin<P>,
) -> bool {
    pin.stable && slot_generations(&managed.resource) == pin.generations
}

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
