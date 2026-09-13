//! Ergonomic constructors for [`Rule`] and leaf rule enums.

use super::{
    DeferredRule, MAX_RULE_OPERANDS, Predicate, Rule, RuleBuildError, RuleNode, RulePattern,
    ValueRule, drop_json_value_iteratively,
    limits::{RuleBudgetState, RuleStats},
};
use crate::foundation::FieldPath;

/// An owned set of JSON operands ready for bounded rule admission.
///
/// The closed owned container lets [`Rule::one_of`] reject early while still
/// destroying every uninspected operand without recursive JSON drops.
pub struct RuleOperands(Vec<serde_json::Value>);

impl std::fmt::Debug for RuleOperands {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuleOperands")
            .field("len", &self.0.len())
            .finish_non_exhaustive()
    }
}

impl Drop for RuleOperands {
    fn drop(&mut self) {
        for value in std::mem::take(&mut self.0) {
            drop_json_value_iteratively(value);
        }
    }
}

impl From<Vec<serde_json::Value>> for RuleOperands {
    fn from(values: Vec<serde_json::Value>) -> Self {
        Self(values)
    }
}

impl<V: Into<serde_json::Value>, const N: usize> From<[V; N]> for RuleOperands {
    fn from(values: [V; N]) -> Self {
        Self(values.into_iter().map(Into::into).collect())
    }
}

impl ValueRule {
    /// Creates a [`ValueRule::MinLength`].
    #[must_use]
    pub fn min_length(n: usize) -> Self {
        Self::MinLength(n)
    }

    /// Creates a [`ValueRule::MaxLength`].
    #[must_use]
    pub fn max_length(n: usize) -> Self {
        Self::MaxLength(n)
    }

    /// Creates a checked [`ValueRule::Pattern`].
    ///
    /// # Errors
    /// Returns a redacted error for a malformed or oversized pattern.
    pub fn pattern(pattern: &str) -> Result<Self, RuleBuildError> {
        RulePattern::new(pattern).map(Self::Pattern)
    }
}

impl Predicate {
    /// Creates a [`Predicate::Eq`]. Returns `None` if the path is invalid.
    #[must_use]
    pub fn eq(field: impl AsRef<str>, value: impl Into<serde_json::Value>) -> Option<Self> {
        Some(Self::Eq(FieldPath::parse(field)?, value.into()))
    }
}

impl Rule {
    /// Creates a rule from a value-validation leaf.
    ///
    /// # Errors
    /// Returns the exhausted operand or text budget.
    pub fn value(value: ValueRule) -> Result<Self, RuleBuildError> {
        Self::from_leaf(RuleNode::Value(value))
    }

    /// Creates a rule from a context predicate.
    ///
    /// # Errors
    /// Returns the exhausted operand or text budget.
    pub fn predicate(predicate: Predicate) -> Result<Self, RuleBuildError> {
        Self::from_leaf(RuleNode::Predicate(predicate))
    }

    /// Creates a minimum string-length rule.
    #[must_use]
    pub fn min_length(n: usize) -> Self {
        trusted_value(ValueRule::MinLength(n))
    }

    /// Creates a maximum string-length rule.
    #[must_use]
    pub fn max_length(n: usize) -> Self {
        trusted_value(ValueRule::MaxLength(n))
    }

    /// Creates a checked regular-expression rule.
    ///
    /// # Errors
    /// Returns a redacted error for a malformed or oversized pattern.
    pub fn pattern(pattern: &str) -> Result<Self, RuleBuildError> {
        ValueRule::pattern(pattern).and_then(Self::value)
    }

    /// Creates a minimum numeric-value rule from an `i64`.
    #[must_use]
    pub fn min_value(n: i64) -> Self {
        trusted_value(ValueRule::Min(serde_json::Number::from(n)))
    }

