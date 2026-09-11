//! Complexity limits for declarative rules.

use serde_json::Value;

use super::{DeferredRule, Predicate, Rule, RuleNode, ValueRule};

/// Maximum number of nested [`Rule`] levels, including the root rule.
pub const MAX_RULE_DEPTH: usize = 64;
/// Maximum number of [`Rule`] nodes in one rule.
pub const MAX_RULE_NODES: usize = 1_024;
/// Maximum aggregate number of logical, decorator, and set operands.
pub const MAX_RULE_OPERANDS: usize = 1_024;
/// Maximum aggregate number of JSON value nodes retained by one rule.
pub const MAX_RULE_JSON_NODES: usize = 1_024;
/// Maximum depth of a JSON value retained as a rule operand.
pub const MAX_RULE_JSON_DEPTH: usize = 64;
/// Maximum aggregate UTF-8 bytes of user-controlled rule text.
///
/// This includes field paths, patterns, deferred expressions, descriptions,
/// and strings or object keys nested in JSON operands.
pub const MAX_RULE_TEXT_BYTES: usize = 16 * 1_024;

/// The bounded resource that rejected a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RuleBudget {
    /// Nested rule depth.
    Depth,
    /// Total rule nodes.
    Nodes,
    /// Total logical and set operands.
    Operands,
    /// Aggregate JSON value nodes.
    JsonNodes,
    /// Nested JSON value depth.
    JsonDepth,
    /// Total user-controlled UTF-8 bytes.
    TextBytes,
}

impl RuleBudget {
    /// Stable diagnostic name for this budget.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Depth => "depth",
            Self::Nodes => "nodes",
            Self::Operands => "operands",
            Self::JsonNodes => "json_nodes",
            Self::JsonDepth => "json_depth",
            Self::TextBytes => "text_bytes",
        }
    }
}

/// A rule could not be constructed or admitted safely.
///
/// Errors contain only the violated budget and fixed limit. They never retain
/// or render rule expressions, messages, paths, patterns, or JSON operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RuleBuildError {
    /// Rule nesting exceeds [`MAX_RULE_DEPTH`].
    #[error("rule depth limit is {limit}")]
    DepthLimit {
        /// Configured maximum depth.
        limit: usize,
    },
    /// Rule node count exceeds [`MAX_RULE_NODES`].
    #[error("rule node limit is {limit}")]
    NodeLimit {
        /// Configured maximum node count.
        limit: usize,
    },
    /// Rule operand count exceeds [`MAX_RULE_OPERANDS`].
    #[error("rule operand limit is {limit}")]
    OperandLimit {
        /// Configured maximum operand count.
        limit: usize,
    },
    /// JSON values contain too many aggregate nodes.
    #[error("rule JSON node limit is {limit}")]
    JsonNodeLimit {
        /// Configured maximum JSON node count.
        limit: usize,
    },
    /// A JSON value is nested too deeply.
    #[error("rule JSON depth limit is {limit}")]
    JsonDepthLimit {
        /// Configured maximum JSON depth.
        limit: usize,
    },
    /// Rule text exceeds [`MAX_RULE_TEXT_BYTES`].
    #[error("rule text limit is {limit} bytes")]
    TextLimit {
        /// Configured maximum text bytes.
        limit: usize,
    },
    /// A regular expression is invalid.
    #[error("invalid rule pattern")]
    InvalidPattern,
    /// A field path is invalid.
    #[error("invalid rule field path")]
    InvalidFieldPath,
}

