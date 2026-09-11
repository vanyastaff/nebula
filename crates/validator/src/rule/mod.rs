//! Unified declarative rules with a bounded, flat representation.
//!
//! [`Rule`] is opaque: every instance is admitted against fixed depth, node,
//! operand, and text budgets before it can exist. Logical children are stored
//! as arena indices, so destroying a rule never recursively drops user-shaped
//! data.

pub mod context;
pub mod deferred;
mod deserialize;
mod limits;
mod logic;
mod pattern;
pub mod predicate;
pub mod value;

mod constructors;
mod helpers;

#[cfg(test)]
mod tests;

pub use constructors::RuleOperands;
pub use context::PredicateContext;
pub use deferred::DeferredRule;
pub use limits::{
    MAX_RULE_DEPTH, MAX_RULE_JSON_DEPTH, MAX_RULE_JSON_NODES, MAX_RULE_NODES, MAX_RULE_OPERANDS,
    MAX_RULE_TEXT_BYTES, RuleBudget, RuleBuildError,
};
pub use pattern::RulePattern;
pub use predicate::Predicate;
use serde::{Serialize, ser::SerializeMap};
pub use value::ValueRule;

use self::limits::RuleStats;
use crate::{
    engine::{DeferredReason, DiagnosticDisclosure, EvaluationOutcome, ExecutionMode},
    foundation::{ValidationError, ValidationErrorKind},
};

type NodeId = usize;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum RuleNode {
    Value(ValueRule),
    Predicate(Predicate),
    All(Vec<NodeId>),
    Any(Vec<NodeId>),
    Not(NodeId),
    Deferred(DeferredRule),
    Described { inner: NodeId, message: String },
}

impl RuleNode {
    fn shift_child_ids(&mut self, offset: usize) {
        match self {
            Self::All(children) | Self::Any(children) => {
                for child in children {
                    *child += offset;
                }
            },
            Self::Not(child) => *child += offset,
            Self::Described { inner, .. } => *inner += offset,
            Self::Value(_) | Self::Predicate(_) | Self::Deferred(_) => {},
        }
    }
}

/// A declarative validation rule admitted against fixed complexity budgets.
///
/// The private indexed arena makes recursive ownership impossible. Use the
/// fallible composition constructors such as [`Rule::all`], [`Rule::not`], and
/// [`Rule::described`] to build nested rules.
#[derive(Clone, PartialEq)]
pub struct Rule {
    nodes: Vec<RuleNode>,
    root: NodeId,
    stats: RuleStats,
}

/// A borrowed reference to one node in a [`Rule`].
#[derive(Clone, Copy)]
pub struct RuleRef<'a> {
    rule: &'a Rule,
    node: NodeId,
}

impl<'a> RuleRef<'a> {
    /// Returns a read-only view of this node.
    #[must_use]
    pub fn view(self) -> RuleView<'a> {
        self.rule.view_at(self.node)
    }
}

