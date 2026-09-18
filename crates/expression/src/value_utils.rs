//! Coercion and numeric helpers over [`RuntimeValue`].

use std::cmp::Ordering;

use num_cmp::NumCmp;

use crate::{ExpressionError, ExpressionResult, value::RuntimeValue};

/// Borrow the type name of a value for error messages.
pub(crate) fn value_type_name(value: &RuntimeValue) -> &'static str {
    value.type_name()
}

/// Check if a value is truthy (not null/undefined, false, 0, empty string/array/object).
pub(crate) fn is_truthy(value: &RuntimeValue) -> bool {
    match value {
        RuntimeValue::Null | RuntimeValue::Undefined => false,
        RuntimeValue::Bool(value) => *value,
        RuntimeValue::Integer(value) => *value != 0,
        RuntimeValue::Unsigned(value) => *value != 0,
        RuntimeValue::Float(value) => *value != 0.0 && !value.is_nan(),
        RuntimeValue::String(text) => !text.is_empty(),
        RuntimeValue::Array(values) => !values.is_empty(),
        RuntimeValue::Object(entries) => !entries.is_empty(),
        RuntimeValue::DateTime(_) => true,
    }
}

/// Convert a value to boolean (truthy/falsy semantics).
pub(crate) fn to_boolean(value: &RuntimeValue) -> bool {
    is_truthy(value)
}

/// Convert a value to `i64` with a human-readable failure reason.
pub(crate) fn to_integer(value: &RuntimeValue) -> Result<i64, &'static str> {
    match value {
        RuntimeValue::Integer(value) => Ok(*value),
        RuntimeValue::Unsigned(value) => {
            i64::try_from(*value).map_err(|_| "number is not an integer")
        },
        RuntimeValue::Float(_) => value.as_i64().ok_or("number is not an integer"),
        RuntimeValue::String(text) => text.parse().map_err(|_| "string is not a valid integer"),
        RuntimeValue::Bool(value) => Ok(i64::from(*value)),
        _ => Err("value cannot be converted to integer"),
    }
}

/// Convert a value to `f64` with a human-readable failure reason.
pub(crate) fn to_float(value: &RuntimeValue) -> Result<f64, &'static str> {
    let float = match value {
        RuntimeValue::Integer(value) => *value as f64,
        RuntimeValue::Unsigned(value) => *value as f64,
        RuntimeValue::Float(value) => *value,
        RuntimeValue::String(text) => text.parse().map_err(|_| "string is not a valid number")?,
        RuntimeValue::Bool(value) => {
            if *value {
                1.0
            } else {
                0.0
            }
        },
        _ => return Err("value cannot be converted to number"),
    };
    if float.is_finite() {
        Ok(float)
    } else {
        Err("number is not finite")
    }
}

/// Parse finite JSON numeric syntax without rounding out-of-range integer text.
pub(crate) fn parse_number(text: &str) -> Result<RuntimeValue, &'static str> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|_| "expected a finite JSON number")?;
    let value = RuntimeValue::from_json(&json);
    if let RuntimeValue::Float(_) = value
        && !text.contains(['.', 'e', 'E'])
    {
        return Err("integer is outside the JSON integer range");
    }
    match value {
        RuntimeValue::Integer(_) | RuntimeValue::Unsigned(_) | RuntimeValue::Float(_) => Ok(value),
        _ => Err("expected a finite JSON number"),
    }
}

/// Adapt the three numeric representations to the shared exact comparator.
///
/// Mixed integer/float comparisons delegate to `num-cmp`, so values above
/// 2^53 compare exactly. `None` means "not comparable as numbers".
pub(crate) fn compare_numbers(left: &RuntimeValue, right: &RuntimeValue) -> Option<Ordering> {
    match (left, right) {
        (RuntimeValue::Integer(left), RuntimeValue::Integer(right)) => left.num_cmp(*right),
        (RuntimeValue::Integer(left), RuntimeValue::Unsigned(right)) => left.num_cmp(*right),
        (RuntimeValue::Integer(left), RuntimeValue::Float(right)) => left.num_cmp(*right),
        (RuntimeValue::Unsigned(left), RuntimeValue::Integer(right)) => left.num_cmp(*right),
        (RuntimeValue::Unsigned(left), RuntimeValue::Unsigned(right)) => left.num_cmp(*right),
        (RuntimeValue::Unsigned(left), RuntimeValue::Float(right)) => left.num_cmp(*right),
        (RuntimeValue::Float(left), RuntimeValue::Integer(right)) => left.num_cmp(*right),
        (RuntimeValue::Float(left), RuntimeValue::Unsigned(right)) => left.num_cmp(*right),
        (RuntimeValue::Float(left), RuntimeValue::Float(right)) => left.num_cmp(*right),
        _ => None,
    }
}

