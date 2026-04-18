//! Scenario: arbitrarily deep nesting of validation errors and rules.
//!
//! Neither the error tree nor the rule tree should hit stack-overflow
//! territory at realistic depths, and the recursive traversals
//! (`flatten`, `total_error_count`, `validate` through `All`/`Any`)
//! must visit every level.

use nebula_validator::{ExecutionMode, Rule, foundation::ValidationError, validate_rules};
use serde_json::json;

/// Constructs a left-nested chain of `All(MinLength, All(MinLength, All(…)))`
/// of the requested depth. Each level adds one more `MinLength` check.
fn nested_all_rule(depth: usize) -> Rule {
    let mut rule = Rule::min_length(1);
    for _ in 0..depth {
        rule = Rule::all([Rule::min_length(1), rule]);
    }
    rule
}

/// Recursively nests `n` levels of `ValidationError` to produce an error
/// tree of known depth.
fn nested_error(depth: usize) -> ValidationError {
    let mut err = ValidationError::new("leaf", "leaf");
    for i in 0..depth {
        err = ValidationError::new("branch", format!("lvl {i}")).with_nested_error(err);
    }
    err
}

#[test]
fn rule_engine_validates_moderately_deep_trees() {
    let rule = nested_all_rule(32);
    assert!(
        validate_rules(
            &json!("x"),
            std::slice::from_ref(&rule),
            ExecutionMode::StaticOnly
        )
        .is_ok()
    );
}

#[test]
fn rule_engine_surfaces_failures_from_deep_trees() {
    // Build an All-chain of depth 20 where the deepest leaf requires
    // length ≥ 100. Outer levels only require ≥ 1, so the failure at the
    // innermost level must still bubble out as an error.
    let mut rule = Rule::min_length(100);
    for _ in 0..20 {
        rule = Rule::all([Rule::min_length(1), rule]);
    }

    let result = validate_rules(
        &json!("short"),
        std::slice::from_ref(&rule),
        ExecutionMode::StaticOnly,
    );
    assert!(result.is_err(), "deep rule tree failed to propagate error");
}

#[test]
fn total_error_count_walks_every_level() {
    let err = nested_error(50);
    // 50 branches + 1 leaf = 51 total.
    assert_eq!(err.total_error_count(), 51);
}

#[test]
fn flatten_returns_entries_in_depth_first_order() {
    let err = nested_error(5);
    let flat = err.flatten();
    // 5 branches + 1 leaf.
    assert_eq!(flat.len(), 6);
    // Deepest error (the leaf) is last in depth-first order after the
    // outermost branch is visited first.
    assert_eq!(flat.first().unwrap().code.as_ref(), "branch");
    assert_eq!(flat.last().unwrap().code.as_ref(), "leaf");
}

#[test]
fn any_combinator_passes_when_deep_branch_succeeds() {
    // Failing outer alternatives followed by a deeply-nested success
    // branch — the combinator must walk to the inner branch and return Ok.
    let inner_passing = Rule::max_length(100);
    let chain = Rule::any([
        Rule::min_length(1000),
        Rule::any([Rule::min_length(999), Rule::any([inner_passing])]),
    ]);
    assert!(
        validate_rules(
            &json!("hello"),
            std::slice::from_ref(&chain),
            ExecutionMode::StaticOnly
        )
        .is_ok()
    );
}
