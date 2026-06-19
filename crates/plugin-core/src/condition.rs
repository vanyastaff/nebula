//! Shared condition model used by `core.if` and `core.switch`.
//!
//! A [`Condition`] is either a leaf field-level predicate or a boolean
//! combinator (`all`, `any`, `not`) that composes leaves recursively.
//! `evaluate_condition` dispatches over the enum and returns a `bool` or an
//! `ActionError::Fatal` for configuration errors.
//!
//! ## Variants
//!
//! | Wire key   | Variant             | Semantics |
//! |------------|---------------------|-----------|
//! | `field`/`op`/`value` | `Leaf`    | single field-level predicate |
//! | `"all": [...]`       | `All`     | logical AND — short-circuits on first false |
//! | `"any": [...]`       | `Any`     | logical OR — short-circuits on first true |
//! | `"not": {...}`       | `Not`     | logical NOT of a single child condition |
//!
//! ## Empty-combinator semantics
//!
//! - `All([])` → `true` (vacuously true — all zero conditions hold)
//! - `Any([])` → `false` (no condition was satisfied)
//!
//! ## Fatal-error propagation
//!
//! A child condition that returns `ActionError::Fatal` propagates immediately
//! via `?`. Sibling conditions after the failing child are **not evaluated**.
//! Recursion depth is bounded by the config size — the workflow JSON is finite.
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
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, MapAccess, Visitor},
    ser::SerializeMap,
};
use serde_json::Value;

use crate::util::ValueTypeNameStr;

// ── Wire types ────────────────────────────────────────────────────────────────

/// The comparison operator applied to a single top-level field.
///
/// Variants without a `value` operand (`Exists`, `NotExists`, `Truthy`)
/// ignore the `value` field in [`Condition::Leaf`].
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

/// A condition: either a leaf field predicate or a boolean combinator.
///
/// ## Wire format
///
/// **Leaf** — flat object with `field`, `op`, and optional `value`:
/// ```json
/// { "field": "status", "op": "eq", "value": "active" }
/// ```
///
/// **All** — `{"all": [...]}`:
/// ```json
/// { "all": [ { "field": "status", "op": "exists" }, { "field": "score", "op": "gt", "value": 0 } ] }
/// ```
///
/// **Any** — `{"any": [...]}`:
/// ```json
/// { "any": [ { "field": "flag", "op": "truthy" } ] }
/// ```
///
/// **Not** — `{"not": {...}}`:
/// ```json
/// { "not": { "field": "archived", "op": "truthy" } }
/// ```
///
/// The deserializer key-sniffs: if the object has key `"all"` it is `All`;
/// `"any"` → `Any`; `"not"` → `Not`; otherwise treated as a `Leaf`.
/// This gives actionable errors for malformed leaves (e.g. unknown `op`
/// value) while still supporting all combinator forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    /// A single field-level predicate.
    ///
    /// `value` is required for `Eq`/`Ne`/`Gt`/`Gte`/`Lt`/`Lte`;
    /// it is ignored for `Exists`, `NotExists`, and `Truthy`.
    Leaf {
        /// Top-level key to test in `data`.
        field: String,
        /// Comparison operator.
        op: ConditionOp,
        /// Right-hand operand. Required (enforced at runtime) for comparison
        /// operators; absence returns `ActionError::Fatal`.
        value: Option<Value>,
    },
    /// Logical AND: all child conditions must hold.
    ///
    /// Short-circuits on the first `false`. `All([])` is `true`.
    All(Vec<Condition>),
    /// Logical OR: at least one child condition must hold.
    ///
    /// Short-circuits on the first `true`. `Any([])` is `false`.
    Any(Vec<Condition>),
    /// Logical NOT: negates the child condition.
    Not(Box<Condition>),
}

// ── Custom Deserialize ────────────────────────────────────────────────────────

