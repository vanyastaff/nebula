use nebula_validator::{
    DeferredReason, DiagnosticDisclosure, EvaluationOutcome, ExecutionMode, FieldDirective,
    FieldPolicyDecl, Predicate, PredicateContext, Presence, RequiredPolicy, Requiredness, Rule,
    ValidationErrorKind, VisibilityPolicy, foundation::FieldPath, resolve_field_policies,
};
use serde_json::json;

const DISCLOSURE: DiagnosticDisclosure = DiagnosticDisclosure::IncludeValue;

fn pending_context() -> PredicateContext {
    PredicateContext::from_json(json!({"settings": {"literal": true}, "other": null}))
        .with_pending_paths([FieldPath::from_segments(["settings", "computed"])])
}

#[test]
fn pending_roots_overlap_ancestors_and_descendants_only() {
    let ctx = pending_context();
    for pointer in [
        "",
        "/settings",
        "/settings/computed",
        "/settings/computed/child",
    ] {
        assert!(
            ctx.is_pending(&FieldPath::from_pointer(pointer).unwrap()),
            "{pointer:?}"
        );
    }
    for pointer in [
        "/setting",
        "/settings/literal",
        "/settings/computed_more",
        "/other",
    ] {
        assert!(
            !ctx.is_pending(&FieldPath::from_pointer(pointer).unwrap()),
            "{pointer:?}"
        );
    }
    let root = PredicateContext::new().with_pending_paths([FieldPath::root()]);
    assert!(root.is_pending(&FieldPath::from_segments(["any", "path"])));
}

#[test]
fn pending_missing_predicates_defer_instead_of_passing_or_rejecting() {
    let ctx = pending_context();
    for predicate in [
        Predicate::Ne(
            FieldPath::from_pointer("/settings/computed").unwrap(),
            json!(0),
        ),
        Predicate::Empty(FieldPath::from_pointer("/settings/computed").unwrap()),
        Predicate::Eq(
            FieldPath::from_pointer("/settings/computed").unwrap(),
            json!(null),
        ),
    ] {
        let rule = Rule::predicate(predicate).unwrap();
        assert_eq!(
            rule.validate(
                &json!(42),
                Some(&ctx),
                ExecutionMode::StaticOnly,
                DISCLOSURE,
            )
            .unwrap(),
            EvaluationOutcome::Deferred(vec![DeferredReason::PredicateContext])
        );
        assert_eq!(
            rule.validate(&json!(42), Some(&ctx), ExecutionMode::Full, DISCLOSURE)
                .unwrap_err()
                .kind(),
            ValidationErrorKind::Unavailable
        );
        assert_eq!(
            rule.matches(&ctx).unwrap_err().kind(),
            ValidationErrorKind::Unavailable
        );
    }
}

#[test]
fn pending_descendant_defers_container_and_root_predicates() {
    let ctx = pending_context();
    for path in [FieldPath::root(), FieldPath::single("settings")] {
        let rule = Rule::predicate(Predicate::Empty(path)).unwrap();
        assert_eq!(
            rule.validate(
                &json!(42),
                Some(&ctx),
                ExecutionMode::StaticOnly,
                DISCLOSURE,
            )
            .unwrap(),
            EvaluationOutcome::Deferred(vec![DeferredReason::PredicateContext])
        );
    }
}

#[test]
fn full_evaluation_rejects_pending_context() {
    let rule = Rule::predicate(Predicate::Empty(
        FieldPath::from_pointer("/settings/computed").unwrap(),
    ))
    .unwrap();
    let error = rule
        .validate(
            &json!(42),
            Some(&pending_context()),
            ExecutionMode::Full,
            DISCLOSURE,
        )
        .unwrap_err();
    assert_eq!(error.kind(), ValidationErrorKind::Unavailable);
}