/// A read-only view of one node in an opaque [`Rule`].
#[non_exhaustive]
pub enum RuleView<'a> {
    /// A value-validation rule.
    Value(&'a ValueRule),
    /// A context predicate.
    Predicate(&'a Predicate),
    /// A conjunction and its direct children.
    All(RuleChildren<'a>),
    /// A disjunction and its direct children.
    Any(RuleChildren<'a>),
    /// A negation and its child.
    Not(RuleRef<'a>),
    /// A deferred runtime rule.
    Deferred(&'a DeferredRule),
    /// A rule decorated with a custom diagnostic message.
    Described {
        /// The decorated rule.
        inner: RuleRef<'a>,
        /// The custom diagnostic message.
        message: &'a str,
    },
}

/// Iterator over the direct children of an `all` or `any` rule node.
#[derive(Clone)]
pub struct RuleChildren<'a> {
    rule: &'a Rule,
    children: std::slice::Iter<'a, NodeId>,
}

impl<'a> Iterator for RuleChildren<'a> {
    type Item = RuleRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.children.next().map(|node| RuleRef {
            rule: self.rule,
            node: *node,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.children.size_hint()
    }
}

impl ExactSizeIterator for RuleChildren<'_> {}
impl std::iter::FusedIterator for RuleChildren<'_> {}

impl std::fmt::Debug for Rule {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Rule")
            .field("kind", &self.kind())
            .field("depth", &self.stats.depth)
            .field("nodes", &self.stats.nodes)
            .field("operands", &self.stats.operands)
            .field("json_nodes", &self.stats.json_nodes)
            .field("json_depth", &self.stats.json_depth)
            .field("text_bytes", &self.stats.text_bytes)
            .finish()
    }
}

impl std::fmt::Debug for RuleRef<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuleRef")
            .field("node", &self.node)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RuleView<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Value(_) => formatter.write_str("Value(<protected>)"),
            Self::Predicate(_) => formatter.write_str("Predicate(<protected>)"),
            Self::All(children) => formatter
                .debug_struct("All")
                .field("children", &children.len())
                .finish(),
            Self::Any(children) => formatter
                .debug_struct("Any")
                .field("children", &children.len())
                .finish(),
            Self::Not(_) => formatter.write_str("Not(<protected>)"),
            Self::Deferred(_) => formatter.write_str("Deferred(<protected>)"),
            Self::Described { .. } => formatter.write_str("Described(<protected>)"),
        }
    }
}

impl std::fmt::Debug for RuleChildren<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuleChildren")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl Serialize for Rule {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.check_limits().map_err(serde::ser::Error::custom)?;
        self.as_ref().serialize(serializer)
    }
}

impl Serialize for RuleRef<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.view() {
            RuleView::Value(value) => value.serialize(serializer),
            RuleView::Predicate(predicate) => predicate.serialize(serializer),
            RuleView::All(children) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("all", &SerializableChildren(children))?;
                map.end()
            },
            RuleView::Any(children) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("any", &SerializableChildren(children))?;
                map.end()
            },
            RuleView::Not(inner) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("not", &inner)?;
                map.end()
            },
            RuleView::Deferred(deferred) => deferred.serialize(serializer),
            RuleView::Described { inner, message } => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("described", &(inner, message))?;
                map.end()
            },
        }
    }
}

struct SerializableChildren<'a>(RuleChildren<'a>);

impl Serialize for SerializableChildren<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;

        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for child in self.0.clone() {
            sequence.serialize_element(&child)?;
        }
        sequence.end()
    }
}

impl Rule {
    pub(super) fn from_leaf(node: RuleNode) -> Result<Self, RuleBuildError> {
        let stats = match limits::measure_leaf(&node) {
            Ok(stats) => stats,
            Err(error) => {
                drop_rule_node_payloads(node);
                return Err(error);
            },
        };
        Ok(Self {
            nodes: vec![node],
            root: 0,
            stats,
        })
    }

    pub(in crate::rule) fn from_bounded_parts(
        nodes: Vec<RuleNode>,
        root: NodeId,
        stats: RuleStats,
    ) -> Self {
        debug_assert!(root < nodes.len());
        debug_assert!(limits::check_rule_limits_parts(&nodes, root).is_ok());
        Self { nodes, root, stats }
    }

    pub(super) fn into_shifted_nodes(mut self, offset: usize) -> (Vec<RuleNode>, NodeId) {
        for node in &mut self.nodes {
            node.shift_child_ids(offset);
        }
        let root = self.root + offset;
        (self.nodes, root)
    }

    pub(in crate::rule) const fn stats(&self) -> RuleStats {
        self.stats
    }

    pub(super) fn node(&self, node: NodeId) -> &RuleNode {
        &self.nodes[node]
    }

