//! Shared condition model used by `core.if` and `core.switch`.
//!
//! A [`Condition`] is a single field-level predicate evaluated against the
//! top-level keys of a JSON object. `evaluate_condition` dispatches over
//! [`ConditionOp`] and returns a `bool` or an `ActionError::Fatal` for
//! configuration errors (missing required value, type mismatch, missing field
//! on ordered comparisons).
//!
//! ## Normalising `data`
//!
//! Call `normalize_data` before calling `evaluate_condition`:
//! - `None` / `null` → `{}`
//! - non-object → `ActionError::Fatal` naming the actual type
//!
//! ## Operators
//!
//! See the `core.if` module doc for the full per-operator semantics table.

use nebula_action::ActionError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::util::ValueTypeNameStr;

// ── Wire types ────────────────────────────────────────────────────────────────

/// The comparison operator applied to a single top-level field.
///
/// Variants without a `value` operand (`Exists`, `NotExists`, `Truthy`)
/// ignore the `value` field in [`Condition`].
///
/// Forward-compatibility for new optional fields is handled via
/// `#[serde(default)]` in future versions, not `#[non_exhaustive]`, because
/// these types are deserialized from workflow JSON rather than
/// literal-constructed by external Rust code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOp {
    /// JSON deep-equality: missing field evaluates as `false`.
    Eq,
    /// Logical negation of `Eq`: missing field evaluates as `true`.
    Ne,
    /// Strictly greater than. Both operands must have the same JSON kind
    /// (both numbers or both strings); missing field is a Fatal error.
    Gt,
    /// Greater than or equal. Same rules as `Gt`.
    Gte,
    /// Strictly less than. Same rules as `Gt`.
    Lt,
    /// Less than or equal. Same rules as `Gt`.
    Lte,
    /// True when the field key is present (any value, including `null`).
    Exists,
    /// True when the field key is absent.
    NotExists,
    /// True when the field value is truthy (see `core.if` module doc table).
    Truthy,
}

/// A single field-level predicate.
///
/// The `value` operand is required for `Eq`/`Ne`/`Gt`/`Gte`/`Lt`/`Lte`;
/// it is ignored for `Exists`, `NotExists`, and `Truthy`.
///
/// `serde(default)` on `value` lets workflows omit it for operator kinds
/// that do not use it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    /// Top-level key to test in `data`.
    pub field: String,
    /// Comparison operator.
    pub op: ConditionOp,
    /// Right-hand operand. **Required** (enforced at runtime) for `Eq`, `Ne`,
    /// `Gt`, `Gte`, `Lt`, and `Lte` — absence returns `ActionError::Fatal`.
    /// Ignored for `Exists`, `NotExists`, and `Truthy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

// ── Data normalisation ────────────────────────────────────────────────────────

/// Normalise `data` to a JSON object.
///
/// `null` and absent values become `{}`. Any other non-object type is a Fatal
/// error because field-level conditions require an object to index into.
pub(crate) fn normalize_data(data: Option<Value>) -> Result<Value, ActionError> {
    match data {
        Some(Value::Object(_)) | None => Ok(data.unwrap_or(Value::Object(Default::default()))),
        Some(Value::Null) => Ok(Value::Object(Default::default())),
        Some(other) => Err(ActionError::fatal(format!(
            "`data` must be a JSON object or null, got {}",
            other.type_name_str()
        ))),
    }
}

// ── Condition evaluation ──────────────────────────────────────────────────────

/// Assert that `condition.value` is present for operators that require it.
///
/// `Eq`, `Ne`, `Gt`, `Gte`, `Lt`, `Lte` all need a right-hand operand.
/// Returning `None` from the workflow definition for those operators is a
/// Fatal configuration error — the action fails closed instead of silently
/// treating the missing value as `null`.
pub(crate) fn require_value(op: ConditionOp, value: &Option<Value>) -> Result<&Value, ActionError> {
    value.as_ref().ok_or_else(|| {
        ActionError::fatal(format!(
            "operator `{op:?}` requires a `value` field, but none was provided"
        ))
    })
}