impl RuleBuildError {
    /// Returns the exhausted budget and its fixed limit, if this is a budget error.
    #[must_use]
    pub const fn budget(self) -> Option<(RuleBudget, usize)> {
        match self {
            Self::DepthLimit { limit } => Some((RuleBudget::Depth, limit)),
            Self::NodeLimit { limit } => Some((RuleBudget::Nodes, limit)),
            Self::OperandLimit { limit } => Some((RuleBudget::Operands, limit)),
            Self::JsonNodeLimit { limit } => Some((RuleBudget::JsonNodes, limit)),
            Self::JsonDepthLimit { limit } => Some((RuleBudget::JsonDepth, limit)),
            Self::TextLimit { limit } => Some((RuleBudget::TextBytes, limit)),
            Self::InvalidPattern | Self::InvalidFieldPath => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RuleStats {
    pub(super) depth: usize,
    pub(super) nodes: usize,
    pub(super) operands: usize,
    pub(super) json_nodes: usize,
    pub(super) json_depth: usize,
    pub(super) text_bytes: usize,
}

impl RuleStats {
    pub(super) fn compose(
        children: &[Rule],
        direct_operands: usize,
        direct_text_bytes: usize,
    ) -> Result<Self, RuleBuildError> {
        let mut stats = Self {
            depth: 1,
            nodes: 1,
            operands: direct_operands,
            json_nodes: 0,
            json_depth: 0,
            text_bytes: direct_text_bytes,
        };
        for child in children {
            let child = child.stats();
            stats.depth = stats.depth.max(child.depth.saturating_add(1));
            stats.nodes = stats.nodes.saturating_add(child.nodes);
            stats.operands = stats.operands.saturating_add(child.operands);
            stats.json_nodes = stats.json_nodes.saturating_add(child.json_nodes);
            stats.json_depth = stats.json_depth.max(child.json_depth);
            stats.text_bytes = stats.text_bytes.saturating_add(child.text_bytes);
        }
        stats.ensure_within_limits()?;
        Ok(stats)
    }

    fn ensure_within_limits(self) -> Result<(), RuleBuildError> {
        if self.depth > MAX_RULE_DEPTH {
            return Err(RuleBuildError::DepthLimit {
                limit: MAX_RULE_DEPTH,
            });
        }
        if self.nodes > MAX_RULE_NODES {
            return Err(RuleBuildError::NodeLimit {
                limit: MAX_RULE_NODES,
            });
        }
        if self.operands > MAX_RULE_OPERANDS {
            return Err(RuleBuildError::OperandLimit {
                limit: MAX_RULE_OPERANDS,
            });
        }
        if self.json_nodes > MAX_RULE_JSON_NODES {
            return Err(RuleBuildError::JsonNodeLimit {
                limit: MAX_RULE_JSON_NODES,
            });
        }
        if self.json_depth > MAX_RULE_JSON_DEPTH {
            return Err(RuleBuildError::JsonDepthLimit {
                limit: MAX_RULE_JSON_DEPTH,
            });
        }
        if self.text_bytes > MAX_RULE_TEXT_BYTES {
            return Err(RuleBuildError::TextLimit {
                limit: MAX_RULE_TEXT_BYTES,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub(super) struct RuleBudgetState {
    nodes: usize,
    operands: usize,
    json_nodes: usize,
    json_depth: usize,
    text_bytes: usize,
}

impl RuleBudgetState {
    pub(super) fn enter_rule(&mut self, depth: usize) -> Result<(), RuleBuildError> {
        if depth > MAX_RULE_DEPTH {
            return Err(RuleBuildError::DepthLimit {
                limit: MAX_RULE_DEPTH,
            });
        }
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_RULE_NODES {
            return Err(RuleBuildError::NodeLimit {
                limit: MAX_RULE_NODES,
            });
        }
        Ok(())
    }

    pub(super) fn add_operands(&mut self, count: usize) -> Result<(), RuleBuildError> {
        self.operands = self.operands.saturating_add(count);
        if self.operands > MAX_RULE_OPERANDS {
            return Err(RuleBuildError::OperandLimit {
                limit: MAX_RULE_OPERANDS,
            });
        }
        Ok(())
    }

    pub(super) fn add_text(&mut self, text: &str) -> Result<(), RuleBuildError> {
        self.add_text_bytes(text.len())
    }

    pub(super) fn add_text_bytes(&mut self, count: usize) -> Result<(), RuleBuildError> {
        self.text_bytes = self.text_bytes.saturating_add(count);
        if self.text_bytes > MAX_RULE_TEXT_BYTES {
            return Err(RuleBuildError::TextLimit {
                limit: MAX_RULE_TEXT_BYTES,
            });
        }
        Ok(())
    }

    pub(super) fn enter_json(&mut self, depth: usize) -> Result<(), RuleBuildError> {
        if depth > MAX_RULE_JSON_DEPTH {
            return Err(RuleBuildError::JsonDepthLimit {
                limit: MAX_RULE_JSON_DEPTH,
            });
        }
        self.json_depth = self.json_depth.max(depth);
        self.json_nodes = self.json_nodes.saturating_add(1);
        if self.json_nodes > MAX_RULE_JSON_NODES {
            return Err(RuleBuildError::JsonNodeLimit {
                limit: MAX_RULE_JSON_NODES,
            });
        }
        Ok(())
    }

    pub(super) fn add_json_value(&mut self, value: &Value) -> Result<(), RuleBuildError> {
        let mut pending = vec![(value, 1_usize)];
        while let Some((value, depth)) = pending.pop() {
            self.enter_json(depth)?;
            match value {
                Value::String(text) => self.add_text(text)?,
                Value::Array(values) => {
                    self.ensure_pending_json_capacity(pending.len(), values.len())?;
                    let child_depth = depth.saturating_add(1);
                    if !values.is_empty() && child_depth > MAX_RULE_JSON_DEPTH {
                        return Err(RuleBuildError::JsonDepthLimit {
                            limit: MAX_RULE_JSON_DEPTH,
                        });
                    }
                    pending.extend(values.iter().map(|value| (value, child_depth)));
                },
                Value::Object(values) => {
                    self.ensure_pending_json_capacity(pending.len(), values.len())?;
                    let child_depth = depth.saturating_add(1);
                    if !values.is_empty() && child_depth > MAX_RULE_JSON_DEPTH {
                        return Err(RuleBuildError::JsonDepthLimit {
                            limit: MAX_RULE_JSON_DEPTH,
                        });
                    }
                    for (key, value) in values {
                        self.add_text(key)?;
                        pending.push((value, child_depth));
                    }
                },
                Value::Null | Value::Bool(_) | Value::Number(_) => {},
            }
        }
        Ok(())
    }

    fn ensure_pending_json_capacity(
        &self,
        pending: usize,
        additional: usize,
    ) -> Result<(), RuleBuildError> {
        if self
            .json_nodes
            .saturating_add(pending)
            .saturating_add(additional)
            > MAX_RULE_JSON_NODES
        {
            return Err(RuleBuildError::JsonNodeLimit {
                limit: MAX_RULE_JSON_NODES,
            });
        }
        Ok(())
    }

    fn finish(self, depth: usize) -> Result<RuleStats, RuleBuildError> {
        let stats = RuleStats {
            depth,
            nodes: self.nodes,
            operands: self.operands,
            json_nodes: self.json_nodes,
            json_depth: self.json_depth,
            text_bytes: self.text_bytes,
        };
        stats.ensure_within_limits()?;
        Ok(stats)
    }
}

pub(super) fn measure_leaf(node: &RuleNode) -> Result<RuleStats, RuleBuildError> {
    let mut budget = RuleBudgetState::default();
    budget.enter_rule(1)?;
    account_node(node, &mut budget)?;
    budget.finish(1)
}

pub(super) fn check_rule_limits(rule: &Rule) -> Result<(), RuleBuildError> {
    let measured = measure_parts(&rule.nodes, rule.root)?;
    debug_assert_eq!(measured, rule.stats);
    Ok(())
}

pub(super) fn check_rule_limits_parts(
    nodes: &[RuleNode],
    root: usize,
) -> Result<(), RuleBuildError> {
    measure_parts(nodes, root).map(|_| ())
}

fn measure_parts(nodes: &[RuleNode], root: usize) -> Result<RuleStats, RuleBuildError> {
    let mut budget = RuleBudgetState::default();
    let mut max_depth = 1;
    let mut pending = vec![(root, 1_usize)];

    while let Some((node_id, depth)) = pending.pop() {
        budget.enter_rule(depth)?;
        max_depth = max_depth.max(depth);
        let Some(node) = nodes.get(node_id) else {
            return Err(RuleBuildError::NodeLimit {
                limit: MAX_RULE_NODES,
            });
        };
        account_node(node, &mut budget)?;
        let child_depth = depth.saturating_add(1);
        match node {
            RuleNode::All(children) | RuleNode::Any(children) => {
                pending.extend(children.iter().map(|child| (*child, child_depth)));
            },
            RuleNode::Not(child) => pending.push((*child, child_depth)),
            RuleNode::Described { inner, .. } => pending.push((*inner, child_depth)),
            RuleNode::Value(_) | RuleNode::Predicate(_) | RuleNode::Deferred(_) => {},
        }
    }

    budget.finish(max_depth)
}

fn account_node(node: &RuleNode, budget: &mut RuleBudgetState) -> Result<(), RuleBuildError> {
    match node {
        RuleNode::Value(value) => account_value_rule(value, budget),
        RuleNode::Predicate(predicate) => account_predicate(predicate, budget),
        RuleNode::All(children) | RuleNode::Any(children) => budget.add_operands(children.len()),
        RuleNode::Not(_) => budget.add_operands(1),
        RuleNode::Deferred(DeferredRule::Custom(expression)) => budget.add_text(expression),
        RuleNode::Deferred(DeferredRule::UniqueBy(path)) => budget.add_text(path.as_str()),
        RuleNode::Described { message, .. } => {
            budget.add_operands(1)?;
            budget.add_text(message)
        },
    }
}

fn account_value_rule(
    rule: &ValueRule,
    budget: &mut RuleBudgetState,
) -> Result<(), RuleBuildError> {
    match rule {
        ValueRule::Pattern(pattern) => budget.add_text(pattern.as_str()),
        ValueRule::OneOf(values) => {
            budget.add_operands(values.len())?;
            for value in values {
                budget.add_json_value(value)?;
            }
            Ok(())
        },
        ValueRule::MinLength(_)
        | ValueRule::MaxLength(_)
        | ValueRule::Min(_)
        | ValueRule::Max(_)
        | ValueRule::GreaterThan(_)
        | ValueRule::LessThan(_)
        | ValueRule::MinItems(_)
        | ValueRule::MaxItems(_)
        | ValueRule::Email
        | ValueRule::Url => Ok(()),
    }
}

fn account_predicate(
    predicate: &Predicate,
    budget: &mut RuleBudgetState,
) -> Result<(), RuleBuildError> {
    budget.add_text(predicate.field().as_str())?;
    match predicate {
        Predicate::Eq(_, value) | Predicate::Ne(_, value) | Predicate::Contains(_, value) => {
            budget.add_json_value(value)
        },
        Predicate::Matches(_, pattern) => budget.add_text(pattern.as_str()),
        Predicate::In(_, values) => {
            budget.add_operands(values.len())?;
            for value in values {
                budget.add_json_value(value)?;
            }
            Ok(())
        },
        Predicate::Gt(_, _)
        | Predicate::Gte(_, _)
        | Predicate::Lt(_, _)
        | Predicate::Lte(_, _)
        | Predicate::IsTrue(_)
        | Predicate::IsFalse(_)
        | Predicate::Set(_)
        | Predicate::Empty(_) => Ok(()),
    }
}
