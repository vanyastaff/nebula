use nebula_validator::{
    DeferredReason, DiagnosticDisclosure, EvaluationOutcome, ExecutionMode, Predicate,
    PredicateContext, Rule, RulePattern, ValidationErrorKind, ValueRule,
    foundation::{FieldPath, Validate, ValidateExt},
    validate_rules,
};
use serde_json::{Value, json};

const DISCLOSURE: DiagnosticDisclosure = DiagnosticDisclosure::IncludeValue;

#[test]
fn incomplete_rule_cannot_issue_a_complete_proof() {
    let error = Rule::custom("check()")
        .unwrap()
        .validate_into(json!(42))
        .unwrap_err();
    let nebula_validator::ValidatorError::ValidationFailed(error) = error else {
        panic!("expected a validation diagnostic");
    };
    assert_eq!(error.code, "evaluation_unavailable");
}

#[test]
fn full_evaluation_requires_predicate_context() {
    let rule = Rule::predicate(Predicate::eq("enabled", true).unwrap()).unwrap();
    let errors = validate_rules(&json!(42), &[rule], ExecutionMode::Full, DISCLOSURE).unwrap_err();
    assert_eq!(errors.errors()[0].code, "evaluation_unavailable");
}

#[test]
fn unavailable_evaluation_cannot_be_negated() {
    let rule = Rule::not(Rule::custom("check()").unwrap()).unwrap();
    let error = rule
        .validate(&json!(42), None, ExecutionMode::Full, DISCLOSURE)
        .unwrap_err();
    assert_eq!(error.code, "evaluation_unavailable");
}

#[test]
fn malformed_patterns_are_rejected_during_rule_deserialization() {
    for wire in [
        json!({"pattern": "["}),
        json!({"matches": ["/name", "["]}),
        json!({"not": {"pattern": "["}}),
        json!({"any": ["email", {"pattern": "["}]}),
    ] {
        let error = serde_json::from_value::<Rule>(wire).unwrap_err();
        assert_eq!(error.to_string(), "invalid rule pattern");
    }
}

#[test]
fn empty_alternatives_reject_every_value() {
    for value in [
        Value::Null,
        json!(false),
        json!(0),
        json!(""),
        json!([]),
        json!({}),
    ] {
        let error = Rule::any([])
            .unwrap()
            .validate(&value, None, ExecutionMode::Full, DISCLOSURE)
            .unwrap_err();
        assert_eq!(error.code, "any_failed");
        let error = Rule::one_of(Vec::<Value>::new())
            .unwrap()
            .validate(&value, None, ExecutionMode::Full, DISCLOSURE)
            .unwrap_err();
        assert_eq!(error.code, "one_of");
    }
}

#[test]
fn integer_above_exact_float_maximum_is_rejected() {
    let bound = serde_json::Number::from_f64(9_007_199_254_740_992.0).unwrap();
    let error = ValueRule::Max(bound)
        .validate_value(&json!(9_007_199_254_740_993_u64), DISCLOSURE)
        .unwrap_err();
    assert_eq!(error.code, "max");
}

#[test]
fn float_below_exact_integer_minimum_is_rejected() {
    let error = ValueRule::Min(9_007_199_254_740_993_u64.into())
        .validate_value(&json!(9_007_199_254_740_992.0), DISCLOSURE)
        .unwrap_err();
    assert_eq!(error.code, "min");
}

#[test]
fn negative_integer_below_exact_float_minimum_is_rejected() {
    let bound = serde_json::Number::from_f64(-9_007_199_254_740_992.0).unwrap();
    let error = ValueRule::Min(bound)
        .validate_value(&json!(-9_007_199_254_740_993_i64), DISCLOSURE)
        .unwrap_err();
    assert_eq!(error.code, "min");
}

#[test]
fn partial_outcomes_propagate_through_nested_logic() {
    let deferred = Rule::custom("check()").unwrap();
    for rule in [
        deferred.clone(),
        Rule::any([Rule::min_value(100), deferred.clone()]).unwrap(),
        Rule::not(Rule::all([Rule::min_value(1), deferred.clone()]).unwrap()).unwrap(),
        Rule::not(Rule::not(deferred).unwrap())
            .unwrap()
            .with_message("pending")
            .unwrap(),
    ] {
        assert_eq!(
            rule.validate(&json!(42), None, ExecutionMode::StaticOnly, DISCLOSURE)
                .unwrap(),
            EvaluationOutcome::Deferred(vec![DeferredReason::DeferredRule])
        );
    }
}