    /// Creates a maximum numeric-value rule from an `i64`.
    #[must_use]
    pub fn max_value(n: i64) -> Self {
        trusted_value(ValueRule::Max(serde_json::Number::from(n)))
    }

    /// Creates a minimum numeric-value rule from an exact JSON number.
    #[must_use]
    pub fn min_number(n: serde_json::Number) -> Self {
        trusted_value(ValueRule::Min(n))
    }

    /// Creates a maximum numeric-value rule from an exact JSON number.
    #[must_use]
    pub fn max_number(n: serde_json::Number) -> Self {
        trusted_value(ValueRule::Max(n))
    }

    /// Creates a minimum numeric-value rule from an `f64`.
    /// Returns `None` if the value is NaN or infinite.
    #[must_use]
    pub fn min_value_f64(n: f64) -> Option<Self> {
        Some(trusted_value(ValueRule::Min(serde_json::Number::from_f64(
            n,
        )?)))
    }

    /// Creates a maximum numeric-value rule from an `f64`.
    /// Returns `None` if the value is NaN or infinite.
    #[must_use]
    pub fn max_value_f64(n: f64) -> Option<Self> {
        Some(trusted_value(ValueRule::Max(serde_json::Number::from_f64(
            n,
        )?)))
    }

    /// Creates a strict minimum numeric-value rule from an `i64`.
    #[must_use]
    pub fn greater_than(n: i64) -> Self {
        trusted_value(ValueRule::GreaterThan(serde_json::Number::from(n)))
    }

    /// Creates a strict maximum numeric-value rule from an `i64`.
    #[must_use]
    pub fn less_than(n: i64) -> Self {
        trusted_value(ValueRule::LessThan(serde_json::Number::from(n)))
    }

    /// Creates a value-set rule.
    ///
    /// # Errors
    /// Returns the exhausted operand or text budget.
    pub fn one_of(values: impl Into<RuleOperands>) -> Result<Self, RuleBuildError> {
        let values = collect_json_operands(values.into())?;
        Self::value(ValueRule::OneOf(values))
    }

    /// Creates a minimum collection-length rule.
    #[must_use]
    pub fn min_items(n: usize) -> Self {
        trusted_value(ValueRule::MinItems(n))
    }

    /// Creates a maximum collection-length rule.
    #[must_use]
    pub fn max_items(n: usize) -> Self {
        trusted_value(ValueRule::MaxItems(n))
    }

    /// Creates an email-address rule.
    #[must_use]
    pub fn email() -> Self {
        trusted_value(ValueRule::Email)
    }

    /// Creates a URL rule.
    #[must_use]
    pub fn url() -> Self {
        trusted_value(ValueRule::Url)
    }

    /// Creates a deferred custom-expression rule.
    ///
    /// # Errors
    /// Returns the exhausted text budget.
    pub fn custom(expression: impl Into<String>) -> Result<Self, RuleBuildError> {
        Self::from_leaf(RuleNode::Deferred(DeferredRule::Custom(expression.into())))
    }

    /// Creates a deferred uniqueness rule.
    ///
    /// # Errors
    /// Returns an error if the path is invalid or exhausts the text budget.
    pub fn unique_by(path: impl AsRef<str>) -> Result<Self, RuleBuildError> {
        let path = FieldPath::parse(path).ok_or(RuleBuildError::InvalidFieldPath)?;
        Self::from_leaf(RuleNode::Deferred(DeferredRule::UniqueBy(path)))
    }

    /// Creates a conjunction.
    ///
    /// # Errors
    /// Returns the exhausted depth, node, operand, or text budget.
    pub fn all(rules: impl IntoIterator<Item = Rule>) -> Result<Self, RuleBuildError> {
        compose_many(rules, RuleNode::All)
    }

    /// Creates a disjunction.
    ///
    /// # Errors
    /// Returns the exhausted depth, node, operand, or text budget.
    pub fn any(rules: impl IntoIterator<Item = Rule>) -> Result<Self, RuleBuildError> {
        compose_many(rules, RuleNode::Any)
    }