/// Private helper that holds the flat fields of a `Leaf` condition.
///
/// Deserializing through a named struct preserves serde's typed error for an
/// unknown `op` value (e.g. `"bogus"`) — the generated derive emits
/// `"unknown variant \`bogus\`, expected one of …"` rather than the generic
/// untagged `"data did not match any variant"`.
#[derive(Deserialize)]
struct LeafFields {
    field: String,
    op: ConditionOp,
    #[serde(default)]
    value: Option<Value>,
}

struct ConditionVisitor;

impl<'de> Visitor<'de> for ConditionVisitor {
    type Value = Condition;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "a condition object with keys `all`, `any`, `not`, or `field`+`op`[+`value`]",
        )
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        // not hot-path — condition eval is at dispatch, not a tight loop.
        // Collect the map into a serde_json::Value so we can key-sniff.
        let raw: Value = Deserialize::deserialize(de::value::MapAccessDeserializer::new(map))?;

        // DISPATCH RULE: leaf-first.
        //
        // A `"field"` key unambiguously identifies a leaf predicate — no
        // legitimate combinator carries one. Check it BEFORE the combinator
        // keys so that a stored leaf with an incidental stray key (e.g.
        // `{"field":"x","op":"eq","value":1,"all":"metadata"}`) continues
        // to deserialize as a leaf, matching the back-compat behaviour of
        // the old derived `Deserialize` (which ignored unknown fields).
        //
        // Only dispatch to a combinator when `"field"` is absent, which is
        // the true combinator shape: `{"all":[...]}` / `{"any":[...]}` /
        // `{"not":{...}}`.
        if raw.get("field").is_some() {
            let leaf: LeafFields = serde_json::from_value(raw).map_err(de::Error::custom)?;
            return Ok(Condition::Leaf {
                field: leaf.field,
                op: leaf.op,
                value: leaf.value,
            });
        }

        // Count how many combinator keys are present before dispatching.
        // A well-formed combinator carries exactly one of `all`, `any`, `not`.
        // Multiple combinator keys on a single object is an ambiguous config —
        // fail closed with a clear error rather than silently picking the first.
        let combinator_count = ["all", "any", "not"]
            .iter()
            .filter(|k| raw.get(*k).is_some())
            .count();

        if combinator_count > 1 {
            return Err(de::Error::custom(
                "ambiguous condition: an object may contain at most one of `all`, `any`, `not`",
            ));
        }

        if let Some(all_val) = raw.get("all") {
            let children: Vec<Condition> =
                serde_json::from_value(all_val.clone()).map_err(de::Error::custom)?;
            return Ok(Condition::All(children));
        }
        if let Some(any_val) = raw.get("any") {
            let children: Vec<Condition> =
                serde_json::from_value(any_val.clone()).map_err(de::Error::custom)?;
            return Ok(Condition::Any(children));
        }
        if let Some(not_val) = raw.get("not") {
            let child: Condition =
                serde_json::from_value(not_val.clone()).map_err(de::Error::custom)?;
            return Ok(Condition::Not(Box::new(child)));
        }

        // No `field` key and no combinator key — delegate to `LeafFields`
        // which will produce the clean "missing field `field`" serde error.
        let leaf: LeafFields = serde_json::from_value(raw).map_err(de::Error::custom)?;
        Ok(Condition::Leaf {
            field: leaf.field,
            op: leaf.op,
            value: leaf.value,
        })
    }
}

impl<'de> Deserialize<'de> for Condition {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(ConditionVisitor)
    }
}

// ── Custom Serialize ──────────────────────────────────────────────────────────

