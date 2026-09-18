//! Runtime values for expression evaluation.
//!
//! `RuntimeValue` is the evaluator's working value type. It mirrors JSON for
//! every scalar and container, and adds values JSON cannot represent:
//!
//! - [`RuntimeValue::DateTime`] — an absolute instant with a fixed offset, so
//!   date methods and arithmetic work on values instead of on formatted strings.
//! - [`RuntimeValue::Undefined`] — the result of a lookup that found nothing,
//!   when [`EvaluationPolicy::with_missing_lookup`](crate::EvaluationPolicy)
//!   selects the non-erroring mode. It is deliberately distinct from
//!   `Null`: "missing is not null" is a crate invariant.
//!
//! Typed values survive inside containers: a `DateTime` stored in an array is
//! still a `DateTime` when it is read back out. Conversion to plain JSON
//! ([`RuntimeValue::to_json`]) happens at the crate boundary only — the Canon
//! §3.5 resolve step receives `serde_json::Value`.

use std::{collections::BTreeMap, fmt, sync::Arc};

use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use serde_json::{Map, Number, Value};

/// A value produced or consumed by expression evaluation.
///
/// Cloning is cheap: strings, arrays, objects, and shared values are `Arc`-backed.
#[derive(Clone)]
pub enum RuntimeValue {
    /// JSON null.
    Null,
    /// A lookup that found nothing, in a policy that permits it.
    ///
    /// Never equal to [`RuntimeValue::Null`]; see the module docs.
    Undefined,
    /// JSON boolean.
    Bool(bool),
    /// JSON integer in the signed range.
    Integer(i64),
    /// JSON integer above [`i64::MAX`], up to [`u64::MAX`].
    Unsigned(u64),
    /// IEEE-754 double. Always finite: non-finite results are rejected at
    /// construction and never reach a value.
    Float(f64),
    /// UTF-8 string.
    String(Arc<str>),
    /// JSON array.
    Array(Arc<[RuntimeValue]>),
    /// JSON object.
    Object(Arc<BTreeMap<Arc<str>, RuntimeValue>>),
    /// An absolute instant with a fixed UTC offset.
    DateTime(DateTime<FixedOffset>),
}

impl RuntimeValue {
    /// Construct a string value.
    pub fn string(value: impl Into<Arc<str>>) -> Self {
        Self::String(value.into())
    }

    /// Construct an array value from owned elements.
    pub fn array(values: impl Into<Arc<[RuntimeValue]>>) -> Self {
        Self::Array(values.into())
    }

    /// Construct an object value from owned entries.
    pub fn object(values: BTreeMap<Arc<str>, RuntimeValue>) -> Self {
        Self::Object(Arc::new(values))
    }

    /// Construct a date-time value from a UTC instant.
    pub fn date_time_utc(value: DateTime<Utc>) -> Self {
        Self::DateTime(value.fixed_offset())
    }

