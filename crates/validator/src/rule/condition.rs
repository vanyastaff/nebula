//! Checked conditions reuse the bounded rule arena and predicate evaluator.

use serde::{Deserialize, Deserializer, Serialize};

use super::{Predicate, PredicateContext, Rule, RuleChildren, RuleNode, RuleRef, RuleView};
use crate::foundation::{ValidationError, ValidationErrorKind};

/// The outcome of evaluating a condition against one predicate context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionOutcome {
    /// Resolved dependencies satisfy the condition.
    Match,
    /// Resolved dependencies do not satisfy the condition.
    NoMatch,
    /// The answer depends on an unresolved value; no boolean proof exists.
    ///
    /// Evaluation is three-valued (Kleene): a definite answer wins over
    /// pending. `all` is `NoMatch` when any child is `NoMatch`, `any` is
    /// `Match` when any child is `Match`, and `not` keeps `Pending`. Only
    /// when no child decides the combination does a pending child make the
    /// result `Pending`. Callers must retain the obligation, never read it
    /// as `NoMatch`.
    Pending,
}

/// A predicate-only rule, checked at construction and deserialization.
///
/// This certifies the condition grammar only: it is not schema admission and
/// not proof about the input. Schema owners must check every referenced path
/// and domain against their admitted input contract before attaching it to a
/// policy. [`Condition::evaluate`] accepts any [`PredicateContext`]; choosing
/// the context (for example one retained by prepared input) is the caller's
/// responsibility.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Condition(Rule);

impl TryFrom<Rule> for Condition {
    type Error = ValidationError;

    fn try_from(rule: Rule) -> Result<Self, Self::Error> {
        for node in &rule.nodes {
            match node {
                RuleNode::Predicate(Predicate::In(_, values)) if values.is_empty() => {
                    return Err(ValidationError::invalid_rule(
                        "condition membership requires at least one operand",
                    ));
                },
                RuleNode::Predicate(_) | RuleNode::Not(_) => {},
                RuleNode::All(children) | RuleNode::Any(children) if !children.is_empty() => {},
                RuleNode::All(_) | RuleNode::Any(_) => {
                    return Err(ValidationError::invalid_rule(
                        "condition combinations require at least one operand",
                    ));
                },
                RuleNode::Value(_) | RuleNode::Deferred(_) | RuleNode::Described { .. } => {
                    return Err(ValidationError::invalid_rule(
                        "conditions may contain only predicates and logical combinators",
                    ));
                },
            }
        }
        Ok(Self(rule))
    }
}

