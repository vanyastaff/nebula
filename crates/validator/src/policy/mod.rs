//! Field visibility / required policy evaluation.
//!
//! Owns the *engine* for `When(Rule)` conditions. Callers get typed
//! `Presence`/`Requiredness` verdicts — never a raw `bool` they could
//! forget to branch on. `resolve_field_policies` is the single entry
//! point `nebula-schema`'s `validate` uses.

use crate::foundation::{FieldPath, ValidationError, ValidationErrors};
use crate::rule::{PredicateContext, Rule};

/// Whether a field participates in this validation round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Presence {
    /// Field is visible; its value rules must run.
    Active,
    /// Field is hidden; its value rules MUST be skipped.
    Skipped,
}

/// Resolved required-ness for a field in this round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Requiredness {
    /// Absence is an error.
    Required,
    /// Absence is allowed.
    Optional,
}

/// A field's visibility policy, borrowed from the schema's serde enum.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum VisibilityPolicy<'a> {
    /// Always visible.
    Always,
    /// Never visible.
    Never,
    /// Visible only when the borrowed rule matches the context.
    When(&'a Rule),
}

/// A field's required policy, borrowed from the schema's serde enum.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum RequiredPolicy<'a> {
    /// Never required.
    Optional,
    /// Always required.
    Always,
    /// Required only when the borrowed rule matches the context.
    When(&'a Rule),
}

impl VisibilityPolicy<'_> {
    /// The only way to turn a visibility policy into a decision.
    #[must_use]
    pub fn resolve(&self, ctx: &PredicateContext) -> Presence {
        match self {
            Self::Always => Presence::Active,
            Self::Never => Presence::Skipped,
            Self::When(r) => {
                if r.matches(ctx) {
                    Presence::Active
                } else {
                    Presence::Skipped
                }
            },
        }
    }
}

impl RequiredPolicy<'_> {
    /// The only way to turn a required policy into a decision.
    #[must_use]
    pub fn resolve(&self, ctx: &PredicateContext) -> Requiredness {
        match self {
            Self::Optional => Requiredness::Optional,
            Self::Always => Requiredness::Required,
            Self::When(r) => {
                if r.matches(ctx) {
                    Requiredness::Required
                } else {
                    Requiredness::Optional
                }
            },
        }
    }
}

/// The single action the caller must take for a field this round.
///
/// A ternary verdict computed once by [`resolve_field_policies`] so the caller
/// is a dumb dispatcher with no policy logic of its own:
///
/// - [`Skip`](FieldDirective::Skip): the field is hidden *and* carries no raw
///   value — no value rules run.
/// - [`RequiredAbsent`](FieldDirective::RequiredAbsent): the field is required
///   and absent. The `required` error has *already* been emitted into
///   [`FieldPolicyResolution::required_failures`]; the caller short-circuits
///   value rules (no double-report).
/// - [`Validate`](FieldDirective::Validate): run the field's structural / value
///   validation. **Reachable while hidden**: a hidden field that nonetheless
///   carries a present (non-absent) value must still be validated — e.g. an
///   expression smuggled into a no-payload mode-variant placeholder must not
///   escape unchecked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldDirective {
    /// Hidden and value-absent: skip value rules entirely.
    Skip,
    /// Required and absent: `required` already emitted; short-circuit value
    /// rules.
    RequiredAbsent,
    /// Run structural / value validation (reachable while hidden).
    Validate,
}

/// Per-field policy declaration the schema hands to the validator.
///
/// `P` is an opaque payload the caller mints alongside each decl; it is
/// threaded verbatim into the matching [`FieldPlan`] so the caller never has
/// to re-correlate a plan with the field it was computed for (see the
/// invariant on [`resolve_field_policies`]).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FieldPolicyDecl<'a, P> {
    /// RFC 6901 path of the field.
    pub path: &'a FieldPath,
    /// Visibility policy borrowed from the schema enum.
    pub visibility: VisibilityPolicy<'a>,
    /// Required policy borrowed from the schema enum.
    pub required: RequiredPolicy<'a>,
    /// Whether a non-absent value is present for this field (the caller's
    /// emptiness verdict — e.g. an empty string counts as *not* present).
    pub value_present: bool,
    /// Whether the field's key is syntactically present in the input at all,
    /// independent of emptiness. Distinguishes a hidden field that was never
    /// supplied (skip) from a hidden field carrying an explicit value.
    pub raw_present: bool,
    /// Opaque payload threaded 1:1 into the resulting [`FieldPlan`].
    pub payload: P,
}

