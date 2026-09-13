//! Exact JSON number comparisons and diagnostic number formatting.

use num_cmp::NumCmp;

// ============================================================================
// JSON NUMBER HELPERS — precision-safe comparison for integers > 2^53
// ============================================================================

/// Ordering between two JSON numbers, using the highest-precision path.
///
/// Dispatches the JSON representations without converting integers to floats.
/// `num-cmp` supplies exact signed/unsigned/float comparisons.
pub(super) fn json_number_cmp(
    value: &serde_json::Value,
    bound: &serde_json::Number,
) -> Option<std::cmp::Ordering> {
    let val_num = value.as_number()?;

    match (
        val_num.as_i64(),
        val_num.as_u64(),
        bound.as_i64(),
        bound.as_u64(),
    ) {
        (Some(left), _, Some(right), _) => left.num_cmp(right),
        (Some(left), _, _, Some(right)) => left.num_cmp(right),
        (Some(left), _, _, _) => left.num_cmp(bound.as_f64()?),
        (_, Some(left), Some(right), _) => left.num_cmp(right),
        (_, Some(left), _, Some(right)) => left.num_cmp(right),
        (_, Some(left), _, _) => left.num_cmp(bound.as_f64()?),
        (_, _, Some(right), _) => val_num.as_f64()?.num_cmp(right),
        (_, _, _, Some(right)) => val_num.as_f64()?.num_cmp(right),
        _ => val_num.as_f64()?.num_cmp(bound.as_f64()?),
    }
}

pub(super) fn format_json_number(n: &serde_json::Number) -> String {
    n.to_string()
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
