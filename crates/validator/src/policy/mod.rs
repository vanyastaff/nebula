//! Field visibility / required policy evaluation (ADR-0052).
//!
//! Owns the *engine* for `When(Rule)` conditions. Callers get typed
//! `Presence`/`Requiredness` verdicts — never a raw `bool` they could
//! forget to branch on.
//!
//! Imports are added by later tasks as each type is first consumed
//! (Task 3 adds `crate::rule::{PredicateContext, Rule}`; Task 4 adds
//! `crate::foundation::{FieldPath, ValidationError, ValidationErrors}`).

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
        let rule = Rule::Predicate(Predicate::IsTrue(
            crate::foundation::FieldPath::parse("enabled").unwrap(),
        ));
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
            crate::foundation::FieldPath::parse("mode").unwrap(),
            serde_json::json!("oauth"),
        ));
        assert_eq!(
            RequiredPolicy::When(&rule).resolve(&ctx),
            Requiredness::Required
        );
    }
}