/// Evaluate `cond` against the top-level fields of `data_object`.
///
/// See the `core.if` module doc for the full semantics per operator.
pub(crate) fn evaluate_condition(
    data_object: &Value,
    cond: &Condition,
) -> Result<bool, ActionError> {
    let field_value = data_object.get(&cond.field);

    match cond.op {
        ConditionOp::Eq => {
            let expected = require_value(ConditionOp::Eq, &cond.value)?;
            let Some(actual) = field_value else {
                return Ok(false);
            };
            Ok(actual == expected)
        },

        ConditionOp::Ne => {
            let expected = require_value(ConditionOp::Ne, &cond.value)?;
            let Some(actual) = field_value else {
                // Missing field is "not equal" to anything.
                return Ok(true);
            };
            Ok(actual != expected)
        },

        ConditionOp::Gt | ConditionOp::Gte | ConditionOp::Lt | ConditionOp::Lte => {
            let expected = require_value(cond.op, &cond.value)?;
            let Some(actual) = field_value else {
                return Err(ActionError::fatal(format!(
                    "ordered comparison requires field `{}` to be present, but it is missing",
                    cond.field
                )));
            };
            evaluate_ordered(cond.op, actual, expected)
        },

        ConditionOp::Exists => Ok(field_value.is_some()),

        ConditionOp::NotExists => Ok(field_value.is_none()),

        ConditionOp::Truthy => Ok(is_truthy(field_value)),
    }
}

/// Evaluate an ordered operator (`Gt`/`Gte`/`Lt`/`Lte`) between two JSON values.
///
/// Both values must be the same JSON kind: both numbers (compared via f64) or
/// both strings (lexicographic byte order). Any other combination is a Fatal
/// error that names both types.
pub(crate) fn evaluate_ordered(
    op: ConditionOp,
    actual: &Value,
    expected: &Value,
) -> Result<bool, ActionError> {
    match (actual, expected) {
        (Value::Number(lhs), Value::Number(rhs)) => {
            // f64 is the common numeric type in serde_json; precision loss on
            // integers larger than 2^53 is a known limitation (documented in
            // the core.if module doc; exact large-integer ops deferred to v2).
            let lhs_f64 = lhs.as_f64().ok_or_else(|| {
                ActionError::fatal(format!(
                    "could not represent left-hand number `{lhs}` as f64"
                ))
            })?;
            let rhs_f64 = rhs.as_f64().ok_or_else(|| {
                ActionError::fatal(format!(
                    "could not represent right-hand number `{rhs}` as f64"
                ))
            })?;
            let result = match op {
                ConditionOp::Gt => lhs_f64 > rhs_f64,
                ConditionOp::Gte => lhs_f64 >= rhs_f64,
                ConditionOp::Lt => lhs_f64 < rhs_f64,
                ConditionOp::Lte => lhs_f64 <= rhs_f64,
                _ => unreachable!("evaluate_ordered called with non-ordered op"),
            };
            Ok(result)
        },
        (Value::String(lhs), Value::String(rhs)) => {
            let result = match op {
                ConditionOp::Gt => lhs > rhs,
                ConditionOp::Gte => lhs >= rhs,
                ConditionOp::Lt => lhs < rhs,
                ConditionOp::Lte => lhs <= rhs,
                _ => unreachable!("evaluate_ordered called with non-ordered op"),
            };
            Ok(result)
        },
        (lhs_val, rhs_val) => Err(ActionError::fatal(format!(
            "cannot apply ordered comparison to {} and {}",
            lhs_val.type_name_str(),
            rhs_val.type_name_str()
        ))),
    }
}