impl<'a, P> FieldPolicyDecl<'a, P> {
    /// Construct a decl. Explicit ctor keeps the `#[non_exhaustive]` struct
    /// constructible across the crate boundary.
    #[must_use]
    pub fn new(
        path: &'a FieldPath,
        visibility: VisibilityPolicy<'a>,
        required: RequiredPolicy<'a>,
        value_present: bool,
        raw_present: bool,
        payload: P,
    ) -> Self {
        Self {
            path,
            visibility,
            required,
            value_present,
            raw_present,
            payload,
        }
    }
}

/// Per-field decision the schema MUST honor.
///
/// Carries the opaque `payload` minted by the caller for *this* field, so the
/// caller dispatches on `directive` without re-deriving which field the plan
/// belongs to.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FieldPlan<'a, P> {
    /// RFC 6901 path of the field.
    pub path: &'a FieldPath,
    /// Whether the field participates this round (diagnostic; the actionable
    /// verdict is `directive`).
    pub presence: Presence,
    /// Resolved required-ness (diagnostic; required failures are already
    /// emitted into `FieldPolicyResolution::required_failures`).
    pub requiredness: Requiredness,
    /// The single action the caller must take for this field.
    pub directive: FieldDirective,
    /// The caller's opaque payload, threaded verbatim from the matching decl.
    pub payload: P,
}

/// Output of [`resolve_field_policies`].
///
/// No `Default`/`Debug` derive: a caller-chosen `P` payload (e.g. a borrowed
/// entry reference) is not necessarily `Default`-constructible.
#[non_exhaustive]
pub struct FieldPolicyResolution<'a, P> {
    /// One plan per input decl, in input order.
    pub plans: Vec<FieldPlan<'a, P>>,
    /// `required` errors for required, absent fields — validator owns this
    /// reporting.
    pub required_failures: ValidationErrors,
}

