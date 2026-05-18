//! (P2) lockdown #1: the ONLY schema→validator behavioral crossing
//! symbols are `validate_rules_with_ctx` and `resolve_field_policies`.
//!
//! This is a symbol-level invariant, not a single-runtime-call one (nested
//! schemas recurse). It pins that the legacy in-schema rule executor and the
//! validator→schema error mapping are gone: rule-failure codes now flow
//! through `nebula-validator` verbatim.

#[test]
fn schema_crosses_into_validator_through_one_surface_only() {
    let src = include_str!("../src/validated.rs");
    for forbidden in [
        "fn run_rules",
        "fn run_root_rules",
        "fn translate_validator_code",
        "fn push_validator_rule_errors",
        "validator_bridge",
        "PredicateContext::from_json",
    ] {
        assert!(
            !src.contains(forbidden),
            "validated.rs must not contain `{forbidden}` (single \
             validator crossing, no in-schema rule executor / code remap)"
        );
    }
    assert!(
        src.contains("validate_rules_with_ctx"),
        "the single value/root rule crossing must be `validate_rules_with_ctx`"
    );
    assert!(
        src.contains("resolve_field_policies"),
        "the single visibility/required crossing must be `resolve_field_policies`"
    );
}