impl<'de> Deserialize<'de> for Condition {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::try_from(Rule::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl Condition {
    /// Borrow the checked rule without granting mutation of its arena.
    #[must_use]
    pub const fn as_rule(&self) -> &Rule {
        &self.0
    }

    /// Visit every predicate for owning-schema path and domain admission.
    pub fn predicates(&self) -> impl Iterator<Item = &Predicate> {
        self.0.nodes.iter().filter_map(|node| match node {
            RuleNode::Predicate(predicate) => Some(predicate),
            _ => None,
        })
    }

    /// Boolean view of [`Self::evaluate`] for callers that need a definite answer.
    ///
    /// # Errors
    /// A [`ConditionOutcome::Pending`] result is returned as an `Unavailable`
    /// error, never as `false`; configuration errors propagate unchanged.
    pub fn matches(&self, context: &PredicateContext) -> Result<bool, ValidationError> {
        match self.evaluate(context)? {
            ConditionOutcome::Match => Ok(true),
            ConditionOutcome::NoMatch => Ok(false),
            ConditionOutcome::Pending => Err(ValidationError::unavailable(
                "condition depends on an unresolved value",
            )),
        }
    }

    /// Evaluate with three-valued (Kleene) logic; see [`ConditionOutcome::Pending`].
    ///
    /// Every branch is visited, so a configuration error is never hidden by
    /// a definite sibling.
    ///
    /// # Errors
    /// Returns configuration errors without negating them into success.
    #[tracing::instrument(level = "trace", skip_all)]
    pub fn evaluate(
        &self,
        context: &PredicateContext,
    ) -> Result<ConditionOutcome, ValidationError> {
        evaluate_node(self.0.root(), context)
    }
}

fn evaluate_node(
    rule: RuleRef<'_>,
    context: &PredicateContext,
) -> Result<ConditionOutcome, ValidationError> {
    match rule.view() {
        RuleView::Predicate(predicate) => match predicate.evaluate(context) {
            Ok(true) => Ok(ConditionOutcome::Match),
            Ok(false) => Ok(ConditionOutcome::NoMatch),
            Err(error) if error.kind() == ValidationErrorKind::Unavailable => {
                Ok(ConditionOutcome::Pending)
            },
            Err(error) => Err(error),
        },
        RuleView::All(children) => combine(children, context, ConditionOutcome::NoMatch),
        RuleView::Any(children) => combine(children, context, ConditionOutcome::Match),
        RuleView::Not(inner) => Ok(match evaluate_node(inner, context)? {
            ConditionOutcome::Match => ConditionOutcome::NoMatch,
            ConditionOutcome::NoMatch => ConditionOutcome::Match,
            ConditionOutcome::Pending => ConditionOutcome::Pending,
        }),
        // Construction rejects these kinds; keep the refusal total anyway.
        RuleView::Value(_) | RuleView::Deferred(_) | RuleView::Described { .. } => {
            Err(ValidationError::invalid_rule(
                "conditions may contain only predicates and logical combinators",
            ))
        },
    }
}

/// `dominant` is the outcome that decides the combination on its own:
/// `NoMatch` for `all`, `Match` for `any`.
fn combine(
    children: RuleChildren<'_>,
    context: &PredicateContext,
    dominant: ConditionOutcome,
) -> Result<ConditionOutcome, ValidationError> {
    let mut decided = false;
    let mut pending = false;
    for child in children {
        match evaluate_node(child, context)? {
            outcome if outcome == dominant => decided = true,
            ConditionOutcome::Pending => pending = true,
            ConditionOutcome::Match | ConditionOutcome::NoMatch => {},
        }
    }
    Ok(if decided {
        dominant
    } else if pending {
        ConditionOutcome::Pending
    } else if dominant == ConditionOutcome::Match {
        ConditionOutcome::NoMatch
    } else {
        ConditionOutcome::Match
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MAX_RULE_DEPTH, foundation::FieldPath};
    use serde_json::json;

    fn predicate() -> Rule {
        Rule::predicate(Predicate::IsTrue(FieldPath::single("enabled"))).unwrap()
    }

    #[test]
    fn checks_every_branch_at_construction_and_deserialization() {
        let invalid = Rule::any([predicate(), Rule::min_length(1)]).unwrap();
        assert!(Condition::try_from(invalid.clone()).is_err());
        let wire = serde_json::to_value(invalid).unwrap();
        assert!(serde_json::from_value::<Condition>(wire).is_err());
        let decorated = Rule::described(predicate(), "private").unwrap();
        assert!(Condition::try_from(decorated).is_err());
        let deferred = Rule::not(Rule::custom("private()").unwrap()).unwrap();
        assert!(Condition::try_from(deferred.clone()).is_err());
        assert!(
            serde_json::from_value::<Condition>(serde_json::to_value(deferred).unwrap()).is_err()
        );
    }

    #[test]
    fn rejects_empty_combinations_and_membership() {
        for rule in [
            Rule::all([]).unwrap(),
            Rule::any([]).unwrap(),
            Rule::predicate(Predicate::In(FieldPath::single("mode"), vec![])).unwrap(),
        ] {
            assert!(Condition::try_from(rule).is_err());
        }
    }

    #[test]
    fn evaluates_snapshot_and_preserves_missing_and_pending_semantics() {
        let condition = Condition::try_from(predicate()).unwrap();
        let context = PredicateContext::from_json(json!({"enabled": true}));
        assert_eq!(
            condition.evaluate(&context).unwrap(),
            ConditionOutcome::Match
        );
        assert_eq!(
            condition
                .evaluate(&PredicateContext::from_json(json!({})))
                .unwrap(),
            ConditionOutcome::NoMatch
        );
        let not = Condition::try_from(Rule::not(predicate()).unwrap()).unwrap();
        let pending = context.with_pending_paths([FieldPath::single("enabled")]);
        assert_eq!(not.evaluate(&pending).unwrap(), ConditionOutcome::Pending);
        assert_eq!(
            not.matches(&pending).unwrap_err().kind(),
            ValidationErrorKind::Unavailable
        );
    }

    #[test]
    fn deserialize_rejects_empty_combinations_membership_and_decoration() {
        for wire in [json!({"all": []}), json!({"any": []})] {
            assert!(serde_json::from_value::<Rule>(wire.clone()).is_ok());
            assert!(serde_json::from_value::<Condition>(wire).is_err());
        }
        for rule in [
            Rule::predicate(Predicate::In(FieldPath::single("mode"), vec![])).unwrap(),
            Rule::described(predicate(), "private").unwrap(),
        ] {
            let wire = serde_json::to_value(rule).unwrap();
            assert!(serde_json::from_value::<Rule>(wire.clone()).is_ok());
            assert!(serde_json::from_value::<Condition>(wire).is_err());
        }
    }

    fn flag(name: &str) -> Rule {
        Rule::predicate(Predicate::IsTrue(FieldPath::single(name))).unwrap()
    }

    /// `t` is true, `f` is false, `p` is pending.
    fn kleene_context() -> PredicateContext {
        PredicateContext::from_json(json!({"t": true, "f": false}))
            .with_pending_paths([FieldPath::single("p")])
    }

    fn outcome(rule: Rule) -> ConditionOutcome {
        Condition::try_from(rule)
            .unwrap()
            .evaluate(&kleene_context())
            .unwrap()
    }

    fn all<const N: usize>(rules: [Rule; N]) -> Rule {
        Rule::all(rules).unwrap()
    }

    fn any<const N: usize>(rules: [Rule; N]) -> Rule {
        Rule::any(rules).unwrap()
    }

    fn not(rule: Rule) -> Rule {
        Rule::not(rule).unwrap()
    }

    #[test]
    fn definite_answer_wins_over_pending() {
        use ConditionOutcome::{Match, NoMatch, Pending};
        assert_eq!(outcome(all([flag("f"), flag("p")])), NoMatch);
        assert_eq!(outcome(all([flag("p"), flag("f")])), NoMatch);
        assert_eq!(outcome(any([flag("t"), flag("p")])), Match);
        assert_eq!(outcome(any([flag("p"), flag("t")])), Match);
        assert_eq!(outcome(all([flag("t"), flag("p")])), Pending);
        assert_eq!(outcome(any([flag("f"), flag("p")])), Pending);
        assert_eq!(outcome(not(flag("p"))), Pending);
    }

    #[test]
    fn nested_combinations_follow_kleene_logic() {
        use ConditionOutcome::{Match, NoMatch, Pending};
        // any(all(f, p), t) -> any(NoMatch, Match) -> Match
        assert_eq!(
            outcome(any([all([flag("f"), flag("p")]), flag("t")])),
            Match
        );
        // all(any(t, p), not(any(f, p))) -> all(Match, Pending) -> Pending
        assert_eq!(
            outcome(all([
                any([flag("t"), flag("p")]),
                not(any([flag("f"), flag("p")])),
            ])),
            Pending
        );
        // not(all(any(p, f), f)) -> not(NoMatch) -> Match
        assert_eq!(
            outcome(not(all([any([flag("p"), flag("f")]), flag("f")]))),
            Match
        );
        // all(not(any(p, t)), p) -> all(NoMatch, Pending) -> NoMatch
        assert_eq!(
            outcome(all([not(any([flag("p"), flag("t")])), flag("p")])),
            NoMatch
        );
    }

    #[test]
    fn resolved_combinations_evaluate_as_booleans() {
        use ConditionOutcome::{Match, NoMatch};
        assert_eq!(outcome(all([flag("t"), flag("f")])), NoMatch);
        assert_eq!(outcome(all([flag("t"), flag("t")])), Match);
        assert_eq!(outcome(any([flag("f"), flag("f")])), NoMatch);
        assert_eq!(outcome(any([flag("f"), flag("t")])), Match);
        assert_eq!(outcome(not(flag("t"))), NoMatch);
        assert_eq!(outcome(not(flag("f"))), Match);
    }

    #[test]
    fn matches_reports_pending_as_unavailable_not_false() {
        let context = kleene_context();
        let decided = Condition::try_from(all([flag("f"), flag("p")])).unwrap();
        assert!(!decided.matches(&context).unwrap());
        let undecided = Condition::try_from(all([flag("t"), flag("p")])).unwrap();
        assert_eq!(
            undecided.matches(&context).unwrap_err().kind(),
            ValidationErrorKind::Unavailable
        );
        assert_eq!(undecided.predicates().count(), 2);
    }

    #[test]
    fn roundtrip_keeps_rule_wire_and_existing_depth_budget() {
        let condition = Condition::try_from(predicate()).unwrap();
        let wire = serde_json::to_value(&condition).unwrap();
        assert_eq!(wire, json!({"is_true": "/enabled"}));
        assert_eq!(
            serde_json::from_value::<Condition>(wire).unwrap(),
            condition
        );
        let mut rule = predicate();
        for _ in 1..MAX_RULE_DEPTH {
            rule = Rule::not(rule).unwrap();
        }
        assert!(Condition::try_from(rule.clone()).is_ok());
        assert!(Rule::not(rule).is_err());
    }

    #[test]
    fn ordering_retains_exact_large_integer_comparisons() {
        let condition = Condition::try_from(
            Rule::predicate(Predicate::Gt(
                FieldPath::single("counter"),
                serde_json::Number::from(9_007_199_254_740_992_u64),
            ))
            .unwrap(),
        )
        .unwrap();
        let context = PredicateContext::from_json(json!({
            "counter": 9_007_199_254_740_993_u64
        }));
        assert_eq!(
            condition.evaluate(&context).unwrap(),
            ConditionOutcome::Match
        );
    }
}