    fn as_ref(&self) -> RuleRef<'_> {
        RuleRef {
            rule: self,
            node: self.root,
        }
    }

    fn ref_at(&self, node: NodeId) -> RuleRef<'_> {
        RuleRef { rule: self, node }
    }

    fn children_at<'a>(&'a self, children: &'a [NodeId]) -> RuleChildren<'a> {
        RuleChildren {
            rule: self,
            children: children.iter(),
        }
    }

    fn view_at(&self, node: NodeId) -> RuleView<'_> {
        match self.node(node) {
            RuleNode::Value(value) => RuleView::Value(value),
            RuleNode::Predicate(predicate) => RuleView::Predicate(predicate),
            RuleNode::All(children) => RuleView::All(self.children_at(children)),
            RuleNode::Any(children) => RuleView::Any(self.children_at(children)),
            RuleNode::Not(inner) => RuleView::Not(self.ref_at(*inner)),
            RuleNode::Deferred(deferred) => RuleView::Deferred(deferred),
            RuleNode::Described { inner, message } => RuleView::Described {
                inner: self.ref_at(*inner),
                message,
            },
        }
    }

    /// Returns a read-only view of the root rule node.
    #[must_use]
    pub fn view(&self) -> RuleView<'_> {
        self.as_ref().view()
    }

    /// Returns a borrowed reference to the root rule node.
    #[must_use]
    pub fn root(&self) -> RuleRef<'_> {
        self.as_ref()
    }

    /// Checks the fixed complexity budgets for this complete rule.
    ///
    /// This iterative verification is primarily an admission assertion for
    /// consumers. Public construction and deserialization already guarantee it.
    ///
    /// # Errors
    /// Returns the exhausted budget without retaining or rendering rule text.
    pub fn check_limits(&self) -> Result<(), RuleBuildError> {
        limits::check_rule_limits(self)
    }

    /// Validates an input against this rule using the given execution mode.
    ///
    /// Partial modes report unavailable checks as [`EvaluationOutcome::Deferred`].
    /// `Full` reports unavailable context or evaluators as errors. A deferred
    /// branch never counts as satisfied under logical composition.
    pub fn validate(
        &self,
        input: &serde_json::Value,
        ctx: Option<&PredicateContext>,
        mode: ExecutionMode,
        disclosure: DiagnosticDisclosure,
    ) -> Result<EvaluationOutcome, ValidationError> {
        self.as_ref().validate_bounded(input, ctx, mode, disclosure)
    }

    /// Classifies this rule by semantic kind.
    #[must_use]
    pub fn kind(&self) -> RuleKind {
        let mut current = self.as_ref();
        loop {
            match current.view() {
                RuleView::Value(_) => return RuleKind::Value,
                RuleView::Predicate(_) => return RuleKind::Predicate,
                RuleView::All(_) | RuleView::Any(_) | RuleView::Not(_) => return RuleKind::Logic,
                RuleView::Deferred(_) => return RuleKind::Deferred,
                RuleView::Described { inner, .. } => current = inner,
            }
        }
    }

    /// True if this rule needs runtime context (Deferred).
    #[must_use]
    pub fn is_deferred(&self) -> bool {
        matches!(self.kind(), RuleKind::Deferred)
    }

    /// Collects all field IDs referenced by context predicates in this rule.
    pub fn field_references<'a>(&'a self, out: &mut Vec<&'a str>) {
        let mut pending = vec![self.as_ref()];
        while let Some(rule) = pending.pop() {
            match rule.view() {
                RuleView::Predicate(predicate) => out.push(predicate.field().as_str()),
                RuleView::All(children) | RuleView::Any(children) => pending.extend(children),
                RuleView::Not(inner) | RuleView::Described { inner, .. } => pending.push(inner),
                RuleView::Value(_) | RuleView::Deferred(_) => {},
            }
        }
    }

    /// Boolean predicate evaluation against a structured context.
    ///
    /// # Errors
    /// Value and deferred rules are invalid in conditions. Pending dependencies
    /// produce `Unavailable`, including beneath `any` and `not`.
    pub fn matches(&self, ctx: &PredicateContext) -> Result<bool, ValidationError> {
        self.as_ref().matches_bounded(ctx)
    }
}

