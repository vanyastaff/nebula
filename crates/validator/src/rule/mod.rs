//! Unified declarative rule system — sum-of-sums over typed kinds.
//!
//! [`Rule`] is a classifier over four semantic kinds:
//!
//! | Kind | Inner type | Method | Wire tag |
//! |---|---|---|---|
//! | Value validation | `ValueRule` | `validate_value(&Value)` | `{"min_length": 3}` etc. |
//! | Context predicate | `Predicate` | `evaluate(&PredicateContext)` | `{"eq": ["/path", value]}` |
//! | Logical combinator | `Logic` | recursive `validate` | `{"all": [...]}` |
//! | Deferred | `DeferredRule` | runtime-evaluated | `{"custom": "expr"}` |
//!
//! The `Described(Box<Rule>, String)` decorator wraps any rule with a
//! custom message (may contain `{placeholder}` templates).
//!
//! Unit variants (`Email`, `Url`) serialize as bare strings.

pub mod context;
pub mod deferred;
mod deserialize;
pub mod logic;
pub mod predicate;
pub mod value;

mod constructors;
mod helpers;

#[cfg(test)]
mod tests;

pub use context::PredicateContext;
pub use deferred::DeferredRule;
pub use logic::Logic;
pub use predicate::Predicate;
use serde::{Serialize, ser::SerializeMap};
pub use value::ValueRule;

use crate::{engine::ExecutionMode, foundation::ValidationError};

/// Unified declarative rule. See module docs.
///
/// `Serialize` is manual because `Described` must emit as
/// `{"described": [inner, msg]}`, which `#[serde(untagged)]` on a tuple
/// variant cannot produce. `Deserialize` is manual for friendly
/// unknown-variant errors (see `rule/deserialize.rs`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Rule {
    /// Value-validation rule.
    Value(ValueRule),
    /// Context predicate.
    Predicate(Predicate),
    /// Logical combinator.
    Logic(Box<Logic>),
    /// Deferred runtime rule.
    Deferred(DeferredRule),
    /// Wrapper with custom error message.
    Described(Box<Rule>, String),
}

impl Serialize for Rule {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Value(v) => v.serialize(s),
            Self::Predicate(p) => p.serialize(s),
            Self::Logic(l) => l.as_ref().serialize(s),
            Self::Deferred(d) => d.serialize(s),
            Self::Described(inner, msg) => {
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("described", &(inner.as_ref(), msg))?;
                m.end()
            },
        }
    }
}

impl Rule {
    /// Validates an input against this rule using the given execution mode.
    ///
    /// `ctx = None` short-circuits `Predicate` dispatch to `Ok(())` —
    /// matching the two-tier semantics (sync client skips, full server
    /// evaluates). Deferred rules are skipped when
    /// `mode == ExecutionMode::StaticOnly`.
    pub fn validate(
        &self,
        input: &serde_json::Value,
        ctx: Option<&PredicateContext>,
        mode: ExecutionMode,
    ) -> Result<(), ValidationError> {
        match self {
            Self::Value(v) => v.validate_value(input),
            Self::Predicate(p) => match ctx {
                Some(c) if p.evaluate(c) => Ok(()),
                Some(_) => Err(predicate_error(p)),
                None => Ok(()),
            },
            Self::Logic(l) => l.validate(input, ctx, mode),
            Self::Deferred(_) if mode == ExecutionMode::StaticOnly => Ok(()),
            Self::Deferred(d) => d.validate(input, ctx),
            Self::Described(inner, msg) => inner.validate(input, ctx, mode).map_err(|mut e| {
                // Render the user-provided template against the inner error's
                // params eagerly — consumers that read `err.message` directly
                // (not via Display) expect the substituted string, not the raw
                // `{name}` template. Fast path for plain messages costs nothing.
                let rendered = crate::foundation::error::render_template(msg, e.params());
                e.message = std::borrow::Cow::Owned(rendered.into_owned());
                e
            }),
        }
    }

    /// Classifies this rule by kind — cheap non-recursive check.
    #[must_use]
    pub fn kind(&self) -> RuleKind {
        match self {
            Self::Value(_) => RuleKind::Value,
            Self::Predicate(_) => RuleKind::Predicate,
            Self::Logic(_) => RuleKind::Logic,
            Self::Deferred(_) => RuleKind::Deferred,
            Self::Described(inner, _) => inner.kind(),
        }
    }

    /// True if this rule needs runtime context (Deferred).
    #[must_use]
    pub fn is_deferred(&self) -> bool {
        matches!(self.kind(), RuleKind::Deferred)
    }

    /// Collects all field IDs referenced by **context predicates** in this rule.
    ///
    /// Recurses into `Logic` combinators and `Described` wrappers. `Value` rules
    /// have no field refs; `Deferred::UniqueBy` carries a sub-path but is not
    /// collected here — it's evaluated at runtime against array elements, not
    /// a sibling field context.
    pub fn field_references<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Self::Predicate(p) => out.push(p.field().as_str()),
            Self::Logic(l) => {
                for child in l.children() {
                    child.field_references(out);
                }
            },
            Self::Described(inner, _) => inner.field_references(out),
            Self::Value(_) | Self::Deferred(_) => {},
        }
    }

    /// Boolean predicate evaluation against a structured context.
    ///
    /// Resolves nested JSON-Pointer paths via [`PredicateContext`]
    /// (`/a/b`-style sibling lookups). `Value` and `Deferred` rules are not
    /// predicates and evaluate to `true`.
    #[must_use]
    pub fn matches(&self, ctx: &PredicateContext) -> bool {
        match self {
            Self::Value(_) | Self::Deferred(_) => true,
            Self::Predicate(p) => p.evaluate(ctx),
            Self::Logic(l) => match l.as_ref() {
                Logic::All(rules) => rules.iter().all(|r| r.matches(ctx)),
                Logic::Any(rules) => rules.iter().any(|r| r.matches(ctx)),
                Logic::Not(inner) => !inner.matches(ctx),
            },
            Self::Described(inner, _) => inner.matches(ctx),
        }
    }
}

/// Four kinds of rule for classification.
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

fn predicate_code(p: &Predicate) -> &'static str {
    match p {
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

/// Constructs a `ValidationError` for a failed predicate, injecting
/// variant-specific operand params so message templates can render
/// `{expected}` / `{allowed}` placeholders.
fn predicate_error(p: &Predicate) -> ValidationError {
    let err = ValidationError::new(predicate_code(p), "predicate failed")
        .with_field_path(p.field().clone());
    match p {
        Predicate::Eq(_, v) | Predicate::Ne(_, v) | Predicate::Contains(_, v) => {
            err.with_param("expected", format!("{v}"))
        },
        Predicate::Gt(_, n) | Predicate::Gte(_, n) | Predicate::Lt(_, n) | Predicate::Lte(_, n) => {
            err.with_param("expected", n.to_string())
        },
        Predicate::In(_, vs) => {
            let allowed = vs
                .iter()
                .map(|v| format!("{v}"))
                .collect::<Vec<_>>()
                .join(", ");
            err.with_param("allowed", allowed)
        },
        // IsTrue / IsFalse / Set / Empty / Matches carry no operand
        // expected-value — no param injection needed.
        Predicate::IsTrue(_)
        | Predicate::IsFalse(_)
        | Predicate::Set(_)
        | Predicate::Empty(_)
        | Predicate::Matches(..) => err,
    }
}

impl crate::foundation::Validate<serde_json::Value> for Rule {
    fn validate(&self, input: &serde_json::Value) -> Result<(), ValidationError> {
        Rule::validate(self, input, None, ExecutionMode::StaticOnly)
    }
}
