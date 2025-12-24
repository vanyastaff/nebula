//! Condition types for validation and display systems.
//!
//! This module provides type aliases for semantic clarity when using conditions
//! in different contexts (validation vs display).

use super::display::DisplayCondition;

/// Condition for cross-field validation rules.
///
/// This is an alias for [`DisplayCondition`] since both validation and display
/// systems use the same set of conditions to evaluate field values.
///
/// # Examples
///
/// ```rust
/// use nebula_parameter::core::FieldCondition;
/// use nebula_value::Value;
///
/// // Check if payment type is "card"
/// let condition = FieldCondition::Equals(Value::text("card"));
/// assert!(condition.evaluate(&Value::text("card")));
///
/// // Check if value is set
/// let condition = FieldCondition::IsSet;
/// assert!(condition.evaluate(&Value::integer(42)));
/// assert!(!condition.evaluate(&Value::Null));
///
/// // Numeric comparison
/// let condition = FieldCondition::GreaterThan(18.0);
/// assert!(condition.evaluate(&Value::integer(21)));
/// ```
pub type FieldCondition = DisplayCondition;