/// Truthiness per the table in the `core.if` module doc.
///
/// Missing field (`None`) is falsy.
pub(crate) fn is_truthy(field_value: Option<&Value>) -> bool {
    match field_value {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Null) => false,
        Some(Value::Number(n)) => {
            // Both 0 and 0.0 are falsy; any other number is truthy.
            n.as_f64() != Some(0.0)
        },
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(arr)) => !arr.is_empty(),
        Some(Value::Object(obj)) => !obj.is_empty(),
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use nebula_action::ActionError;
    use serde_json::json;

    use super::*;

    // ── normalize_data ────────────────────────────────────────────────────────

    #[test]
    fn normalize_none_yields_empty_object() {
        let result = normalize_data(None).unwrap();
        assert_eq!(result, json!({}));
    }

    #[test]
    fn normalize_null_yields_empty_object() {
        let result = normalize_data(Some(json!(null))).unwrap();
        assert_eq!(result, json!({}));
    }

    #[test]
    fn normalize_object_passes_through() {
        let obj = json!({"a": 1});
        let result = normalize_data(Some(obj.clone())).unwrap();
        assert_eq!(result, obj);
    }

    #[test]
    fn normalize_array_returns_fatal() {
        let err = normalize_data(Some(json!([1, 2]))).unwrap_err();
        assert!(matches!(err, ActionError::Fatal { .. }));
    }

    #[test]
    fn normalize_string_returns_fatal() {
        let err = normalize_data(Some(json!("hello"))).unwrap_err();
        assert!(matches!(err, ActionError::Fatal { .. }));
    }

    #[test]
    fn normalize_number_returns_fatal() {
        let err = normalize_data(Some(json!(42))).unwrap_err();
        assert!(matches!(err, ActionError::Fatal { .. }));
    }

    #[test]
    fn normalize_bool_returns_fatal() {
        let err = normalize_data(Some(json!(true))).unwrap_err();
        assert!(matches!(err, ActionError::Fatal { .. }));
    }

    // ── is_truthy ─────────────────────────────────────────────────────────────

    #[test]
    fn truthy_none_is_false() {
        assert!(!is_truthy(None));
    }

    #[test]
    fn truthy_false_bool_is_false() {
        assert!(!is_truthy(Some(&json!(false))));
    }

    #[test]
    fn truthy_true_bool_is_true() {
        assert!(is_truthy(Some(&json!(true))));
    }

    #[test]
    fn truthy_null_is_false() {
        assert!(!is_truthy(Some(&json!(null))));
    }

    #[test]
    fn truthy_zero_int_is_false() {
        assert!(!is_truthy(Some(&json!(0))));
    }

    #[test]
    fn truthy_zero_float_is_false() {
        assert!(!is_truthy(Some(&json!(0.0))));
    }

    #[test]
    fn truthy_nonzero_int_is_true() {
        assert!(is_truthy(Some(&json!(42))));
    }

    #[test]
    fn truthy_empty_string_is_false() {
        assert!(!is_truthy(Some(&json!(""))));
    }

    #[test]
    fn truthy_non_empty_string_is_true() {
        assert!(is_truthy(Some(&json!("hi"))));
    }

    #[test]
    fn truthy_empty_array_is_false() {
        assert!(!is_truthy(Some(&json!([]))));
    }

    #[test]
    fn truthy_non_empty_array_is_true() {
        assert!(is_truthy(Some(&json!([1]))));
    }

    #[test]
    fn truthy_empty_object_is_false() {
        assert!(!is_truthy(Some(&json!({}))));
    }

    #[test]
    fn truthy_non_empty_object_is_true() {
        assert!(is_truthy(Some(&json!({"k": 1}))));
    }

    // ── evaluate_condition — Eq ───────────────────────────────────────────────

    #[test]
    fn eq_matching_field_is_true() {
        let data = json!({"status": "active"});
        let cond = Condition {
            field: "status".into(),
            op: ConditionOp::Eq,
            value: Some(json!("active")),
        };
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn eq_non_matching_field_is_false() {
        let data = json!({"status": "inactive"});
        let cond = Condition {
            field: "status".into(),
            op: ConditionOp::Eq,
            value: Some(json!("active")),
        };
        assert!(!evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn eq_missing_field_is_false() {
        let data = json!({"other": 1});
        let cond = Condition {
            field: "status".into(),
            op: ConditionOp::Eq,
            value: Some(json!("active")),
        };
        assert!(!evaluate_condition(&data, &cond).unwrap());
    }

    // RED witness: absent `value` for Eq must Fatal, not silently compare as null.
    #[test]
    fn eq_missing_value_returns_fatal() {
        let data = json!({"x": "hello"});
        let cond = Condition {
            field: "x".into(),
            op: ConditionOp::Eq,
            value: None,
        };
        let err = evaluate_condition(&data, &cond).unwrap_err();
        assert!(matches!(err, ActionError::Fatal { .. }));
    }

    // ── evaluate_condition — Ne ───────────────────────────────────────────────

    #[test]
    fn ne_different_is_true() {
        let data = json!({"status": "inactive"});
        let cond = Condition {
            field: "status".into(),
            op: ConditionOp::Ne,
            value: Some(json!("active")),
        };
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn ne_same_is_false() {
        let data = json!({"status": "active"});
        let cond = Condition {
            field: "status".into(),
            op: ConditionOp::Ne,
            value: Some(json!("active")),
        };
        assert!(!evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn ne_missing_field_is_true() {
        let data = json!({});
        let cond = Condition {
            field: "status".into(),
            op: ConditionOp::Ne,
            value: Some(json!("active")),
        };
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    // ── evaluate_condition — Gt / ordered ────────────────────────────────────

    #[test]
    fn gt_numbers_true() {
        let data = json!({"score": 10});
        let cond = Condition {
            field: "score".into(),
            op: ConditionOp::Gt,
            value: Some(json!(5)),
        };
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn gt_numbers_false() {
        let data = json!({"score": 3});
        let cond = Condition {
            field: "score".into(),
            op: ConditionOp::Gt,
            value: Some(json!(5)),
        };
        assert!(!evaluate_condition(&data, &cond).unwrap());
    }

    // RED witness: type mismatch must Fatal, not silently return false.
    #[test]
    fn gt_type_mismatch_returns_fatal() {
        let data = json!({"score": 10});
        let cond = Condition {
            field: "score".into(),
            op: ConditionOp::Gt,
            value: Some(json!("five")),
        };
        let err = evaluate_condition(&data, &cond).unwrap_err();
        assert!(matches!(err, ActionError::Fatal { .. }));
    }

    // RED witness: missing field on ordered op must Fatal, not return false.
    #[test]
    fn gt_missing_field_returns_fatal() {
        let data = json!({});
        let cond = Condition {
            field: "score".into(),
            op: ConditionOp::Gt,
            value: Some(json!(5)),
        };
        let err = evaluate_condition(&data, &cond).unwrap_err();
        assert!(matches!(err, ActionError::Fatal { .. }));
    }

    // ── evaluate_condition — Exists / NotExists ───────────────────────────────

    #[test]
    fn exists_present_is_true() {
        let data = json!({"key": null});
        let cond = Condition {
            field: "key".into(),
            op: ConditionOp::Exists,
            value: None,
        };
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn exists_absent_is_false() {
        let data = json!({});
        let cond = Condition {
            field: "missing".into(),
            op: ConditionOp::Exists,
            value: None,
        };
        assert!(!evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn not_exists_absent_is_true() {
        let data = json!({});
        let cond = Condition {
            field: "missing".into(),
            op: ConditionOp::NotExists,
            value: None,
        };
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn not_exists_present_is_false() {
        let data = json!({"key": "val"});
        let cond = Condition {
            field: "key".into(),
            op: ConditionOp::NotExists,
            value: None,
        };
        assert!(!evaluate_condition(&data, &cond).unwrap());
    }

    // ── ConditionOp serde ────────────────────────────────────────────────────

    #[test]
    fn condition_op_serde_roundtrip() {
        for op in [
            ConditionOp::Eq,
            ConditionOp::Ne,
            ConditionOp::Gt,
            ConditionOp::Gte,
            ConditionOp::Lt,
            ConditionOp::Lte,
            ConditionOp::Exists,
            ConditionOp::NotExists,
            ConditionOp::Truthy,
        ] {
            let serialized = serde_json::to_string(&op).unwrap();
            let round_tripped: ConditionOp = serde_json::from_str(&serialized).unwrap();
            assert_eq!(
                round_tripped, op,
                "ConditionOp::{op:?} must survive a serde round-trip"
            );
        }
    }
}
