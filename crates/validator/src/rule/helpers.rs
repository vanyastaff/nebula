//! Internal helpers shared across `Rule` impls: regex compilation, JSON
//! number comparison, message overrides, and precision-safe predicates.

use crate::foundation::ValidationError;

/// Compiles a regex pattern, returning a validation error if invalid.
pub(super) fn compile_regex(pattern: &str) -> Result<regex::Regex, ValidationError> {
    regex::Regex::new(pattern)
        .map_err(|e| ValidationError::new("invalid_pattern", format!("invalid regex: {e}")))
}

// ============================================================================
// JSON NUMBER HELPERS — precision-safe comparison for integers > 2^53
// ============================================================================

/// Ordering between two JSON numbers, using the highest-precision path.
///
/// Tries `i64` first, then `u64`, then `f64`. Returns `None` if either
/// operand is not a number or if a `NaN` comparison is indeterminate.
pub(super) fn json_number_cmp(
    value: &serde_json::Value,
    bound: &serde_json::Number,
) -> Option<std::cmp::Ordering> {
    let val_num = value.as_number()?;

    // i64 path — covers most integers exactly
    if let (Some(a), Some(b)) = (val_num.as_i64(), bound.as_i64()) {
        return Some(a.cmp(&b));
    }

    // u64 path — covers large positive integers that don't fit in i64
    if let (Some(a), Some(b)) = (val_num.as_u64(), bound.as_u64()) {
        return Some(a.cmp(&b));
    }

    // f64 fallback — handles floats; may lose precision for very large ints
    let a = val_num.as_f64()?;
    let b = bound.as_f64()?;
    a.partial_cmp(&b)
}

pub(super) fn format_json_number(n: &serde_json::Number) -> String {
    n.to_string()
}

/// Replaces the error message if an override is provided.
pub(super) fn override_message(
    mut error: ValidationError,
    message: &Option<String>,
) -> ValidationError {
    if let Some(msg) = message {
        error.message = std::borrow::Cow::Owned(msg.clone());
    }
    error
}

/// Precision-safe numeric comparison for predicate evaluation.
pub(super) fn cmp_number_predicate(
    value: Option<&serde_json::Value>,
    rhs: &serde_json::Number,
    expected: impl Fn(std::cmp::Ordering) -> bool,
) -> bool {
    let Some(val) = value else { return false };
    json_number_cmp(val, rhs).is_some_and(expected)
}