#[test]
fn resolved_predicates_distinguish_missing_null_and_values() {
    let rule = Rule::predicate(Predicate::eq("value", Value::Null).unwrap()).unwrap();
    assert_eq!(
        rule.validate(&json!(42), None, ExecutionMode::StaticOnly, DISCLOSURE)
            .unwrap(),
        EvaluationOutcome::Deferred(vec![DeferredReason::PredicateContext])
    );
    let ctx = PredicateContext::from_json(json!({"value": null}));
    assert_eq!(
        rule.validate(&json!(42), Some(&ctx), ExecutionMode::Full, DISCLOSURE)
            .unwrap(),
        EvaluationOutcome::Satisfied
    );
    for ctx in [
        PredicateContext::new(),
        PredicateContext::from_json(json!({"value": 0})),
    ] {
        let error = rule
            .validate(&json!(42), Some(&ctx), ExecutionMode::Full, DISCLOSURE)
            .unwrap_err();
        assert_eq!(error.code, "eq_failed");
        assert_eq!(error.kind(), ValidationErrorKind::Violation);
    }
}

#[test]
fn predicates_resolve_the_rfc6901_root() {
    let root = FieldPath::root();
    let populated = PredicateContext::from_json(json!(["value"]));
    for predicate in [
        Predicate::Eq(root.clone(), json!(["value"])),
        Predicate::Ne(root.clone(), json!(["other"])),
        Predicate::Set(root.clone()),
    ] {
        assert_eq!(
            predicate.evaluate(&populated),
            Ok(true),
            "{predicate:?} must evaluate against the complete JSON root"
        );
    }
    assert_eq!(
        Predicate::Empty(root.clone()).evaluate(&populated),
        Ok(false)
    );

    let empty = PredicateContext::from_json(json!([]));
    assert_eq!(
        Predicate::Eq(root.clone(), json!([])).evaluate(&empty),
        Ok(true)
    );
    assert_eq!(
        Predicate::Ne(root.clone(), json!(["value"])).evaluate(&empty),
        Ok(true)
    );
    assert_eq!(Predicate::Set(root.clone()).evaluate(&empty), Ok(false));
    assert_eq!(Predicate::Empty(root).evaluate(&empty), Ok(true));
}

#[test]
fn predicates_resolve_rfc6901_array_indices() {
    let context = PredicateContext::from_json(json!({"array": ["value", ""]}));
    let populated = FieldPath::from_pointer("/array/0").unwrap();
    let empty = FieldPath::from_pointer("/array/1").unwrap();

    for predicate in [
        Predicate::Eq(populated.clone(), json!("value")),
        Predicate::Ne(populated.clone(), json!("other")),
        Predicate::Set(populated.clone()),
    ] {
        assert_eq!(
            predicate.evaluate(&context),
            Ok(true),
            "{predicate:?} must resolve array index zero"
        );
    }
    assert_eq!(Predicate::Empty(populated).evaluate(&context), Ok(false));
    assert_eq!(Predicate::Set(empty.clone()).evaluate(&context), Ok(false));
    assert_eq!(Predicate::Empty(empty).evaluate(&context), Ok(true));
}

#[test]
fn rfc6901_lookup_drives_visibility_and_requiredness() {
    use nebula_validator::{
        FieldDirective, FieldPolicyDecl, Presence, RequiredPolicy, Requiredness, VisibilityPolicy,
        resolve_field_policies,
    };

    let context = PredicateContext::from_json(json!({"array": [true]}));
    let visible_when = Rule::predicate(Predicate::Eq(
        FieldPath::from_pointer("/array/0").unwrap(),
        json!(true),
    ))
    .unwrap();
    let required_when = Rule::predicate(Predicate::Set(FieldPath::root())).unwrap();
    let field_path = FieldPath::single("dependent");
    let declaration = FieldPolicyDecl::new(
        &field_path,
        VisibilityPolicy::When(&visible_when),
        RequiredPolicy::When(&required_when),
        false,
        false,
        (),
    );

    let resolution = resolve_field_policies([declaration], &context).unwrap();
    assert_eq!(resolution.plans.len(), 1);
    assert_eq!(resolution.plans[0].presence, Presence::Active);
    assert_eq!(resolution.plans[0].requiredness, Requiredness::Required);
    assert_eq!(
        resolution.plans[0].directive,
        FieldDirective::RequiredAbsent
    );
    assert_eq!(resolution.required_failures.len(), 1);
    assert_eq!(
        resolution.required_failures.errors()[0]
            .field_pointer()
            .as_deref(),
        Some("/dependent")
    );
}