impl RuleRef<'_> {
    fn validate_bounded(
        self,
        input: &serde_json::Value,
        ctx: Option<&PredicateContext>,
        mode: ExecutionMode,
        disclosure: DiagnosticDisclosure,
    ) -> Result<EvaluationOutcome, ValidationError> {
        match self.view() {
            RuleView::Value(_) | RuleView::Predicate(_) if mode == ExecutionMode::Deferred => {
                Ok(EvaluationOutcome::Deferred(vec![
                    DeferredReason::StaticRule,
                ]))
            },
            RuleView::Value(value) => value
                .validate_value(input, disclosure)
                .map(|()| EvaluationOutcome::Satisfied),
            RuleView::Predicate(predicate) => validate_predicate(predicate, ctx, mode, disclosure),
            RuleView::All(children) => logic::validate_all(children, input, ctx, mode, disclosure),
            RuleView::Any(children) => logic::validate_any(children, input, ctx, mode, disclosure),
            RuleView::Not(inner) => logic::validate_not(inner, input, ctx, mode, disclosure),
            RuleView::Deferred(_) if mode == ExecutionMode::StaticOnly => {
                Ok(EvaluationOutcome::Deferred(vec![
                    DeferredReason::DeferredRule,
                ]))
            },
            RuleView::Deferred(deferred) => deferred
                .validate(input, ctx)
                .map(|()| EvaluationOutcome::Satisfied),
            RuleView::Described { inner, message } => {
                let outcome = inner.validate_bounded(input, ctx, mode, disclosure);
                match disclosure {
                    DiagnosticDisclosure::IncludeValue => outcome.map_err(|mut error| {
                        let rendered =
                            crate::foundation::error::render_template(message, error.params());
                        error.message = std::borrow::Cow::Owned(rendered.into_owned());
                        error
                    }),
                    DiagnosticDisclosure::OmitValue => outcome,
                }
            },
        }
    }

    fn matches_bounded(self, ctx: &PredicateContext) -> Result<bool, ValidationError> {
        match self.view() {
            RuleView::Value(_) | RuleView::Deferred(_) => Err(ValidationError::invalid_rule(
                "conditions may contain only predicates and logical combinators",
            )),
            RuleView::Predicate(predicate) => predicate.evaluate(ctx),
            RuleView::All(children) => logic::matches_all(children, ctx),
            RuleView::Any(children) => logic::matches_any(children, ctx),
            RuleView::Not(inner) => inner.matches_bounded(ctx).map(|matched| !matched),
            RuleView::Described { inner, .. } => inner.matches_bounded(ctx),
        }
    }
}

/// Four semantic rule kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RuleKind {
    /// Value-validation rule.
    Value,
    /// Context predicate.
    Predicate,
    /// Logical combinator.
    Logic,
    /// Deferred runtime rule.
    Deferred,
}

fn validate_predicate(
    predicate: &Predicate,
    ctx: Option<&PredicateContext>,
    mode: ExecutionMode,
    disclosure: DiagnosticDisclosure,
) -> Result<EvaluationOutcome, ValidationError> {
    match ctx {
        Some(context) => match predicate.evaluate(context) {
            Ok(true) => Ok(EvaluationOutcome::Satisfied),
            Ok(false) => Err(predicate_error(predicate, disclosure)),
            Err(error)
                if mode == ExecutionMode::StaticOnly
                    && error.kind() == ValidationErrorKind::Unavailable =>
            {
                Ok(EvaluationOutcome::Deferred(vec![
                    DeferredReason::PredicateContext,
                ]))
            },
            Err(error) => Err(error),
        },
        None if mode == ExecutionMode::StaticOnly => Ok(EvaluationOutcome::Deferred(vec![
            DeferredReason::PredicateContext,
        ])),
        None => Err(
            ValidationError::unavailable("predicate context is required")
                .with_field_path(predicate.field().clone()),
        ),
    }
}

