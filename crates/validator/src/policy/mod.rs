//! Field visibility / required policy evaluation (ADR-0052).
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

/// Per-field policy declaration the schema hands to the validator.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FieldPolicyDecl<'a> {
    /// RFC 6901 path of the field.
    pub path: &'a FieldPath,
    /// Visibility policy borrowed from the schema enum.
    pub visibility: VisibilityPolicy<'a>,
    /// Required policy borrowed from the schema enum.
    pub required: RequiredPolicy<'a>,
    /// Whether a non-absent raw value is present for this field.
    pub value_present: bool,
}

impl<'a> FieldPolicyDecl<'a> {
    /// Construct a decl. Explicit ctor keeps the `#[non_exhaustive]` struct
    /// constructible across the crate boundary.
    #[must_use]
    pub fn new(
        path: &'a FieldPath,
        visibility: VisibilityPolicy<'a>,
        required: RequiredPolicy<'a>,
        value_present: bool,
    ) -> Self {
        Self {
            path,
            visibility,
            required,
            value_present,
        }
    }
}

/// Per-field decision the schema MUST honor.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FieldPlan<'a> {
    /// RFC 6901 path of the field.
    pub path: &'a FieldPath,
    /// Whether the field participates this round.
    pub presence: Presence,
    /// Resolved required-ness (informational; required failures are already
    /// emitted into `FieldPolicyResolution::required_failures`).
    pub requiredness: Requiredness,
}

/// Output of [`resolve_field_policies`].
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct FieldPolicyResolution<'a> {
    /// One plan per input decl, in input order.
    pub plans: Vec<FieldPlan<'a>>,
    /// `required` errors for visible, required, absent fields — validator
    /// owns this reporting (ADR-0052).
    pub required_failures: ValidationErrors,
}

/// Resolve visibility/required for a set of fields against one context.
///
/// A `Presence::Skipped` field never produces a required failure even if its
/// `RequiredPolicy` is `Always` — a hidden field cannot be required.
///
/// INVARIANT: exactly one `FieldPlan` is emitted per input decl, in input
/// order — never filtered, reordered, or deduped. Callers rely on positional
/// `plans[i]` ↔ `decls[i]` correspondence; breaking it silently misvalidates.
#[must_use]
pub fn resolve_field_policies<'a, I>(decls: I, ctx: &PredicateContext) -> FieldPolicyResolution<'a>
where
    I: IntoIterator<Item = FieldPolicyDecl<'a>>,
{
    let mut out = FieldPolicyResolution::default();
    for d in decls {
        let presence = d.visibility.resolve(ctx);
        let requiredness = d.required.resolve(ctx);
        if presence == Presence::Active
            && requiredness == Requiredness::Required
            && !d.value_present
        {
            out.required_failures.add(
                ValidationError::new("required", "field is required")
                    .with_field_path(d.path.clone()),
            );
        }
        out.plans.push(FieldPlan {
            path: d.path,
            presence,
            requiredness,
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
            },
            FieldPolicyDecl {
                path: &hidden_path,
                visibility: VisibilityPolicy::Never,
                required: RequiredPolicy::Always,
                value_present: false, // hidden → no required failure
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
}
