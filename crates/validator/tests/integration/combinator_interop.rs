//! Scenario: mixing the programmatic combinator API (`.and`/`.or`/`.not`)
//! with the derive macro and the `Rule` engine. This guards the
//! architectural promise that the three surfaces compose cleanly.

use nebula_validator::{
    Rule,
    foundation::{Validate, ValidateExt, ValidationError},
    validators::{alphanumeric, email, max_length, min_length, not_empty},
};

#[cfg(feature = "derive")]
use super::common::expect_errors;
#[cfg(feature = "derive")]
use nebula_validator::Validator;

#[test]
fn programmatic_composition_matches_rule_equivalent() {
    // Two validators that should behave the same: one programmatic, one declarative.
    let programmatic = min_length(3).and(max_length(32)).and(alphanumeric());

    let declarative = Rule::all([
        Rule::min_length(3),
        Rule::max_length(32),
        Rule::pattern(r"^[A-Za-z0-9]+$").unwrap(),
    ])
    .unwrap();

    for input in [
        "alice42",
        "!",
        "",
        "A",
        "this-is-way-too-long-for-32-chars-at-last-count",
    ] {
        let prog_ok = programmatic.validate(input).is_ok();
        let decl_ok = <Rule as Validate<serde_json::Value>>::validate(
            &declarative,
            &serde_json::json!(input),
        )
        .is_ok();
        assert_eq!(
            prog_ok, decl_ok,
            "surfaces disagree on `{input:?}`: prog={prog_ok}, decl={decl_ok}"
        );
    }
}

#[test]
fn or_combinator_short_circuits_on_success() {
    // `ab` fails min_length(5), but email() is the fallback — which also fails.
    let validator = min_length(5).or(email());
    let err: ValidationError = validator.validate("ab").unwrap_err();
    assert_eq!(err.code.as_ref(), "or_failed");
    // Both alternatives' errors should be captured as nested for diagnostics.
    assert_eq!(err.nested().len(), 2);
}

#[test]
fn not_combinator_inverts_success() {
    let validator = not_empty().not();
    assert!(validator.validate("").is_ok());
    assert!(validator.validate("nonempty").is_err());
}

/// Free function built from combinator chaining; we hand the *function
/// pointer* to the derive via `custom = ...` so method-resolution (which
/// requires `ValidateExt` in scope at the derive expansion site) is
/// irrelevant to the generated code.
#[cfg(feature = "derive")]
fn bounded_name(input: &str) -> Result<(), ValidationError> {
    min_length(3).and(max_length(10)).validate(input)
}

#[derive(Validator)]
#[cfg(feature = "derive")]
struct UsingCustomCombinator {
    #[validate(custom = bounded_name)]
    name: String,
}

#[test]
#[cfg(feature = "derive")]
fn derive_custom_accepts_combinator_function() {
    let ok = UsingCustomCombinator {
        name: "hello".into(),
    };
    assert!(ok.validate_fields().is_ok());

    let too_long = UsingCustomCombinator {
        name: "waaaaaaaaaaaay-too-long".into(),
    };
    assert!(!expect_errors(too_long.validate_fields()).is_empty());

    let too_short = UsingCustomCombinator { name: "ab".into() };
    assert!(!expect_errors(too_short.validate_fields()).is_empty());
}

#[test]
fn not_over_predicate_propagates_skip_when_no_ctx() {
    use nebula_validator::Predicate;
    use serde_json::json;

    let rule =
        Rule::not(Rule::predicate(Predicate::eq("status", json!("active")).unwrap()).unwrap())
            .unwrap();

    let result = rule
        .validate(
            &json!(null),
            None,
            nebula_validator::ExecutionMode::StaticOnly,
            nebula_validator::DiagnosticDisclosure::IncludeValue,
        )
        .unwrap();
    assert_eq!(
        result,
        nebula_validator::EvaluationOutcome::Deferred(vec![
            nebula_validator::DeferredReason::PredicateContext
        ])
    );
    assert_eq!(
        Validate::validate(&rule, &json!(null)).unwrap_err().kind(),
        nebula_validator::ValidationErrorKind::Unavailable
    );
}

#[test]
fn not_over_deferred_propagates_skip_in_static_only() {
    use serde_json::json;

    let rule = Rule::not(Rule::custom("dynamic_check()").unwrap()).unwrap();
    let result = rule
        .validate(
            &json!("any"),
            None,
            nebula_validator::ExecutionMode::StaticOnly,
            nebula_validator::DiagnosticDisclosure::IncludeValue,
        )
        .unwrap();
    assert_eq!(
        result,
        nebula_validator::EvaluationOutcome::Deferred(vec![
            nebula_validator::DeferredReason::DeferredRule
        ])
    );
    assert_eq!(
        Validate::validate(&rule, &json!("any")).unwrap_err().kind(),
        nebula_validator::ValidationErrorKind::Unavailable
    );
}