#[test]
fn any_requires_a_satisfied_branch_and_not_preserves_unavailable_errors() {
    let rule = Rule::any([Rule::custom("check()").unwrap(), Rule::min_value(1)]).unwrap();
    assert_eq!(
        rule.validate(&json!(42), None, ExecutionMode::StaticOnly, DISCLOSURE)
            .unwrap(),
        EvaluationOutcome::Satisfied
    );
    let rule = Rule::not(Rule::not(Rule::custom("check()").unwrap()).unwrap()).unwrap();
    let error = rule
        .validate(&json!(42), None, ExecutionMode::Full, DISCLOSURE)
        .unwrap_err();
    assert_eq!(error.kind(), ValidationErrorKind::Unavailable);
}

#[test]
fn deferred_mode_recurses_into_logic_and_reports_static_obligations() {
    let rule = Rule::all([
        Rule::min_value(100),
        Rule::not(Rule::custom("check()").unwrap()).unwrap(),
    ])
    .unwrap();
    let error =
        validate_rules(&json!(42), &[rule], ExecutionMode::Deferred, DISCLOSURE).unwrap_err();
    assert_eq!(error.errors()[0].kind(), ValidationErrorKind::Unavailable);
    let rule = Rule::not(Rule::all([Rule::min_value(100)]).unwrap()).unwrap();
    assert_eq!(
        rule.validate(&json!(42), None, ExecutionMode::Deferred, DISCLOSURE)
            .unwrap(),
        EvaluationOutcome::Deferred(vec![DeferredReason::StaticRule])
    );
}

#[test]
fn protected_diagnostics_never_retain_input_across_nested_rules() {
    const SECRET: &str = "PRIVATE_RULE_SENTINEL";

    let rule = Rule::all([
        Rule::min_length(100)
            .with_message("submitted {value}")
            .unwrap(),
        Rule::one_of([json!("allowed")]).unwrap(),
    ])
    .unwrap();
    let errors = validate_rules(
        &json!(SECRET),
        &[rule],
        ExecutionMode::Full,
        DiagnosticDisclosure::OmitValue,
    )
    .unwrap_err();

    let mut pending = errors.errors().iter().collect::<Vec<_>>();
    while let Some(error) = pending.pop() {
        assert_eq!(error.param("value"), None);
        assert!(!format!("{error:?} {error}").contains(SECRET));
        pending.extend(error.nested());
    }
}

#[test]
fn condition_misuse_cannot_be_hidden_in_any_or_not() {
    let ctx = PredicateContext::new();
    for rule in [
        Rule::email(),
        Rule::not(Rule::email()).unwrap(),
        Rule::any([Rule::all([]).unwrap(), Rule::email()]).unwrap(),
    ] {
        let error = rule.matches(&ctx).unwrap_err();
        assert_eq!(error.kind(), ValidationErrorKind::InvalidRule);
    }
}

#[test]
fn checked_pattern_construction_and_serde_agree() {
    for pattern in ["[", "(?P<)"] {
        assert_eq!(
            Rule::pattern(pattern).unwrap_err().to_string(),
            RulePattern::new(pattern).unwrap_err().to_string()
        );
        assert!(serde_json::from_value::<ValueRule>(json!({"pattern": pattern})).is_err());
        assert!(
            serde_json::from_value::<Predicate>(json!({"matches": ["/name", pattern]})).is_err()
        );
    }
    let rule = Rule::predicate(Predicate::Matches(
        FieldPath::single("name"),
        RulePattern::new("^a+$").unwrap(),
    ))
    .unwrap();
    let wire = serde_json::to_value(&rule).unwrap();
    assert_eq!(wire, json!({"matches": ["/name", "^a+$"]}));
    assert_eq!(serde_json::from_value::<Rule>(wire).unwrap(), rule);
}

fn assert_order(left: Value, right: Value, expected: std::cmp::Ordering) {
    let bound = right.as_number().unwrap().clone();
    assert_eq!(
        ValueRule::LessThan(bound.clone())
            .validate_value(&left, DISCLOSURE)
            .is_ok(),
        expected.is_lt(),
        "{left} < {right}"
    );
    assert_eq!(
        ValueRule::GreaterThan(bound.clone())
            .validate_value(&left, DISCLOSURE)
            .is_ok(),
        expected.is_gt(),
        "{left} > {right}"
    );
    assert_eq!(
        ValueRule::Min(bound.clone())
            .validate_value(&left, DISCLOSURE)
            .is_ok(),
        expected.is_ge(),
        "{left} >= {right}"
    );
    assert_eq!(
        ValueRule::Max(bound.clone())
            .validate_value(&left, DISCLOSURE)
            .is_ok(),
        expected.is_le(),
        "{left} <= {right}"
    );
    let path = FieldPath::single("value");
    let ctx = PredicateContext::from_json(json!({"value": left}));
    for (predicate, matches) in [
        (Predicate::Lt(path.clone(), bound.clone()), expected.is_lt()),
        (Predicate::Gt(path.clone(), bound.clone()), expected.is_gt()),
        (
            Predicate::Gte(path.clone(), bound.clone()),
            expected.is_ge(),
        ),
        (Predicate::Lte(path, bound), expected.is_le()),
    ] {
        assert_eq!(
            predicate.evaluate(&ctx).unwrap(),
            matches,
            "{predicate:?}: {left}"
        );
    }
}