    /// The name used in type errors and diagnostics.
    ///
    /// Numbers report `"number"` regardless of their internal representation,
    /// matching the expression language's single numeric type.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Undefined => "undefined",
            Self::Bool(_) => "boolean",
            Self::Integer(_) | Self::Unsigned(_) | Self::Float(_) => "number",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
            Self::DateTime(_) => "date",
        }
    }

    /// Whether this value is a number of any internal representation.
    pub fn is_number(&self) -> bool {
        matches!(self, Self::Integer(_) | Self::Unsigned(_) | Self::Float(_))
    }

    /// Whether this value is [`RuntimeValue::Null`] or [`RuntimeValue::Undefined`].
    pub fn is_nullish(&self) -> bool {
        matches!(self, Self::Null | Self::Undefined)
    }

    /// Borrow this value as a string, if it is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    /// Borrow this value as an array, if it is one.
    pub fn as_array(&self) -> Option<&[RuntimeValue]> {
        match self {
            Self::Array(values) => Some(values),
            _ => None,
        }
    }

    /// Borrow this value as an object, if it is one.
    pub fn as_object(&self) -> Option<&BTreeMap<Arc<str>, RuntimeValue>> {
        match self {
            Self::Object(entries) => Some(entries),
            _ => None,
        }
    }

    /// This value as a boolean, if it is one.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// This value as an exact `i64`, accepting integral floats only within range.
    ///
    /// Unsigned values above [`i64::MAX`] return `None`.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            Self::Unsigned(value) => i64::try_from(*value).ok(),
            Self::Float(value) => exact_i64(*value),
            _ => None,
        }
    }

    /// This value as an exact `u64`, accepting integral floats only within range.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Integer(value) => u64::try_from(*value).ok(),
            Self::Unsigned(value) => Some(*value),
            Self::Float(value) => exact_u64(*value),
            _ => None,
        }
    }

    /// This value as an `i128`, for exact integer arithmetic that may overflow
    /// the JSON integer range before it is narrowed again.
    pub fn as_i128(&self) -> Option<i128> {
        match self {
            Self::Integer(value) => Some(i128::from(*value)),
            Self::Unsigned(value) => Some(i128::from(*value)),
            _ => None,
        }
    }

    /// This value as an `f64`. Every number representation converts; `None`
    /// only for non-numbers.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Integer(value) => Some(*value as f64),
            Self::Unsigned(value) => Some(*value as f64),
            Self::Float(value) => Some(*value),
            _ => None,
        }
    }

    /// This value as a date-time, if it is one.
    pub fn as_date_time(&self) -> Option<&DateTime<FixedOffset>> {
        match self {
            Self::DateTime(value) => Some(value),
            _ => None,
        }
    }

    /// Convert to plain JSON for the crate boundary.
    ///
    /// Date-times become RFC 3339 strings (always with an offset, seconds
    /// preserved). `Undefined` becomes `Null` — by the time a value reaches
    /// the boundary, a policy has already decided how missing lookups behave.
    pub fn to_json(&self) -> Value {
        match self {
            Self::Null | Self::Undefined => Value::Null,
            Self::Bool(value) => Value::Bool(*value),
            Self::Integer(value) => Value::Number((*value).into()),
            Self::Unsigned(value) => Value::Number((*value).into()),
            Self::Float(value) => Number::from_f64(*value)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            Self::String(text) => Value::String(text.to_string()),
            Self::Array(values) => Value::Array(values.iter().map(RuntimeValue::to_json).collect()),
            Self::Object(entries) => {
                let mut object = Map::new();
                for (key, value) in entries.iter() {
                    object.insert(key.to_string(), value.to_json());
                }
                Value::Object(object)
            },
            Self::DateTime(value) => {
                Value::String(value.to_rfc3339_opts(SecondsFormat::AutoSi, true))
            },
        }
    }

    /// Convert from plain JSON. Every JSON value maps to exactly one runtime
    /// value; dates stay strings until a datetime builtin parses them.
    pub fn from_json(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(value) => Self::Bool(*value),
            Value::Number(number) => Self::from_number(number),
            Value::String(text) => Self::String(Arc::from(text.as_str())),
            Value::Array(values) => Self::Array(
                values
                    .iter()
                    .map(Self::from_json)
                    .collect::<Vec<_>>()
                    .into(),
            ),
            Value::Object(entries) => {
                let mut object = BTreeMap::new();
                for (key, value) in entries {
                    object.insert(Arc::from(key.as_str()), Self::from_json(value));
                }
                Self::Object(Arc::new(object))
            },
        }
    }

    /// Convert a JSON number, preserving its signed/unsigned/float representation.
    pub fn from_number(number: &Number) -> Self {
        if let Some(value) = number.as_i64() {
            Self::Integer(value)
        } else if let Some(value) = number.as_u64() {
            Self::Unsigned(value)
        } else if let Some(value) = number.as_f64() {
            Self::Float(value)
        } else {
            // A `Number` always exposes at least one of the three views.
            Self::Null
        }
    }

    /// The string a template renders for this value.
    ///
    /// Strings render as their contents; every other value renders as its JSON
    /// form, except `Undefined`, which renders as the empty string (a missing
    /// value interpolates to nothing rather than the literal text `null`).
    pub fn to_display_string(&self) -> String {
        match self {
            Self::String(text) => text.to_string(),
            Self::Undefined => String::new(),
            other => other.to_json().to_string(),
        }
    }
}

/// Narrow an `f64` to `i64` only when it is finite, integral, and in range.
fn exact_i64(value: f64) -> Option<i64> {
    (value.is_finite()
        && value.fract() == 0.0
        && value >= -(2_f64.powi(63))
        && value < 2_f64.powi(63))
    .then_some(value as i64)
}

/// Narrow an `f64` to `u64` only when it is finite, integral, and in range.
fn exact_u64(value: f64) -> Option<u64> {
    (value.is_finite() && value.fract() == 0.0 && value >= 0.0 && value < 2_f64.powi(64))
        .then_some(value as u64)
}

/// Structural equality over runtime representations.
///
/// Numeric variants compare exactly across representations, matching the
/// evaluator's `==` operator: `Integer(1) == Float(1.0)`, and
/// `Unsigned(2^63) != Integer(i64::MIN)`.
impl PartialEq for RuntimeValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) | (Self::Undefined, Self::Undefined) => true,
            (Self::Bool(left), Self::Bool(right)) => left == right,
            (Self::String(left), Self::String(right)) => left == right,
            (Self::Array(left), Self::Array(right)) => left == right,
            (Self::Object(left), Self::Object(right)) => left == right,
            (Self::DateTime(left), Self::DateTime(right)) => left == right,
            (
                Self::Integer(_) | Self::Unsigned(_) | Self::Float(_),
                Self::Integer(_) | Self::Unsigned(_) | Self::Float(_),
            ) => {
                crate::value_utils::compare_numbers(self, other) == Some(std::cmp::Ordering::Equal)
            },
            _ => false,
        }
    }
}