#[test]
fn pending_required_condition_does_not_emit_required_early() {
    let path = FieldPath::single("name");
    let condition = Rule::predicate(Predicate::Ne(
        FieldPath::from_pointer("/settings/computed").unwrap(),
        json!(0),
    ))
    .unwrap();
    let decl = FieldPolicyDecl::new(
        &path,
        VisibilityPolicy::Always,
        RequiredPolicy::When(&condition),
        false,
        false,
        (),
    );
    let resolution = resolve_field_policies([decl], &pending_context()).unwrap();
    assert_eq!(resolution.required_failures.len(), 0);
    assert_eq!(resolution.plans[0].directive, FieldDirective::Deferred);
}

#[test]
fn pending_policy_dependencies_defer_without_required_failures() {
    let ctx = pending_context();
    let path = FieldPath::single("name");
    let condition = Rule::predicate(Predicate::Empty(FieldPath::single("settings"))).unwrap();
    for (value_present, raw_present) in [(false, false), (false, true), (true, true)] {
        for (visibility, required, presence, requiredness) in [
            (
                VisibilityPolicy::When(&condition),
                RequiredPolicy::Always,
                Presence::Pending,
                Requiredness::Required,
            ),
            (
                VisibilityPolicy::Always,
                RequiredPolicy::When(&condition),
                Presence::Active,
                Requiredness::Pending,
            ),
            (
                VisibilityPolicy::When(&condition),
                RequiredPolicy::When(&condition),
                Presence::Pending,
                Requiredness::Pending,
            ),
            (
                VisibilityPolicy::Never,
                RequiredPolicy::When(&condition),
                Presence::Skipped,
                Requiredness::Pending,
            ),
        ] {
            let decl = FieldPolicyDecl::new(
                &path,
                visibility,
                required,
                value_present,
                raw_present,
                "payload",
            );
            let result = resolve_field_policies([decl], &ctx).unwrap();
            assert!(result.required_failures.is_empty());
            assert_eq!(result.plans.len(), 1);
            assert_eq!(result.plans[0].directive, FieldDirective::Deferred);
            assert_eq!(result.plans[0].presence, presence);
            assert_eq!(result.plans[0].requiredness, requiredness);
            assert_eq!(result.plans[0].payload, "payload");
        }
    }
}

#[test]
fn pending_conditions_cannot_be_decided_by_any_or_not() {
    let ctx = pending_context();
    let pending = Rule::predicate(Predicate::Empty(FieldPath::single("settings"))).unwrap();
    for rule in [
        Rule::not(pending.clone()).unwrap(),
        Rule::any([Rule::all([]).unwrap(), pending.clone()]).unwrap(),
        Rule::all([Rule::any([]).unwrap(), pending]).unwrap(),
    ] {
        assert_eq!(
            rule.matches(&ctx).unwrap_err().kind(),
            ValidationErrorKind::Unavailable
        );
        assert_eq!(
            RequiredPolicy::When(&rule).resolve(&ctx).unwrap(),
            Requiredness::Pending
        );
    }
}

#[test]
fn invalid_conditions_are_not_hidden_by_pending_dependencies() {
    let ctx = pending_context();
    let pending = Rule::predicate(Predicate::Empty(FieldPath::single("settings"))).unwrap();
    for rule in [
        Rule::any([pending.clone(), Rule::email()]).unwrap(),
        Rule::not(Rule::all([pending, Rule::email()]).unwrap()).unwrap(),
    ] {
        assert_eq!(
            rule.matches(&ctx).unwrap_err().kind(),
            ValidationErrorKind::InvalidRule
        );
        assert_eq!(
            RequiredPolicy::When(&rule)
                .resolve(&ctx)
                .unwrap_err()
                .kind(),
            ValidationErrorKind::InvalidRule
        );
    }
}

#[test]
fn direct_predicates_reject_pending_placeholders() {
    let path = FieldPath::single("computed");
    for value in [json!(null), json!(""), json!(0), json!({"child": true})] {
        let ctx = PredicateContext::from_fields([(path.clone(), value)])
            .with_pending_paths([path.clone()]);
        let error = Predicate::Empty(path.clone()).evaluate(&ctx).unwrap_err();
        assert_eq!(error.kind(), ValidationErrorKind::Unavailable);
        assert_eq!(error.field_pointer().as_deref(), Some("/computed"));
    }
}