#[test]
fn numeric_boundaries_have_exact_order_in_both_directions() {
    use std::cmp::Ordering::{Equal, Greater, Less};
    for (left, right, expected) in [
        (
            json!(9_007_199_254_740_993_u64),
            json!(9_007_199_254_740_992.0),
            Greater,
        ),
        (
            json!(-9_007_199_254_740_993_i64),
            json!(-9_007_199_254_740_992.0),
            Less,
        ),
        (json!(u64::MAX), json!(18_446_744_073_709_551_616.0), Less),
        (json!(i64::MAX), json!(9_223_372_036_854_775_808.0), Less),
        (json!(i64::MIN), json!(-9_223_372_036_854_775_808.0), Equal),
        (json!(-1_i64), json!(u64::MAX), Less),
        (json!(0), json!(-0.0), Equal),
        (json!(0), json!(f64::MIN_POSITIVE), Less),
        (json!(0), json!(-f64::MIN_POSITIVE), Greater),
        (json!(u64::MAX), json!(f64::MAX), Less),
        (json!(0), json!(f64::from_bits(1)), Less),
    ] {
        assert_order(left.clone(), right.clone(), expected);
        assert_order(right, left, expected.reverse());
    }
}

proptest::proptest! {
    #[test]
    fn integer_float_order_matches_exact_quarter_units(integer: u64, quarters: i32) {
        let float = f64::from(quarters) / 4.0;
        let expected = (i128::from(integer) * 4).cmp(&i128::from(quarters));
        assert_order(json!(integer), json!(float), expected);
        assert_order(json!(float), json!(integer), expected.reverse());
    }
}

#[test]
fn programmatic_not_preserves_unavailable_rule_errors() {
    let validator = ValidateExt::not(Rule::custom("check()").unwrap());
    let error = validator.validate(&json!(42)).unwrap_err();
    assert_eq!(error.kind(), ValidationErrorKind::Unavailable);
}

#[test]
fn programmatic_or_preserves_unavailable_rule_errors() {
    let validator = Rule::custom("check()").unwrap().or(Rule::min_value(1));
    let error = validator.validate(&json!(42)).unwrap_err();
    assert_eq!(error.kind(), ValidationErrorKind::Unavailable);
}

#[test]
fn nested_programmatic_error_preserves_evaluation_classification() {
    let validator = ValidateExt::not(Rule::custom("check()").unwrap().and(Rule::min_value(100)));
    let error = validator.validate(&json!(42)).unwrap_err();
    assert_eq!(error.kind(), ValidationErrorKind::Unavailable);
}

#[test]
fn serialized_diagnostics_preserve_evaluation_classification() {
    let error = Validate::validate(&Rule::custom("check()").unwrap(), &json!(42)).unwrap_err();
    assert_eq!(error.to_json_value()["kind"], json!("unavailable"));
}

#[test]
fn unit_rule_deserialization_rejects_ignored_configuration() {
    for wire in [json!({"email": {"min_length": 100}}), json!({"url": false})] {
        assert!(serde_json::from_value::<ValueRule>(wire.clone()).is_err());
        assert!(
            serde_json::from_value::<Rule>(wire.clone()).is_err(),
            "ignored configuration: {wire}"
        );
    }
}

#[test]
fn programmatic_empty_alternatives_reject_every_value() {
    let validator = nebula_validator::combinators::any_of(Vec::<Rule>::new());
    let error = validator.validate(&json!(42)).unwrap_err();
    assert_eq!(error.code, "any_of_failed");
}

#[test]
fn invalid_policy_conditions_return_field_diagnostics() {
    use nebula_validator::{
        FieldPolicyDecl, RequiredPolicy, VisibilityPolicy, resolve_field_policies,
    };
    let path = FieldPath::single("name");
    let condition = Rule::not(Rule::email()).unwrap();
    let decl = FieldPolicyDecl::new(
        &path,
        VisibilityPolicy::Always,
        RequiredPolicy::When(&condition),
        false,
        false,
        (),
    );
    let result = resolve_field_policies([decl], &PredicateContext::new());
    let Err(errors) = result else {
        panic!("invalid condition produced a field plan");
    };
    assert_eq!(errors.errors()[0].kind(), ValidationErrorKind::InvalidRule);
    assert_eq!(errors.errors()[0].field_pointer().as_deref(), Some("/name"));
}