fn predicate_code(predicate: &Predicate) -> &'static str {
    match predicate {
        Predicate::Eq(..) => "eq_failed",
        Predicate::Ne(..) => "ne_failed",
        Predicate::Gt(..) => "gt_failed",
        Predicate::Gte(..) => "gte_failed",
        Predicate::Lt(..) => "lt_failed",
        Predicate::Lte(..) => "lte_failed",
        Predicate::IsTrue(_) => "is_true_failed",
        Predicate::IsFalse(_) => "is_false_failed",
        Predicate::Set(_) => "set_failed",
        Predicate::Empty(_) => "empty_failed",
        Predicate::Contains(..) => "contains_failed",
        Predicate::Matches(..) => "matches_failed",
        Predicate::In(..) => "in_failed",
    }
}

fn predicate_error(predicate: &Predicate, disclosure: DiagnosticDisclosure) -> ValidationError {
    let error = ValidationError::new(predicate_code(predicate), "predicate failed")
        .with_field_path(predicate.field().clone());
    if disclosure == DiagnosticDisclosure::OmitValue {
        return error;
    }
    match predicate {
        Predicate::Eq(_, value) | Predicate::Ne(_, value) | Predicate::Contains(_, value) => {
            error.with_param("expected", format!("{value}"))
        },
        Predicate::Gt(_, number)
        | Predicate::Gte(_, number)
        | Predicate::Lt(_, number)
        | Predicate::Lte(_, number) => error.with_param("expected", number.to_string()),
        Predicate::In(_, values) => {
            let allowed = values
                .iter()
                .map(|value| format!("{value}"))
                .collect::<Vec<_>>()
                .join(", ");
            error.with_param("allowed", allowed)
        },
        Predicate::IsTrue(_)
        | Predicate::IsFalse(_)
        | Predicate::Set(_)
        | Predicate::Empty(_)
        | Predicate::Matches(..) => error,
    }
}

enum JsonChildren {
    Array(std::vec::IntoIter<serde_json::Value>),
    Object(serde_json::map::IntoIter),
}

impl JsonChildren {
    fn next_value(&mut self) -> Option<serde_json::Value> {
        match self {
            Self::Array(values) => values.next(),
            Self::Object(values) => values.next().map(|(_, value)| value),
        }
    }
}

pub(super) fn drop_json_value_iteratively(value: serde_json::Value) {
    let mut current = Some(value);
    let mut parents = Vec::new();
    loop {
        if let Some(value) = current.take() {
            match value {
                serde_json::Value::Array(values) => {
                    parents.push(JsonChildren::Array(values.into_iter()));
                },
                serde_json::Value::Object(values) => {
                    parents.push(JsonChildren::Object(values.into_iter()));
                },
                serde_json::Value::Null
                | serde_json::Value::Bool(_)
                | serde_json::Value::Number(_)
                | serde_json::Value::String(_) => {},
            }
        }

        let Some(parent) = parents.last_mut() else {
            break;
        };
        if let Some(value) = parent.next_value() {
            current = Some(value);
        } else {
            parents.pop();
        }
    }
}

fn drop_json_values_iteratively(values: Vec<serde_json::Value>) {
    for value in values {
        drop_json_value_iteratively(value);
    }
}

fn drop_rule_node_payloads(node: RuleNode) {
    match node {
        RuleNode::Value(ValueRule::OneOf(values))
        | RuleNode::Predicate(Predicate::In(_, values)) => {
            drop_json_values_iteratively(values);
        },
        RuleNode::Predicate(
            Predicate::Eq(_, value) | Predicate::Ne(_, value) | Predicate::Contains(_, value),
        ) => drop_json_value_iteratively(value),
        RuleNode::Value(_)
        | RuleNode::Predicate(_)
        | RuleNode::All(_)
        | RuleNode::Any(_)
        | RuleNode::Not(_)
        | RuleNode::Deferred(_)
        | RuleNode::Described { .. } => {},
    }
}

impl crate::foundation::Validate<serde_json::Value> for Rule {
    fn validate(&self, input: &serde_json::Value) -> Result<(), ValidationError> {
        Rule::validate(
            self,
            input,
            None,
            ExecutionMode::Full,
            DiagnosticDisclosure::OmitValue,
        )?
        .require_satisfied()
    }
}