#[test]
fn settled_missing_null_and_sibling_semantics_are_unchanged() {
    let ctx = pending_context();
    for (predicate, expected) in [
        (Predicate::Eq(FieldPath::single("other"), json!(null)), true),
        (
            Predicate::Eq(FieldPath::single("absent"), json!(null)),
            false,
        ),
        (
            Predicate::Ne(FieldPath::single("absent"), json!(null)),
            true,
        ),
        (Predicate::Empty(FieldPath::single("absent")), true),
        (
            Predicate::Set(FieldPath::from_segments(["settings", "literal"])),
            true,
        ),
    ] {
        assert_eq!(predicate.evaluate(&ctx).unwrap(), expected);
    }
}

#[test]
fn partial_rule_logic_preserves_pending_without_skipping_known_constraints() {
    let ctx = pending_context();
    let pending = Rule::predicate(Predicate::Empty(FieldPath::single("settings"))).unwrap();
    for rule in [
        Rule::any([Rule::min_value(100), pending.clone()]).unwrap(),
        Rule::not(Rule::all([Rule::min_value(1), pending.clone()]).unwrap()).unwrap(),
    ] {
        assert_eq!(
            rule.validate(
                &json!(42),
                Some(&ctx),
                ExecutionMode::StaticOnly,
                DISCLOSURE,
            )
            .unwrap(),
            EvaluationOutcome::Deferred(vec![DeferredReason::PredicateContext])
        );
    }
    let rule = Rule::all([Rule::min_value(100), pending]).unwrap();
    assert_eq!(
        rule.validate(
            &json!(42),
            Some(&ctx),
            ExecutionMode::StaticOnly,
            DISCLOSURE,
        )
        .unwrap_err()
        .kind(),
        ValidationErrorKind::Violation
    );
}

#[test]
fn resolved_context_decides_previously_pending_requiredness() {
    let field = FieldPath::single("name");
    let condition = Rule::predicate(Predicate::Eq(FieldPath::single("flag"), json!(true))).unwrap();
    let ctx = PredicateContext::new().with_pending_paths([FieldPath::single("flag")]);
    assert_eq!(
        RequiredPolicy::When(&condition).resolve(&ctx).unwrap(),
        Requiredness::Pending
    );
    for (flag, expected_directive, failures) in [
        (true, FieldDirective::RequiredAbsent, 1),
        (false, FieldDirective::Validate, 0),
    ] {
        let resolved = PredicateContext::from_json(json!({"flag": flag}));
        let decl = FieldPolicyDecl::new(
            &field,
            VisibilityPolicy::Always,
            RequiredPolicy::When(&condition),
            false,
            false,
            (),
        );
        let resolution = resolve_field_policies([decl], &resolved).unwrap();
        assert_eq!(resolution.required_failures.len(), failures);
        assert_eq!(resolution.plans[0].directive, expected_directive);
    }
}

#[test]
fn pending_context_debug_exposes_only_counts() {
    let ctx = PredicateContext::from_json(json!({"value": "private-value"}))
        .with_pending_paths([FieldPath::single("private-path")]);
    let debug = format!("{ctx:?}");
    assert!(!debug.contains("private-value"), "{debug}");
    assert!(!debug.contains("private-path"), "{debug}");
    assert!(debug.contains("pending_count: 1"), "{debug}");
}

proptest::proptest! {
    #[test]
    fn pending_overlap_matches_segment_prefixes(
        path in proptest::collection::vec("[a-z0-9~/]{0,6}", 0..5),
        pending in proptest::collection::vec("[a-z0-9~/]{0,6}", 0..5),
    ) {
        let expected = path.starts_with(&pending) || pending.starts_with(&path);
        let ctx = PredicateContext::new().with_pending_paths([FieldPath::from_segments(&pending)]);
        proptest::prop_assert_eq!(ctx.is_pending(&FieldPath::from_segments(&path)), expected);
    }
}