impl Serialize for Condition {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Condition::Leaf { field, op, value } => {
                // Leaf serializes to a flat `{"field":..,"op":..,"value":..}`
                // object, matching the existing wire shape so stored configs
                // and doc-test wire assertions continue to hold.
                // `value` is omitted when `None` (mirrors the old
                // `#[serde(skip_serializing_if = "Option::is_none")]`).
                let field_count = if value.is_some() { 3 } else { 2 };
                let mut map = serializer.serialize_map(Some(field_count))?;
                map.serialize_entry("field", field)?;
                map.serialize_entry("op", op)?;
                if let Some(v) = value {
                    map.serialize_entry("value", v)?;
                }
                map.end()
            },
            Condition::All(children) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("all", children)?;
                map.end()
            },
            Condition::Any(children) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("any", children)?;
                map.end()
            },
            Condition::Not(child) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("not", child.as_ref())?;
                map.end()
            },
        }
    }
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
///
/// `All([])` is `true`, `Any([])` is `false`.
/// A child that returns `ActionError::Fatal` propagates immediately via `?`;
/// no sibling is evaluated after the failure.
/// Recursion is bounded by config size (the workflow JSON is finite).
pub(crate) fn evaluate_condition(
    data_object: &Value,
    cond: &Condition,
) -> Result<bool, ActionError> {
    match cond {
        Condition::Leaf { field, op, value } => {
            let field_value = data_object.get(field.as_str());

            match op {
                ConditionOp::Eq => {
                    let expected = require_value(ConditionOp::Eq, value)?;
                    let Some(actual) = field_value else {
                        return Ok(false);
                    };
                    Ok(actual == expected)
                },

                ConditionOp::Ne => {
                    let expected = require_value(ConditionOp::Ne, value)?;
                    let Some(actual) = field_value else {
                        // Missing field is "not equal" to anything.
                        return Ok(true);
                    };
                    Ok(actual != expected)
                },

                ConditionOp::Gt | ConditionOp::Gte | ConditionOp::Lt | ConditionOp::Lte => {
                    let expected = require_value(*op, value)?;
                    let Some(actual) = field_value else {
                        return Err(ActionError::fatal(format!(
                            "ordered comparison requires field `{field}` to be present, but it is missing"
                        )));
                    };
                    evaluate_ordered(*op, actual, expected)
                },

                ConditionOp::Exists => Ok(field_value.is_some()),

                ConditionOp::NotExists => Ok(field_value.is_none()),

                ConditionOp::Truthy => Ok(is_truthy(field_value)),
            }
        },

        Condition::All(children) => {
            for child in children {
                if !evaluate_condition(data_object, child)? {
                    return Ok(false);
                }
            }
            Ok(true)
        },

        Condition::Any(children) => {
            for child in children {
                if evaluate_condition(data_object, child)? {
                    return Ok(true);
                }
            }
            Ok(false)
        },

        Condition::Not(child) => Ok(!evaluate_condition(data_object, child)?),
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
        let cond = Condition::Leaf {
            field: "status".into(),
            op: ConditionOp::Eq,
            value: Some(json!("active")),
        };
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn eq_non_matching_field_is_false() {
        let data = json!({"status": "inactive"});
        let cond = Condition::Leaf {
            field: "status".into(),
            op: ConditionOp::Eq,
            value: Some(json!("active")),
        };
        assert!(!evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn eq_missing_field_is_false() {
        let data = json!({"other": 1});
        let cond = Condition::Leaf {
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
        let cond = Condition::Leaf {
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
        let cond = Condition::Leaf {
            field: "status".into(),
            op: ConditionOp::Ne,
            value: Some(json!("active")),
        };
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn ne_same_is_false() {
        let data = json!({"status": "active"});
        let cond = Condition::Leaf {
            field: "status".into(),
            op: ConditionOp::Ne,
            value: Some(json!("active")),
        };
        assert!(!evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn ne_missing_field_is_true() {
        let data = json!({});
        let cond = Condition::Leaf {
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
        let cond = Condition::Leaf {
            field: "score".into(),
            op: ConditionOp::Gt,
            value: Some(json!(5)),
        };
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn gt_numbers_false() {
        let data = json!({"score": 3});
        let cond = Condition::Leaf {
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
        let cond = Condition::Leaf {
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
        let cond = Condition::Leaf {
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
        let cond = Condition::Leaf {
            field: "key".into(),
            op: ConditionOp::Exists,
            value: None,
        };
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn exists_absent_is_false() {
        let data = json!({});
        let cond = Condition::Leaf {
            field: "missing".into(),
            op: ConditionOp::Exists,
            value: None,
        };
        assert!(!evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn not_exists_absent_is_true() {
        let data = json!({});
        let cond = Condition::Leaf {
            field: "missing".into(),
            op: ConditionOp::NotExists,
            value: None,
        };
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn not_exists_present_is_false() {
        let data = json!({"key": "val"});
        let cond = Condition::Leaf {
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

    // ── Combinator: All ───────────────────────────────────────────────────────

    fn leaf_exists(field: &str) -> Condition {
        Condition::Leaf {
            field: field.into(),
            op: ConditionOp::Exists,
            value: None,
        }
    }

    fn leaf_gt(field: &str, threshold: i64) -> Condition {
        Condition::Leaf {
            field: field.into(),
            op: ConditionOp::Gt,
            value: Some(json!(threshold)),
        }
    }

    fn leaf_eq(field: &str, val: &str) -> Condition {
        Condition::Leaf {
            field: field.into(),
            op: ConditionOp::Eq,
            value: Some(json!(val)),
        }
    }

    #[test]
    fn all_all_true_is_true() {
        let data = json!({"a": 1, "b": "hello"});
        let cond = Condition::All(vec![leaf_exists("a"), leaf_exists("b")]);
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn all_one_false_is_false() {
        let data = json!({"a": 1});
        // "b" does not exist → leaf_exists("b") is false → All is false
        let cond = Condition::All(vec![leaf_exists("a"), leaf_exists("b")]);
        assert!(!evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn all_empty_is_true() {
        let data = json!({});
        let cond = Condition::All(vec![]);
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    // ── Combinator: Any ───────────────────────────────────────────────────────

    #[test]
    fn any_one_true_is_true() {
        let data = json!({"b": 5});
        // "a" missing (false), "b" exists (true)
        let cond = Condition::Any(vec![leaf_exists("a"), leaf_exists("b")]);
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn any_all_false_is_false() {
        let data = json!({});
        let cond = Condition::Any(vec![leaf_exists("a"), leaf_exists("b")]);
        assert!(!evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn any_empty_is_false() {
        let data = json!({"x": 1});
        let cond = Condition::Any(vec![]);
        assert!(!evaluate_condition(&data, &cond).unwrap());
    }

    // ── Combinator: Not ───────────────────────────────────────────────────────

    #[test]
    fn not_of_true_is_false() {
        let data = json!({"x": 1});
        let cond = Condition::Not(Box::new(leaf_exists("x")));
        assert!(!evaluate_condition(&data, &cond).unwrap());
    }

    #[test]
    fn not_of_false_is_true() {
        let data = json!({});
        let cond = Condition::Not(Box::new(leaf_exists("x")));
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    // ── Combinator: nesting ───────────────────────────────────────────────────

    #[test]
    fn all_of_any_nested_is_true() {
        // All([ Any([exists("a"), eq("status","active")]), gt("score", 0) ])
        // data has: status=active, score=5
        // Any: eq(status,active) = true → Any = true
        // gt(score,0) = true
        // All = true
        let data = json!({"status": "active", "score": 5});
        let cond = Condition::All(vec![
            Condition::Any(vec![leaf_exists("a"), leaf_eq("status", "active")]),
            leaf_gt("score", 0),
        ]);
        assert!(evaluate_condition(&data, &cond).unwrap());
    }

    // ── Fatal propagation inside combinators ──────────────────────────────────

    // A `gt` on a missing field inside All must propagate Fatal immediately.
    // The Fatal child is placed first so All evaluates it before any short-circuit.
    // RED: if All swallowed the error and returned Ok(false), this would not unwrap_err().
    #[test]
    fn fatal_in_all_child_propagates() {
        let data = json!({}); // "score" missing → gt Fatal
        // leaf_gt("score", 0) is the first child; All evaluates it immediately → Fatal.
        let cond = Condition::All(vec![leaf_gt("score", 0), leaf_exists("anything")]);
        let err = evaluate_condition(&data, &cond).unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal from All child; got: {err:?}"
        );
    }

    #[test]
    fn fatal_in_any_child_propagates() {
        let data = json!({}); // both "a" missing (Exists→false) and "score" missing (Gt→Fatal)
        // Any evaluates "a" first → false, then "score" → Fatal
        let cond = Condition::Any(vec![leaf_exists("a"), leaf_gt("score", 0)]);
        let err = evaluate_condition(&data, &cond).unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal from Any child; got: {err:?}"
        );
    }

    #[test]
    fn not_of_fatal_propagates() {
        let data = json!({}); // "score" missing → Gt Fatal
        let cond = Condition::Not(Box::new(leaf_gt("score", 0)));
        let err = evaluate_condition(&data, &cond).unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal from Not child; got: {err:?}"
        );
    }

    // ── Serde: leaf round-trip ────────────────────────────────────────────────

    // RED: under `#[serde(untagged)]` a bad op gives "data did not match any variant";
    // with custom deserialization it gives the ConditionOp variant error containing "bogus".
    #[test]
    fn leaf_bad_op_gives_actionable_error() {
        let result = serde_json::from_str::<Condition>(r#"{"field":"x","op":"bogus","value":1}"#);
        let err = result.expect_err("must fail to deserialize unknown op");
        let msg = err.to_string();
        assert!(
            msg.contains("bogus") || msg.contains("unknown variant"),
            "error message must name the bad value or say 'unknown variant'; got: {msg}"
        );
    }

    // ── Back-compat: leaf-first dispatch ─────────────────────────────────────
    //
    // RED with combinator-first order: `{"field":"status","op":"eq","value":"active","all":"metadata"}`
    // would attempt to parse `"metadata"` as `Vec<Condition>` → Err.
    // With leaf-first dispatch the `"field"` key wins and the stray key is ignored,
    // matching the back-compat behaviour of the old derived `Deserialize`.
    #[test]
    fn flat_leaf_with_stray_combinator_key_parses_as_leaf() {
        let result = serde_json::from_str::<Condition>(
            r#"{"field":"status","op":"eq","value":"active","all":"metadata"}"#,
        );
        let cond = result.expect("must parse successfully despite stray 'all' key");
        assert_eq!(
            cond,
            Condition::Leaf {
                field: "status".into(),
                op: ConditionOp::Eq,
                value: Some(json!("active")),
            },
            "stray 'all' key must be ignored; leaf fields take precedence"
        );
        // Verify the deserialized leaf evaluates correctly.
        let data = json!({"status": "active"});
        assert!(
            evaluate_condition(&data, &cond).unwrap(),
            "leaf parsed from a doc with stray 'all' key must evaluate correctly"
        );
    }

    // A pure combinator shape (no "field" key) with a non-array "all" value
    // must produce a clean error — not a panic.
    #[test]
    fn combinator_all_with_non_array_value_errors() {
        let result = serde_json::from_str::<Condition>(r#"{"all":"nope"}"#);
        assert!(
            result.is_err(),
            r#"{{"all":"nope"}} must fail (expected Vec<Condition>, got string)"#
        );
    }

    // A real All (no "field" key) must still deserialize as Condition::All.
    #[test]
    fn real_all_without_field_key_parses_as_all() {
        let result = serde_json::from_str::<Condition>(
            r#"{"all":[{"field":"a","op":"exists"},{"field":"b","op":"exists"}]}"#,
        );
        let cond = result.expect("must parse as All");
        assert!(
            matches!(cond, Condition::All(ref cs) if cs.len() == 2),
            "must be Condition::All with 2 children; got: {cond:?}"
        );
    }

    // ── Ambiguous combinator detection ────────────────────────────────────────

    // RED with the old first-match short-circuit: `{"all":[],"any":[]}` would
    // have returned `Ok(Condition::All([]))` silently ignoring `"any"`.
    // With the ambiguity check it must return an Err whose message names the
    // problem.
    #[test]
    fn ambiguous_combinator_keys_error() {
        let result = serde_json::from_str::<Condition>(r#"{"all":[],"any":[]}"#);
        let err = result.expect_err("ambiguous combinator must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("ambiguous") || msg.contains("at most one"),
            "error must describe the ambiguity; got: {msg}"
        );
    }

    // A single `any` combinator (no `field`) must still parse as Any.
    #[test]
    fn single_any_combinator_parses_as_any() {
        let cond = serde_json::from_str::<Condition>(r#"{"any":[{"field":"flag","op":"truthy"}]}"#)
            .expect("single-combinator Any must parse");
        assert!(
            matches!(cond, Condition::Any(ref cs) if cs.len() == 1),
            "must be Condition::Any with 1 child; got: {cond:?}"
        );
    }

    // A single `not` combinator (no `field`) must still parse as Not.
    #[test]
    fn single_not_combinator_parses_as_not() {
        let cond =
            serde_json::from_str::<Condition>(r#"{"not":{"field":"archived","op":"truthy"}}"#)
                .expect("single-combinator Not must parse");
        assert!(
            matches!(cond, Condition::Not(_)),
            "must be Condition::Not; got: {cond:?}"
        );
    }

    // Back-compat: a leaf WITH a `field` key is never treated as ambiguous,
    // even if it also carries combinator keys — the `field` key wins.
    #[test]
    fn leaf_with_extras_is_not_ambiguous() {
        // This already has a test (`flat_leaf_with_stray_combinator_key_parses_as_leaf`)
        // but confirm it still holds after the ambiguity check was added.
        let cond = serde_json::from_str::<Condition>(
            r#"{"field":"status","op":"eq","value":"active","all":"metadata","any":"x"}"#,
        )
        .expect("leaf with stray combinator keys must parse as leaf (back-compat)");
        assert!(
            matches!(cond, Condition::Leaf { .. }),
            "must be Condition::Leaf; got: {cond:?}"
        );
    }

    #[test]
    fn leaf_serde_roundtrip() {
        let cond = Condition::Leaf {
            field: "status".into(),
            op: ConditionOp::Eq,
            value: Some(json!("active")),
        };
        let serialized = serde_json::to_value(&cond).unwrap();
        // Must serialize to the FLAT form — no wrapper key.
        assert_eq!(
            serialized,
            json!({"field": "status", "op": "eq", "value": "active"}),
            "Leaf must serialize to flat object without wrapper key"
        );
        let restored: Condition = serde_json::from_value(serialized).unwrap();
        assert_eq!(restored, cond, "Leaf must survive a serde round-trip");
    }

    #[test]
    fn leaf_no_value_serde_roundtrip() {
        let cond = Condition::Leaf {
            field: "key".into(),
            op: ConditionOp::Exists,
            value: None,
        };
        let serialized = serde_json::to_value(&cond).unwrap();
        // `value` must be absent when None.
        assert_eq!(
            serialized,
            json!({"field": "key", "op": "exists"}),
            "Leaf with None value must serialize without 'value' key"
        );
        let restored: Condition = serde_json::from_value(serialized).unwrap();
        assert_eq!(
            restored, cond,
            "Leaf (no value) must survive a serde round-trip"
        );
    }

    #[test]
    fn all_serde_roundtrip() {
        let cond = Condition::All(vec![leaf_exists("a"), leaf_exists("b")]);
        let serialized = serde_json::to_value(&cond).unwrap();
        assert_eq!(serialized["all"].as_array().unwrap().len(), 2);
        let restored: Condition = serde_json::from_value(serialized).unwrap();
        assert_eq!(restored, cond);
    }

    #[test]
    fn any_serde_roundtrip() {
        let cond = Condition::Any(vec![leaf_eq("status", "active")]);
        let serialized = serde_json::to_value(&cond).unwrap();
        assert!(serialized.get("any").is_some(), "must have 'any' key");
        let restored: Condition = serde_json::from_value(serialized).unwrap();
        assert_eq!(restored, cond);
    }

    #[test]
    fn not_serde_roundtrip() {
        let cond = Condition::Not(Box::new(leaf_eq("archived", "true")));
        let serialized = serde_json::to_value(&cond).unwrap();
        assert!(serialized.get("not").is_some(), "must have 'not' key");
        let restored: Condition = serde_json::from_value(serialized).unwrap();
        assert_eq!(restored, cond);
    }
}