    /// Creates a negation.
    ///
    /// # Errors
    /// Returns the exhausted depth, node, operand, or text budget.
    #[expect(
        clippy::should_implement_trait,
        reason = "this is a rule constructor, not boolean negation"
    )]
    pub fn not(inner: Rule) -> Result<Self, RuleBuildError> {
        let stats = RuleStats::compose(std::slice::from_ref(&inner), 1, 0)?;
        let (mut nodes, root) = inner.into_shifted_nodes(0);
        nodes.push(RuleNode::Not(root));
        let root = nodes.len() - 1;
        Ok(Self::from_bounded_parts(nodes, root, stats))
    }

    /// Wraps a rule with a custom error message.
    ///
    /// # Errors
    /// Returns the exhausted depth, node, operand, or text budget.
    pub fn described(rule: Rule, message: impl Into<String>) -> Result<Self, RuleBuildError> {
        let message = message.into();
        let stats = RuleStats::compose(std::slice::from_ref(&rule), 1, message.len())?;
        let (mut nodes, inner) = rule.into_shifted_nodes(0);
        nodes.push(RuleNode::Described { inner, message });
        let root = nodes.len() - 1;
        Ok(Self::from_bounded_parts(nodes, root, stats))
    }

    /// Consumes this rule and wraps it in a custom error message.
    ///
    /// # Errors
    /// Returns the exhausted depth, node, operand, or text budget.
    pub fn with_message(self, message: impl Into<String>) -> Result<Self, RuleBuildError> {
        Self::described(self, message)
    }
}

fn trusted_value(value: ValueRule) -> Rule {
    Rule::from_bounded_parts(
        vec![RuleNode::Value(value)],
        0,
        RuleStats {
            depth: 1,
            nodes: 1,
            operands: 0,
            json_nodes: 0,
            json_depth: 0,
            text_bytes: 0,
        },
    )
}

fn compose_many(
    rules: impl IntoIterator<Item = Rule>,
    composite: impl FnOnce(Vec<usize>) -> RuleNode,
) -> Result<Rule, RuleBuildError> {
    let rules = collect_operands(rules)?;
    let stats = RuleStats::compose(&rules, rules.len(), 0)?;
    let mut nodes = Vec::with_capacity(stats.nodes);
    let mut children = Vec::with_capacity(rules.len());
    for rule in rules {
        let (mut child_nodes, child_root) = rule.into_shifted_nodes(nodes.len());
        nodes.append(&mut child_nodes);
        children.push(child_root);
    }
    nodes.push(composite(children));
    let root = nodes.len() - 1;
    Ok(Rule::from_bounded_parts(nodes, root, stats))
}

fn collect_operands<T>(values: impl IntoIterator<Item = T>) -> Result<Vec<T>, RuleBuildError> {
    let mut collected = Vec::new();
    for value in values {
        if collected.len() == MAX_RULE_OPERANDS {
            return Err(RuleBuildError::OperandLimit {
                limit: MAX_RULE_OPERANDS,
            });
        }
        collected.push(value);
    }
    Ok(collected)
}

fn collect_json_operands(values: RuleOperands) -> Result<Vec<serde_json::Value>, RuleBuildError> {
    let mut budget = RuleBudgetState::default();
    let mut collected = Vec::new();
    let mut values = values;
    let mut remaining = std::mem::take(&mut values.0).into_iter();
    while let Some(value) = remaining.next() {
        let admitted = budget
            .add_operands(1)
            .and_then(|()| budget.add_json_value(&value));
        if let Err(error) = admitted {
            drop_json_value_iteratively(value);
            for admitted_value in collected {
                drop_json_value_iteratively(admitted_value);
            }
            for uninspected_value in remaining {
                drop_json_value_iteratively(uninspected_value);
            }
            return Err(error);
        }
        collected.push(value);
    }
    Ok(collected)
}