impl fmt::Debug for RuntimeValue {
    /// Structured debug output that redacts string and container payloads.
    ///
    /// Values can carry credential material; `Debug` must never print it.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => formatter.write_str("Null"),
            Self::Undefined => formatter.write_str("Undefined"),
            Self::Bool(value) => formatter.debug_tuple("Bool").field(value).finish(),
            Self::Integer(value) => formatter.debug_tuple("Integer").field(value).finish(),
            Self::Unsigned(value) => formatter.debug_tuple("Unsigned").field(value).finish(),
            Self::Float(value) => formatter.debug_tuple("Float").field(value).finish(),
            Self::String(text) => formatter
                .debug_struct("String")
                .field("bytes", &text.len())
                .finish_non_exhaustive(),
            Self::Array(values) => formatter
                .debug_struct("Array")
                .field("len", &values.len())
                .finish_non_exhaustive(),
            Self::Object(entries) => formatter
                .debug_struct("Object")
                .field("len", &entries.len())
                .finish_non_exhaustive(),
            Self::DateTime(value) => formatter
                .debug_tuple("DateTime")
                .field(&value.to_rfc3339_opts(SecondsFormat::Secs, true))
                .finish(),
        }
    }
}

impl From<bool> for RuntimeValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for RuntimeValue {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

impl From<u64> for RuntimeValue {
    fn from(value: u64) -> Self {
        Self::Unsigned(value)
    }
}

impl From<f64> for RuntimeValue {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<&str> for RuntimeValue {
    fn from(value: &str) -> Self {
        Self::String(Arc::from(value))
    }
}

impl From<String> for RuntimeValue {
    fn from(value: String) -> Self {
        Self::String(Arc::from(value))
    }
}

impl From<Arc<str>> for RuntimeValue {
    fn from(value: Arc<str>) -> Self {
        Self::String(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip_preserves_every_scalar_shape() {
        for value in [
            serde_json::json!(null),
            serde_json::json!(true),
            serde_json::json!(0),
            serde_json::json!(-1),
            serde_json::json!(u64::MAX),
            serde_json::json!(1.5),
            serde_json::json!("text"),
            serde_json::json!([1, {"a": null}]),
        ] {
            let runtime = RuntimeValue::from_json(&value);
            assert_eq!(runtime.to_json(), value, "{value}");
        }
    }

    #[test]
    fn undefined_is_not_null_and_renders_empty() {
        assert_ne!(RuntimeValue::Undefined, RuntimeValue::Null);
        assert_eq!(RuntimeValue::Undefined.to_display_string(), "");
        assert_eq!(RuntimeValue::Null.to_display_string(), "null");
        assert_eq!(RuntimeValue::Undefined.to_json(), Value::Null);
    }

    #[test]
    fn integer_widths_convert_exactly_across_representations() {
        assert_eq!(RuntimeValue::Integer(-5).as_i64(), Some(-5));
        assert_eq!(RuntimeValue::Unsigned(u64::MAX).as_i64(), None);
        assert!(RuntimeValue::Unsigned(u64::MAX).as_f64().is_some());
        assert_eq!(RuntimeValue::Float(2.0).as_i64(), Some(2));
        assert_eq!(RuntimeValue::Float(2.5).as_i64(), None);
        assert_eq!(RuntimeValue::Float(f64::NAN).as_i64(), None);
    }

    #[test]
    fn date_time_survives_inside_containers() {
        let instant = DateTime::parse_from_rfc3339("2024-03-01T10:00:00+03:00").unwrap();
        let value = RuntimeValue::array(vec![RuntimeValue::DateTime(instant)]);
        let RuntimeValue::Array(items) = &value else {
            panic!("expected array");
        };
        let restored = items[0].as_date_time().expect("date survives nesting");
        assert_eq!(restored.to_rfc3339(), "2024-03-01T10:00:00+03:00");
        assert_eq!(
            value.to_json()[0],
            serde_json::json!("2024-03-01T10:00:00+03:00")
        );
    }

    #[test]
    fn debug_redacts_payloads() {
        const CANARY: &str = "RUNTIME_VALUE_SECRET_CANARY";
        let value = RuntimeValue::Object(Arc::new(BTreeMap::from([(
            Arc::from("key"),
            RuntimeValue::String(Arc::from(CANARY)),
        )])));
        let diagnostic = format!("{value:?}");
        assert!(!diagnostic.contains(CANARY), "leaked payload: {diagnostic}");
        assert!(diagnostic.contains("len"));
    }
}
