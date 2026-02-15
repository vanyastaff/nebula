//! Property tests for lifecycle state machine consistency

use nebula_resource::LifecycleState;
use proptest::prelude::*;

/// Generate an arbitrary LifecycleState
fn arb_lifecycle_state() -> impl Strategy<Value = LifecycleState> {
    prop_oneof![
        Just(LifecycleState::Created),
        Just(LifecycleState::Initializing),
        Just(LifecycleState::Ready),
        Just(LifecycleState::InUse),
        Just(LifecycleState::Idle),
        Just(LifecycleState::Maintenance),
        Just(LifecycleState::Draining),
        Just(LifecycleState::Cleanup),
        Just(LifecycleState::Terminated),
        Just(LifecycleState::Failed),
    ]
}

proptest! {
    /// can_transition_to(target) must agree with next_states() containing target,
    /// except for self-transitions which are always allowed by can_transition_to.
    #[test]
    fn can_transition_consistent_with_next_states(
        from in arb_lifecycle_state(),
        to in arb_lifecycle_state(),
    ) {
        let next = from.next_states();
        let in_next = next.contains(&to);
        let can = from.can_transition_to(to);

        if from == to {
            // Self-transitions: can_transition_to allows them
            // (Terminated self-transition is false by the explicit (Terminated, _) => false rule)
            if from == LifecycleState::Terminated {
                prop_assert!(!can, "Terminated should not allow even self-transition");
            } else {
                prop_assert!(can, "Self-transition should be allowed for {:?}", from);
            }
        } else {
            prop_assert_eq!(
                in_next, can,
                "Mismatch for {:?} -> {:?}: next_states contains={}, can_transition_to={}",
                from, to, in_next, can
            );
        }
    }

    /// Terminal states must have no valid outgoing transitions in next_states()
    /// EXCEPT Failed -> Cleanup/Terminated (Failed is recoverable).
    #[test]
    fn terminated_has_no_next_states(
        target in arb_lifecycle_state(),
    ) {
        let terminated = LifecycleState::Terminated;
        prop_assert!(
            terminated.next_states().is_empty(),
            "Terminated should have no next states"
        );
        prop_assert!(
            !terminated.can_transition_to(target),
            "Terminated should not transition to {:?}", target
        );
    }

    /// Failed is terminal (is_terminal() == true) but it has outgoing transitions
    /// to Cleanup and Terminated. Verify this is consistent.
    #[test]
    fn failed_state_recovery_paths(
        _dummy in 0u8..1u8, // proptest requires at least one input
    ) {
        let failed = LifecycleState::Failed;
        prop_assert!(failed.is_terminal(), "Failed should be terminal");
        prop_assert!(
            failed.can_transition_to(LifecycleState::Cleanup),
            "Failed should be able to transition to Cleanup"
        );
        prop_assert!(
            failed.can_transition_to(LifecycleState::Terminated),
            "Failed should be able to transition to Terminated"
        );
        // But not back to Ready
        prop_assert!(
            !failed.can_transition_to(LifecycleState::Ready),
            "Failed should not transition to Ready"
        );
    }

    /// Every state that can_acquire must also be is_available
    #[test]
    fn can_acquire_implies_available(state in arb_lifecycle_state()) {
        if state.can_acquire() {
            prop_assert!(
                state.is_available(),
                "{:?} can_acquire but !is_available", state
            );
        }
    }

    /// Transitional states are not available
    #[test]
    fn transitional_not_available(state in arb_lifecycle_state()) {
        if state.is_transitional() {
            prop_assert!(
                !state.is_available(),
                "{:?} is transitional but also available", state
            );
        }
    }
}

/// Exhaustive test: verify every (state, state) pair for consistency
#[test]
fn exhaustive_transition_consistency() {
    let all_states = [
        LifecycleState::Created,
        LifecycleState::Initializing,
        LifecycleState::Ready,
        LifecycleState::InUse,
        LifecycleState::Idle,
        LifecycleState::Maintenance,
        LifecycleState::Draining,
        LifecycleState::Cleanup,
        LifecycleState::Terminated,
        LifecycleState::Failed,
    ];

    for from in &all_states {
        let next = from.next_states();
        for to in &all_states {
            let can = from.can_transition_to(*to);
            let in_next = next.contains(to);

            if from == to {
                // Self-transitions
                if *from == LifecycleState::Terminated {
                    assert!(!can, "Terminated -> Terminated should be false");
                } else {
                    assert!(can, "{:?} -> {:?} self-transition should be true", from, to);
                }
            } else {
                assert_eq!(
                    in_next, can,
                    "Inconsistency: {:?} -> {:?}: next_states={}, can_transition_to={}",
                    from, to, in_next, can
                );
            }
        }
    }
}