/// Resolve visibility/required for a set of fields against one context.
///
/// This is the **sole** `required` emitter: a required-and-absent field
/// produces exactly one `required` error into `required_failures` when it is
/// visible *or* its key is syntactically present (the hidden-but-present
/// carve-out). A hidden field whose key was never supplied produces no
/// `required` failure — a hidden field cannot be required.
///
/// INVARIANT: exactly one [`FieldPlan`] is emitted per input decl. Cross-wiring
/// is now type-enforced — each plan carries the opaque `payload` minted for
/// *that* decl, so there is no parallel decls/plans collection a caller could
/// desync by reordering. The residual risk is **omission**: callers MUST NOT
/// filter, dedupe, or reorder-drop `plans`. A dropped plan is a field that is
/// silently never validated.
#[must_use]
pub fn resolve_field_policies<'a, P, I>(
    decls: I,
    ctx: &PredicateContext,
) -> FieldPolicyResolution<'a, P>
where
    I: IntoIterator<Item = FieldPolicyDecl<'a, P>>,
{
    let mut out = FieldPolicyResolution {
        plans: Vec::new(),
        required_failures: ValidationErrors::default(),
    };
    for d in decls {
        let presence = d.visibility.resolve(ctx);
        let requiredness = d.required.resolve(ctx);
        let active = presence == Presence::Active;
        let required_absent = requiredness == Requiredness::Required && !d.value_present;

        // Sole emitter: one `required` for a required-and-absent field that is
        // visible OR syntactically present (hidden-but-present carve-out).
        if required_absent && (active || d.raw_present) {
            out.required_failures.add(
                ValidationError::new("required", "field is required")
                    .with_field_path(d.path.clone()),
            );
        }

        let directive = if !active && !d.raw_present {
            FieldDirective::Skip
        } else if required_absent {
            FieldDirective::RequiredAbsent
        } else {
            FieldDirective::Validate
        };

        out.plans.push(FieldPlan {
            path: d.path,
            presence,
            requiredness,
            directive,
            payload: d.payload,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_variants_are_copy_and_eq() {
        let p = Presence::Active;
        let q = p; // Copy
        assert_eq!(p, q);
        assert_ne!(Presence::Active, Presence::Skipped);
    }

    #[test]
    fn requiredness_variants_are_copy_and_eq() {
        let r = Requiredness::Required;
        let s = r;
        assert_eq!(r, s);
        assert_ne!(Requiredness::Required, Requiredness::Optional);
    }

    #[test]
    fn visibility_policy_resolves_to_presence() {
        use crate::rule::{context::PredicateContext, predicate::Predicate};
        let ctx = PredicateContext::from_json(&serde_json::json!({"enabled": true}));
        assert_eq!(VisibilityPolicy::Always.resolve(&ctx), Presence::Active);
        assert_eq!(VisibilityPolicy::Never.resolve(&ctx), Presence::Skipped);
        let rule = Rule::Predicate(Predicate::IsTrue(FieldPath::parse("enabled").unwrap()));
        assert_eq!(
            VisibilityPolicy::When(&rule).resolve(&ctx),
            Presence::Active
        );
    }

    #[test]
    fn required_policy_resolves_to_requiredness() {
        use crate::rule::{context::PredicateContext, predicate::Predicate};
        let ctx = PredicateContext::from_json(&serde_json::json!({"mode": "oauth"}));
        assert_eq!(
            RequiredPolicy::Optional.resolve(&ctx),
            Requiredness::Optional
        );
        assert_eq!(RequiredPolicy::Always.resolve(&ctx), Requiredness::Required);
        let rule = Rule::Predicate(Predicate::Eq(
            FieldPath::parse("mode").unwrap(),
            serde_json::json!("oauth"),
        ));
        assert_eq!(
            RequiredPolicy::When(&rule).resolve(&ctx),
            Requiredness::Required
        );
    }

    #[test]
    fn resolve_field_policies_plans_and_required_failures() {
        use crate::rule::{context::PredicateContext, predicate::Predicate};
        let ctx = PredicateContext::from_json(&serde_json::json!({"mode": "oauth"}));

        let visible_path = FieldPath::parse("client_id").unwrap();
        let hidden_path = FieldPath::parse("legacy").unwrap();
        let req_rule = Rule::Predicate(Predicate::Eq(
            FieldPath::parse("mode").unwrap(),
            serde_json::json!("oauth"),
        ));

        let decls = vec![
            FieldPolicyDecl {
                path: &visible_path,
                visibility: VisibilityPolicy::Always,
                required: RequiredPolicy::When(&req_rule),
                value_present: false, // required (mode==oauth) but absent → failure
                raw_present: false,
                payload: (),
            },
            FieldPolicyDecl {
                path: &hidden_path,
                visibility: VisibilityPolicy::Never,
                required: RequiredPolicy::Always,
                value_present: false, // hidden, no raw → no required failure
                raw_present: false,
                payload: (),
            },
        ];

        let res = resolve_field_policies(decls, &ctx);

        assert_eq!(res.plans.len(), 2);
        let visible_plan = res.plans.iter().find(|p| p.path == &visible_path).unwrap();
        assert_eq!(visible_plan.presence, Presence::Active);
        assert_eq!(visible_plan.requiredness, Requiredness::Required);
        let hidden_plan = res.plans.iter().find(|p| p.path == &hidden_path).unwrap();
        assert_eq!(hidden_plan.presence, Presence::Skipped);

        // Exactly one required failure: the visible, required, absent field.
        // The hidden field is skipped → its `Always` required does not fire.
        let failures: Vec<_> = res.required_failures.errors().iter().collect();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].code, "required");
    }

    #[test]
    fn payload_is_threaded_one_to_one_into_plans() {
        use crate::rule::context::PredicateContext;
        let ctx = PredicateContext::from_json(&serde_json::json!({}));
        let p0 = FieldPath::parse("a").unwrap();
        let p1 = FieldPath::parse("b").unwrap();
        let decls = vec![
            FieldPolicyDecl::new(
                &p0,
                VisibilityPolicy::Always,
                RequiredPolicy::Optional,
                true,
                true,
                "A",
            ),
            FieldPolicyDecl::new(
                &p1,
                VisibilityPolicy::Never,
                RequiredPolicy::Optional,
                false,
                false,
                "B",
            ),
        ];
        let res = resolve_field_policies(decls, &ctx);
        assert_eq!(res.plans.len(), 2);
        assert_eq!(res.plans[0].payload, "A");
        assert_eq!(res.plans[1].payload, "B");
    }

    #[test]
    fn hidden_present_required_absent_emits_one_required_and_required_absent_directive() {
        use crate::rule::context::PredicateContext;
        // Hidden (Never) + Always-required + value absent + raw syntactically
        // present: the carve-out the schema gate used to own. The validator is
        // now the sole emitter — exactly one `required`, directive
        // `RequiredAbsent` (value rules short-circuited, but the field is NOT
        // skipped because its raw value is present).
        let ctx = PredicateContext::from_json(&serde_json::json!({}));
        let path = FieldPath::parse("secret_slot").unwrap();
        let decls = vec![FieldPolicyDecl::new(
            &path,
            VisibilityPolicy::Never,
            RequiredPolicy::Always,
            false, // value_present = false (empty / absent value)
            true,  // raw_present = true (the key syntactically exists)
            (),
        )];
        let res = resolve_field_policies(decls, &ctx);

        assert_eq!(res.plans.len(), 1);
        assert_eq!(res.plans[0].presence, Presence::Skipped);
        assert_eq!(res.plans[0].requiredness, Requiredness::Required);
        assert_eq!(res.plans[0].directive, FieldDirective::RequiredAbsent);

        let failures = res.required_failures.errors();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].code, "required");
    }

    #[test]
    fn hidden_no_raw_required_absent_is_skipped_with_no_required_failure() {
        use crate::rule::context::PredicateContext;
        // Hidden + required + no raw value at all: a hidden field cannot be
        // required → no `required` failure, directive `Skip` (no value rules).
        let ctx = PredicateContext::from_json(&serde_json::json!({}));
        let path = FieldPath::parse("legacy").unwrap();
        let decls = vec![FieldPolicyDecl::new(
            &path,
            VisibilityPolicy::Never,
            RequiredPolicy::Always,
            false, // value_present = false
            false, // raw_present = false (key not supplied at all)
            (),
        )];
        let res = resolve_field_policies(decls, &ctx);

        assert_eq!(res.plans.len(), 1);
        assert_eq!(res.plans[0].directive, FieldDirective::Skip);
        assert!(res.required_failures.errors().is_empty());
    }

    #[test]
    fn visible_required_absent_emits_one_required_and_required_absent_directive() {
        use crate::rule::context::PredicateContext;
        // Visible + Always-required + absent: classic required failure.
        // Exactly one `required`, directive `RequiredAbsent`.
        let ctx = PredicateContext::from_json(&serde_json::json!({}));
        let path = FieldPath::parse("client_id").unwrap();
        let decls = vec![FieldPolicyDecl::new(
            &path,
            VisibilityPolicy::Always,
            RequiredPolicy::Always,
            false, // value_present = false
            false, // raw_present irrelevant when active
            (),
        )];
        let res = resolve_field_policies(decls, &ctx);

        assert_eq!(res.plans.len(), 1);
        assert_eq!(res.plans[0].presence, Presence::Active);
        assert_eq!(res.plans[0].directive, FieldDirective::RequiredAbsent);

        let failures = res.required_failures.errors();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].code, "required");
    }

    #[test]
    fn hidden_present_non_absent_is_validated_not_skipped() {
        use crate::rule::context::PredicateContext;
        // Hidden + present non-absent value (value_present = true): the field
        // is NOT skipped — a hidden field carrying a real value must still be
        // structurally validated. Directive `Validate`, no `required` failure.
        let ctx = PredicateContext::from_json(&serde_json::json!({}));
        let path = FieldPath::parse("auth").unwrap();
        let decls = vec![FieldPolicyDecl::new(
            &path,
            VisibilityPolicy::Never,
            RequiredPolicy::Always,
            true, // value_present = true → not required-absent
            true, // raw_present = true
            (),
        )];
        let res = resolve_field_policies(decls, &ctx);

        assert_eq!(res.plans.len(), 1);
        assert_eq!(res.plans[0].presence, Presence::Skipped);
        assert_eq!(res.plans[0].directive, FieldDirective::Validate);
        assert!(res.required_failures.errors().is_empty());
    }
}
