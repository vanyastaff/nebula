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
            Self::Not(inner) => match inner.validate(input, ctx, mode) {
                Ok(()) => Err(ValidationError::new("not_failed", "negated rule passed")),
                // Inner error is intentionally ignored: Not succeeds when its child fails.
                Err(_) => Ok(()),
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
