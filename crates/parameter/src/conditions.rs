//! Backward-compatible re-export of the unified [`Rule`] type.
//!
//! The standalone `Condition` enum has been merged into [`Rule`].
//! Context predicates (`Eq`, `Ne`, `IsTrue`, `Set`, etc.) and logical
//! combinators (`All`, `Any`, `Not`) are now `Rule` variants.

use crate::rules::Rule;
use crate::values::FieldValues;

/// Backward-compatible alias — the `Condition` type is now [`Rule`].
pub type Condition = Rule;

/// Evaluate a [`Rule`] predicate against runtime field values.
#[must_use]
pub fn evaluate_condition(condition: &Rule, values: &FieldValues) -> bool {
    condition.evaluate(values)
}
