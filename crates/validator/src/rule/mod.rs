//! Unified declarative rule system — sum-of-sums over typed kinds.
//!
//! [`Rule`] is a classifier over four semantic kinds:
//!
//! | Kind | Inner type | Method | Wire tag |
//! |---|---|---|---|
//! | Value validation | [`ValueRule`] | `validate_value(&Value)` | `{"min_length": 3}` etc. |
//! | Context predicate | [`Predicate`] | `evaluate(&PredicateContext)` | `{"eq": ["/path", value]}` |
//! | Logical combinator | [`Logic`] | recursive `validate` | `{"all": [...]}` |
//! | Deferred | [`DeferredRule`] | runtime-evaluated | `{"custom": "expr"}` |
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

/// Borrowed view over a value bag used by predicate rules.
///
/// Kept for backward compatibility with callers that built their own
/// `HashMap<String, Value>`-based contexts. New code should prefer
/// [`PredicateContext`].
pub trait RuleContext {
    /// Fetch a value by key.
    fn get(&self, key: &str) -> Option<&serde_json::Value>;
}

impl RuleContext for std::collections::HashMap<String, serde_json::Value> {
    fn get(&self, key: &str) -> Option<&serde_json::Value> {
        std::collections::HashMap::get(self, key)
    }
}

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

    /// Evaluates this rule as a boolean predicate against field values.
    ///
    /// **Compat bridge**: this method exists for callers that still pass a
    /// flat `HashMap<String, Value>` (via [`RuleContext`]). New code should
    /// use [`Rule::validate`] with a [`PredicateContext`], which handles
    /// nested JSON Pointer paths (`/user/email`) correctly.
    ///
    /// Value and Deferred rules always return `true` (they are not
    /// predicates). Predicates look up their field via `ctx.get(key)` where
    /// the lookup key is the predicate's path with the leading `/` stripped.
    ///
    /// # Known limitation
    ///
    /// For nested paths like `/user/email`, this method strips the leading
    /// `/` and does a flat key lookup for `"user/email"`. Flat `RuleContext`
    /// implementations won't have such a key, so nested predicates will
    /// silently evaluate to `false`. Use
    /// `Rule::validate(input, Some(&ctx), mode)` instead for nested-path
    /// predicate evaluation.
    ///
    /// TODO(post-refactor): retire this method once `schema::validated`
    /// migrates to `PredicateContext::from_json`.
    #[must_use]
    pub fn evaluate(&self, ctx: &dyn RuleContext) -> bool {
        match self {
            Self::Value(_) | Self::Deferred(_) => true,
            Self::Predicate(p) => evaluate_predicate_via_rule_context(p, ctx),
            Self::Logic(l) => match l.as_ref() {
                Logic::All(rules) => rules.iter().all(|r| r.evaluate(ctx)),
                Logic::Any(rules) => rules.iter().any(|r| r.evaluate(ctx)),
                Logic::Not(inner) => !inner.evaluate(ctx),
            },
            Self::Described(inner, _) => inner.evaluate(ctx),
        }
    }
}

/// Evaluates a single `Predicate` against a legacy `RuleContext`.
///
/// The `FieldPath` is flattened to a legacy flat key by stripping the
/// leading `/` (e.g. `/status` → `status`). Multi-segment paths are
/// passed through verbatim after stripping the slash; `RuleContext`
/// implementations that only understand flat keys will return `None`
/// for nested paths.
fn evaluate_predicate_via_rule_context(p: &Predicate, ctx: &dyn RuleContext) -> bool {
    fn lookup_key(p: &Predicate) -> &str {
        let s = p.field().as_str();
        s.strip_prefix('/').unwrap_or(s)
    }
    let key = lookup_key(p);

    fn number_cmp(
        ctx_val: Option<&serde_json::Value>,
        rhs: &serde_json::Number,
        expected: impl Fn(std::cmp::Ordering) -> bool,
    ) -> bool {
        let Some(v) = ctx_val else { return false };
        // Try i64/u64/f64 comparison in decreasing precision order.
        let num = match v.as_number() {
            Some(n) => n,
            None => return false,
        };
        let ord = if let (Some(a), Some(b)) = (num.as_i64(), rhs.as_i64()) {
            a.cmp(&b)
        } else if let (Some(a), Some(b)) = (num.as_u64(), rhs.as_u64()) {
            a.cmp(&b)
        } else if let (Some(a), Some(b)) = (num.as_f64(), rhs.as_f64()) {
            match a.partial_cmp(&b) {
                Some(o) => o,
                None => return false,
            }
        } else {
            return false;
        };
        expected(ord)
    }

    match p {
        Predicate::Eq(_, v) => ctx.get(key).is_some_and(|x| x == v),
        Predicate::Ne(_, v) => ctx.get(key).is_none_or(|x| x != v),
        Predicate::Gt(_, rhs) => number_cmp(ctx.get(key), rhs, |o| o.is_gt()),
        Predicate::Gte(_, rhs) => number_cmp(ctx.get(key), rhs, |o| o.is_ge()),
        Predicate::Lt(_, rhs) => number_cmp(ctx.get(key), rhs, |o| o.is_lt()),
        Predicate::Lte(_, rhs) => number_cmp(ctx.get(key), rhs, |o| o.is_le()),
        Predicate::IsTrue(_) => ctx.get(key).and_then(serde_json::Value::as_bool) == Some(true),
        Predicate::IsFalse(_) => ctx.get(key).and_then(serde_json::Value::as_bool) == Some(false),
        Predicate::Set(_) => ctx.get(key).is_some_and(|v| {
            !v.is_null()
                && match v {
                    serde_json::Value::String(s) => !s.is_empty(),
                    serde_json::Value::Array(a) => !a.is_empty(),
                    _ => true,
                }
        }),
        Predicate::Empty(_) => ctx.get(key).is_none_or(|v| {
            v.is_null()
                || match v {
                    serde_json::Value::String(s) => s.is_empty(),
                    serde_json::Value::Array(a) => a.is_empty(),
                    _ => false,
                }
        }),
        Predicate::Contains(_, v) => ctx.get(key).is_some_and(|x| match x {
            serde_json::Value::String(s) => v.as_str().is_some_and(|needle| s.contains(needle)),
            serde_json::Value::Array(items) => items.contains(v),
            _ => false,
        }),
        Predicate::Matches(_, pat) => ctx
            .get(key)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|s| regex::Regex::new(pat).is_ok_and(|re| re.is_match(s))),
        Predicate::In(_, allowed) => ctx.get(key).is_some_and(|x| allowed.contains(x)),
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
        Predicate::Eq(_, _) => "eq_failed",
        Predicate::Ne(_, _) => "ne_failed",
        Predicate::Gt(_, _) => "gt_failed",
        Predicate::Gte(_, _) => "gte_failed",
        Predicate::Lt(_, _) => "lt_failed",
        Predicate::Lte(_, _) => "lte_failed",
        Predicate::IsTrue(_) => "is_true_failed",
        Predicate::IsFalse(_) => "is_false_failed",
        Predicate::Set(_) => "set_failed",
        Predicate::Empty(_) => "empty_failed",
        Predicate::Contains(_, _) => "contains_failed",
        Predicate::Matches(_, _) => "matches_failed",
        Predicate::In(_, _) => "in_failed",
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
        | Predicate::Matches(_, _) => err,
    }
}

impl crate::foundation::Validate<serde_json::Value> for Rule {
    fn validate(&self, input: &serde_json::Value) -> Result<(), ValidationError> {
        Rule::validate(self, input, None, ExecutionMode::StaticOnly)
    }
}