/// Narrow an `i128` integer result back into the JSON integer range.
pub(crate) fn integer_result(
    value: Option<i128>,
    operation: &'static str,
) -> ExpressionResult<RuntimeValue> {
    let narrowed = value.and_then(|value| {
        i64::try_from(value)
            .map(RuntimeValue::Integer)
            .ok()
            .or_else(|| u64::try_from(value).map(RuntimeValue::Unsigned).ok())
    });
    if let Some(value) = narrowed {
        Ok(value)
    } else {
        tracing::debug!(operation, "integer arithmetic overflow");
        Err(ExpressionError::NumericOverflow { operation })
    }
}

/// Reject a non-finite float result instead of letting it become silent null.
pub(crate) fn finite_result(value: f64, operation: &'static str) -> ExpressionResult<RuntimeValue> {
    if value.is_finite() {
        Ok(RuntimeValue::Float(value))
    } else {
        tracing::debug!(operation, "non-finite numeric result");
        Err(ExpressionError::NonFiniteNumber { operation })
    }
}

/// Count Unicode scalar values (Rust `char`s) in a string.
///
/// **Note on n8n / JavaScript parity.** JavaScript's `String.length`
/// counts UTF-16 code units, so `"🙂".length` is 2 (the emoji is a
/// surrogate pair). Rust strings can't store unpaired surrogates, so
/// matching that exactly would require returning shapes Rust cannot
/// produce safely. This function instead counts Unicode scalar values:
/// `"🙂"` is 1, `"über"` is 4. That keeps `length` / `substring` /
/// padding builtins in agreement with each other (you cannot index
/// into half a codepoint) at the cost of a documented deviation from
/// JS for non-BMP input. Both `str::len` (UTF-8 bytes) and
/// `chars().count()` (this) are wrong in different ways relative to JS;
/// scalar-value count is the closest stable behaviour Rust supports.
#[inline]
pub(crate) fn char_count(s: &str) -> i64 {
    s.chars().count() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_type_name() {
        assert_eq!(value_type_name(&RuntimeValue::Null), "null");
        assert_eq!(value_type_name(&RuntimeValue::Undefined), "undefined");
        assert_eq!(value_type_name(&RuntimeValue::Bool(true)), "boolean");
        assert_eq!(value_type_name(&RuntimeValue::Integer(42)), "number");
        assert_eq!(value_type_name(&RuntimeValue::Unsigned(42)), "number");
        assert_eq!(value_type_name(&RuntimeValue::Float(1.5)), "number");
        assert_eq!(value_type_name(&RuntimeValue::string("test")), "string");
        assert_eq!(value_type_name(&RuntimeValue::array(vec![])), "array");
        assert_eq!(
            value_type_name(&RuntimeValue::object(Default::default())),
            "object"
        );
        assert_eq!(
            value_type_name(&RuntimeValue::date_time_utc(chrono::Utc::now())),
            "date"
        );
    }

    #[test]
    fn test_is_truthy() {
        assert!(!is_truthy(&RuntimeValue::Null));
        assert!(!is_truthy(&RuntimeValue::Undefined));
        assert!(!is_truthy(&RuntimeValue::Bool(false)));
        assert!(is_truthy(&RuntimeValue::Bool(true)));
        assert!(!is_truthy(&RuntimeValue::Integer(0)));
        assert!(is_truthy(&RuntimeValue::Integer(1)));
        assert!(!is_truthy(&RuntimeValue::string(String::new())));
        assert!(is_truthy(&RuntimeValue::string("test")));
    }

    #[test]
    fn to_integer_rejects_fractional_and_out_of_range_numbers() {
        for value in [
            RuntimeValue::Float(1.5),
            RuntimeValue::Unsigned(u64::MAX),
            RuntimeValue::Float(9_223_372_036_854_775_808.0),
        ] {
            to_integer(&value).unwrap_err();
        }
        assert_eq!(to_integer(&RuntimeValue::Float(12.0)).unwrap(), 12);
        assert_eq!(
            to_integer(&RuntimeValue::Integer(i64::MIN)).unwrap(),
            i64::MIN
        );
    }

    #[test]
    fn exact_mixed_comparison_spans_every_representation() {
        use std::cmp::Ordering;

        assert_eq!(
            compare_numbers(
                &RuntimeValue::Unsigned(9_007_199_254_740_993),
                &RuntimeValue::Float(9_007_199_254_740_992.0)
            ),
            Some(Ordering::Greater)
        );
        assert_eq!(
            compare_numbers(
                &RuntimeValue::Integer(-1),
                &RuntimeValue::Unsigned(u64::MAX)
            ),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_numbers(&RuntimeValue::Integer(0), &RuntimeValue::string("0")),
            None
        );
    }

    #[test]
    fn overflow_and_non_finite_results_are_typed_errors() {
        assert!(matches!(
            integer_result(None, "addition"),
            Err(ExpressionError::NumericOverflow { .. })
        ));
        assert!(matches!(
            finite_result(f64::INFINITY, "division"),
            Err(ExpressionError::NonFiniteNumber { .. })
        ));
    }
}
