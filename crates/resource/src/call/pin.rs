//! Per-unit credential pinning: the slot snapshot one unit of work runs on.

use crate::resource::HasCredentialSlots;

/// Pins a resource's credential slots for one unit of work.
///
/// A unit (one submitted operation) snapshots every `#[credential]` slot once,
/// when it starts, by loading each slot's current `Arc` (`SlotCell::load`).
/// Every attempt of that unit sees the same snapshot: a rotation that lands
/// mid-unit swaps the slot for later units but never changes material under
/// a unit already running (Design CONTRACT.md:73, DX-API.md:136). The pinned
/// `Arc`s keep the old guards alive until the unit ends, however often the
/// slot rotates meanwhile.
///
/// The snapshot is per slot. Two slots are loaded one after the other, so a
/// unit may pin one slot before a rotation and the next after it; nothing
/// promises cross-slot atomicity (Design CONTRACT.md:68). A provider whose
/// slots must change together has to model them as one slot.
///
/// A pinned slot is `None` when the slot was unbound (or revoked) when the
/// unit started. Handle it like any other absent credential: refuse the
/// attempt, never fall back to reading the live cell.
///
/// This is a bound on the managed call facade only, not a [`Provider`]
/// supertrait: rows that never use the facade do not need it.
/// `#[derive(Resource)]` and [`no_credential_slots!`](crate::no_credential_slots)
/// emit it; a hand-written [`HasCredentialSlots`] impl writes one by hand:
///
/// ```
/// use std::sync::Arc;
///
/// use nebula_resource::{HasCredentialSlots, PinSlots, SlotCell};
///
/// struct Api {
///     token: SlotCell<String>,
/// }
///
/// impl HasCredentialSlots for Api {
///     fn credential_slot_epoch(&self) -> u64 {
///         self.token.generation()
///     }
///     fn declares_credential_slots() -> bool {
///         true
///     }
///     fn credential_slot_names() -> &'static [&'static str] {
///         &["token"]
///     }
/// }
///
/// impl PinSlots for Api {
///     type Pinned = Option<Arc<String>>;
///
///     fn pin_slots(&self) -> Self::Pinned {
///         self.token.load()
///     }
/// }
///
/// let api = Api { token: SlotCell::empty() };
/// api.token.store(Arc::new("v1".to_owned()));
/// let pinned = api.pin_slots();
/// api.token.store(Arc::new("v2".to_owned()));
/// assert_eq!(pinned.as_deref().map(String::as_str), Some("v1"));
/// ```
///
/// [`Provider`]: crate::Provider
pub trait PinSlots: HasCredentialSlots {
    /// The snapshot of every slot one unit runs on. `()` for a resource
    /// without credential slots.
    type Pinned: Send + Sync + 'static;

    /// Loads every slot's current value once. Called by the facade when a
    /// unit starts, never per attempt.
    fn pin_slots(&self) -> Self::Pinned;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::PinSlots;
    use crate::{HasCredentialSlots, SlotCell};

    /// A hand-written two-slot resource: the pinned snapshot carries each
    /// slot's generation so a test can tell which epoch a unit runs on.
    struct TwoSlots {
        primary: SlotCell<String>,
        secondary: SlotCell<String>,
    }

    impl HasCredentialSlots for TwoSlots {
        fn credential_slot_epoch(&self) -> u64 {
            self.primary
                .generation()
                .wrapping_mul(31)
                .wrapping_add(self.secondary.generation())
        }
        fn declares_credential_slots() -> bool {
            true
        }
        fn credential_slot_names() -> &'static [&'static str] {
            &["primary", "secondary"]
        }
    }

    impl PinSlots for TwoSlots {
        type Pinned = (Option<(u64, Arc<String>)>, Option<(u64, Arc<String>)>);

        fn pin_slots(&self) -> Self::Pinned {
            (
                self.primary.load_versioned(),
                self.secondary.load_versioned(),
            )
        }
    }

    fn two_slots() -> TwoSlots {
        TwoSlots {
            primary: SlotCell::empty(),
            secondary: SlotCell::empty(),
        }
    }

    #[test]
    fn a_pinned_guard_survives_a_rotation_mid_unit() {
        let resource = two_slots();
        resource.primary.store(Arc::new("primary-v1".to_owned()));
        resource
            .secondary
            .store(Arc::new("secondary-v1".to_owned()));

        let unit = resource.pin_slots();
        resource.primary.store(Arc::new("primary-v2".to_owned()));

        let (generation, value) = unit.0.as_ref().expect("primary was bound");
        assert_eq!(*generation, 1);
        assert_eq!(value.as_str(), "primary-v1", "the unit keeps its material");
        assert_eq!(
            Arc::strong_count(value),
            1,
            "the slot let go of the rotated-out guard; the pin alone keeps it alive"
        );
        assert_eq!(
            resource.primary.load().as_deref().map(String::as_str),
            Some("primary-v2"),
            "the live slot moved on"
        );
    }

    #[test]
    fn the_next_unit_sees_the_new_epoch() {
        let resource = two_slots();
        resource.primary.store(Arc::new("v1".to_owned()));
        let first = resource.pin_slots();
        resource.primary.store(Arc::new("v2".to_owned()));
        let second = resource.pin_slots();

        let first_generation = first.0.as_ref().map(|(generation, _)| *generation);
        let second_generation = second.0.as_ref().map(|(generation, _)| *generation);
        assert_eq!(first_generation, Some(1));
        assert_eq!(second_generation, Some(2));
        assert_eq!(
            second.0.as_ref().map(|(_, value)| value.as_str()),
            Some("v2")
        );
        assert!(first.1.is_none() && second.1.is_none(), "unbound pins None");
    }

    #[test]
    fn slot_less_resources_pin_nothing() {
        struct Plain;
        crate::no_credential_slots!(Plain);

        let (): <Plain as PinSlots>::Pinned = Plain.pin_slots();
        assert!(!Plain::declares_credential_slots());
    }
}
