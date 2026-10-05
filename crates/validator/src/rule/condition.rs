//! Checked conditions reuse the bounded rule arena and predicate evaluator.

use serde::{Deserialize, Deserializer, Serialize};

use super::{Predicate, PredicateContext, Rule, RuleNode};
use crate::foundation::{ValidationError, ValidationErrorKind};

/// The outcome of evaluating a condition against one prepared snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionOutcome {
    /// Resolved dependencies satisfy the condition.
    Match,
    /// Resolved dependencies do not satisfy the condition.
    NoMatch,
    /// A dependency is unresolved; no boolean proof exists.
    Pending,
}

/// A predicate-only rule, checked at construction and deserialization.
///
/// This certifies the condition grammar, not schema admission or input proof.
/// Schema owners must check every referenced path and domain against their
/// admitted input contract before attaching it to a policy. Runtime owners
/// evaluate against the snapshot retained by their prepared input witness.
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

    /// Evaluate against the snapshot borrowed from prepared input.
    ///
    /// # Errors
    /// Pending dependencies remain `Unavailable`, including under `any` and
    /// `not`. Callers must retain the obligation rather than interpret it as false.
    #[tracing::instrument(level = "trace", skip_all)]
    pub fn matches(&self, context: &PredicateContext) -> Result<bool, ValidationError> {
        self.0.matches(context)
    }

    /// Classify a checked predicate result without losing pending obligations.
    ///
    /// # Errors
    /// Returns configuration errors without negating them into success.
    pub fn evaluate(
        &self,
        context: &PredicateContext,
    ) -> Result<ConditionOutcome, ValidationError> {
        match self.matches(context) {
            Ok(true) => Ok(ConditionOutcome::Match),
            Ok(false) => Ok(ConditionOutcome::NoMatch),
            Err(error) if error.kind() == ValidationErrorKind::Unavailable => {
                Ok(ConditionOutcome::Pending)
            },
            Err(error) => Err(error),
        }
    }
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
    fn resolved_alternative_cannot_hide_pending_dependency() {
        let condition = Condition::try_from(
            Rule::any([
                predicate(),
                Rule::predicate(Predicate::IsTrue(FieldPath::single("other"))).unwrap(),
            ])
            .unwrap(),
        )
        .unwrap();
        let context = PredicateContext::from_json(json!({"enabled": true}))
            .with_pending_paths([FieldPath::single("other")]);
        assert_eq!(
            condition.evaluate(&context).unwrap(),
            ConditionOutcome::Pending
        );
        assert_eq!(condition.predicates().count(), 2);
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
