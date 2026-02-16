//! T045: Scope containment transitivity property tests.
//!
//! Uses proptest to verify structural properties of `Scope::contains()`:
//! - Transitivity: if A contains B and B contains C, then A contains C
//! - Global contains everything
//! - Same-scope is always compatible (reflexivity)

use nebula_resource::Scope;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategy: generate arbitrary Scope values with parent chains
// ---------------------------------------------------------------------------

fn arb_tenant_id() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("tenant-A".to_string()),
        Just("tenant-B".to_string()),
        Just("tenant-C".to_string()),
    ]
}

fn arb_workflow_id() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("wf-1".to_string()),
        Just("wf-2".to_string()),
        Just("wf-3".to_string()),
    ]
}

fn arb_execution_id() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("ex-1".to_string()),
        Just("ex-2".to_string()),
        Just("ex-3".to_string()),
    ]
}

fn arb_action_id() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("act-1".to_string()),
        Just("act-2".to_string()),
        Just("act-3".to_string()),
    ]
}

/// Generate scopes with full parent chain information so that
/// containment checks can actually succeed through the hierarchy.
fn arb_scope() -> impl Strategy<Value = Scope> {
    prop_oneof![
        // Global
        Just(Scope::Global),
        // Tenant
        arb_tenant_id().prop_map(|t| Scope::tenant(t)),
        // Workflow with tenant parent
        (arb_workflow_id(), arb_tenant_id()).prop_map(|(w, t)| Scope::workflow_in_tenant(w, t)),
        // Workflow without tenant parent
        arb_workflow_id().prop_map(|w| Scope::workflow(w)),
        // Execution with full chain
        (arb_execution_id(), arb_workflow_id(), arb_tenant_id())
            .prop_map(|(e, w, t)| { Scope::execution_in_workflow(e, w, Some(t)) }),
        // Execution without tenant
        (arb_execution_id(), arb_workflow_id())
            .prop_map(|(e, w)| Scope::execution_in_workflow(e, w, None)),
        // Action with full chain
        (
            arb_action_id(),
            arb_execution_id(),
            arb_workflow_id(),
            arb_tenant_id(),
        )
            .prop_map(|(a, e, w, t)| { Scope::action_in_execution(a, e, Some(w), Some(t)) }),
        // Action without parents
        arb_action_id().prop_map(|a| Scope::action(a)),
    ]
}

// ---------------------------------------------------------------------------
// Property: transitivity
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn transitivity_holds(
        a in arb_scope(),
        b in arb_scope(),
        c in arb_scope(),
    ) {
        if a.contains(&b) && b.contains(&c) {
            prop_assert!(
                a.contains(&c),
                "Transitivity violated: {:?} contains {:?} and {:?} contains {:?}, \
                 but {:?} does NOT contain {:?}",
                a, b, b, c, a, c,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property: Global contains everything
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn global_contains_everything(scope in arb_scope()) {
        prop_assert!(
            Scope::Global.contains(&scope),
            "Global should contain {:?}",
            scope,
        );
    }
}

// ---------------------------------------------------------------------------
// Property: reflexivity (same scope is always compatible)
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn same_scope_contains_itself(scope in arb_scope()) {
        prop_assert!(
            scope.contains(&scope),
            "{:?} should contain itself",
            scope,
        );
    }
}

// ---------------------------------------------------------------------------
// Property: nothing contains Global except Global itself
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn only_global_contains_global(scope in arb_scope()) {
        if scope.contains(&Scope::Global) {
            prop_assert_eq!(
                scope.clone(),
                Scope::Global,
                "Only Global should contain Global, but {:?} also does",
                scope,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property: broader scope never contained by narrower scope
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn broader_never_contained_by_narrower(
        a in arb_scope(),
        b in arb_scope(),
    ) {
        // If a is strictly broader than b (lower hierarchy level),
        // then b should NOT contain a (unless they are the same scope, which
        // is excluded because is_broader_than uses strict <).
        if a.is_broader_than(&b) {
            prop_assert!(
                !b.contains(&a),
                "Narrower scope {:?} should not contain broader scope {:?}",
                b, a,
            );
        }
    }
}
