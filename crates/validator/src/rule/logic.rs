//! Logical combinators — `All`, `Any`, `Not` over nested `Rule`.

use serde::{Deserialize, Serialize};

use super::Rule;
use crate::{engine::ExecutionMode, foundation::ValidationError, rule::context::PredicateContext};

/// Logical combinator. Children are `Rule`, so combinators can mix kinds
/// (e.g. an `All` containing both `ValueRule::Email` and a `Predicate`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Logic {
    /// All children must pass.
    All(Vec<Rule>),
    /// At least one child must pass. Evaluation short-circuits on the first
    /// success; remaining children are not evaluated, so errors from earlier
    /// children that failed are also discarded.
    Any(Vec<Rule>),
    /// Negates the child.
    Not(Rule),
}

impl Logic {
    /// Dispatches the combinator. Errors from children are collected into
    /// the parent's `nested` chain.
    pub fn validate(
        &self,
        input: &serde_json::Value,
        ctx: Option<&PredicateContext>,
        mode: ExecutionMode,
    ) -> Result<(), ValidationError> {
        match self {
            Self::All(rules) => {
                let mut errs = Vec::new();
                for r in rules {
                    if let Err(e) = r.validate(input, ctx, mode) {
                        errs.push(e);
                    }
                }
                if errs.is_empty() {
                    Ok(())
                } else if errs.len() == 1 {
                    Err(errs.into_iter().next().unwrap())
                } else {
                    let n = errs.len();
                    Err(
                        ValidationError::new("all_failed", format!("{n} of the rules failed"))
                            .with_nested(errs),
                    )
                }
            },
            Self::Any(rules) => {
                if rules.is_empty() {
                    return Ok(());
                }
                let mut errs = Vec::new();
                for r in rules {
                    match r.validate(input, ctx, mode) {
                        Ok(()) => return Ok(()),
                        Err(e) => errs.push(e),
                    }
                }
                let n = errs.len();
                Err(
                    ValidationError::new("any_failed", format!("All {n} alternatives failed"))
                        .with_nested(errs),
                )
            },
            Self::Not(inner) => {
                // Skip propagation: when the inner rule would be silently
                // skipped at this dispatch point (Predicate without ctx,
                // Deferred in StaticOnly), Not must also skip. Otherwise
                // `not(predicate)` in a context-free run wrongly errors.
                // Deep propagation through nested Logic is a known limitation
                // of the two-state Result shape (see PR #415 follow-up).
                if inner_would_skip(inner, ctx, mode) {
                    return Ok(());
                }
                match inner.validate(input, ctx, mode) {
                    Ok(()) => Err(ValidationError::new("not_failed", "negated rule passed")),
                    // Inner error is intentionally ignored: Not succeeds when its child fails.
                    Err(_) => Ok(()),
                }
            },
        }
    }

    /// Iterates all direct child rules (shallow — does not recurse).
    pub fn children(&self) -> &[Rule] {
        match self {
            Self::All(v) | Self::Any(v) => v.as_slice(),
            Self::Not(inner) => std::slice::from_ref(inner),
        }
    }
}

/// Returns true if `rule` would be silently skipped at this dispatch point —
/// `Rule::validate` returns `Ok(())` without actually enforcing anything.
/// Used by `Logic::Not` to propagate the skip instead of inverting it to a
/// false failure. Looks through `Described` wrappers; does not recurse into
/// nested `Logic`.
fn inner_would_skip(rule: &Rule, ctx: Option<&PredicateContext>, mode: ExecutionMode) -> bool {
    match rule {
        Rule::Predicate(_) => ctx.is_none(),
        Rule::Deferred(_) => mode == ExecutionMode::StaticOnly,
        Rule::Described(inner, _) => inner_would_skip(inner, ctx, mode),
        _ => false,
    }
}
