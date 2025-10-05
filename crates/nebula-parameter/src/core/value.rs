use crate::types::{ExpirableValue, ListValue, ModeValue, ObjectValue, RoutingValue};
use nebula_value::Value;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParameterValue {
    Value(nebula_value::Value),
    Expression(String),
    Routing(RoutingValue),
    Mode(ModeValue),
    Expirable(ExpirableValue),
    List(ListValue),
    Object(ObjectValue),
}

impl ParameterValue {
    /// Get the underlying Value if this is not an expression
    pub fn as_value(&self) -> Option<&Value> {
        match self {
            ParameterValue::Value(v) => Some(v),
            ParameterValue::Expression(_) => None,
            ParameterValue::Routing(_) => None,
            ParameterValue::Mode(_) => None,
            ParameterValue::Expirable(exp_val) => Some(&exp_val.value),
            ParameterValue::List(_) => None,
            ParameterValue::Object(_) => None,
        }
    }

    /// Check if this is an expression
    pub fn is_expression(&self) -> bool {
        matches!(self, ParameterValue::Expression(_))
    }

    /// Check if this is a routing value
    pub fn is_routing(&self) -> bool {
        matches!(self, ParameterValue::Routing(_))
    }

    /// Check if this is a mode value
    pub fn is_mode(&self) -> bool {
        matches!(self, ParameterValue::Mode(_))
    }

    /// Check if this is an expirable value
    pub fn is_expirable(&self) -> bool {
        matches!(self, ParameterValue::Expirable(_))
    }

    /// Check if this is a list value
    pub fn is_list(&self) -> bool {
        matches!(self, ParameterValue::List(_))
    }

    /// Check if this is an object value
    pub fn is_object(&self) -> bool {
        matches!(self, ParameterValue::Object(_))
    }

    /// Check if this parameter value is considered "empty"
    pub fn is_empty(&self) -> bool {
        match self {
            ParameterValue::Value(value) => match value {
                Value::Null => true,
                Value::Text(s) => s.is_empty(),
                Value::Array(a) => a.is_empty(),
                Value::Object(o) => o.is_empty(),
                _ => false,
            },
            ParameterValue::Expression(expr) => expr.is_empty(),
            ParameterValue::Routing(_) => false, // Routing values are never considered empty
            ParameterValue::Mode(mode_val) => {
                // Check if mode value is empty
                match &mode_val.value {
                    Value::Null => true,
                    Value::Text(s) => s.as_str().trim().is_empty(),
                    Value::Array(a) => a.is_empty(),
                    Value::Object(o) => o.is_empty(),
                    _ => false,
                }
            }
            ParameterValue::Expirable(exp_val) => {
                exp_val.is_expired()
                    || match &exp_val.value {
                        Value::Text(s) => s.as_str().trim().is_empty(),
                        Value::Null => true,
                        Value::Array(a) => a.is_empty(),
                        Value::Object(o) => o.is_empty(),
                        _ => false,
                    }
            }
            ParameterValue::List(list_val) => list_val.is_empty(),
            ParameterValue::Object(obj_val) => obj_val.is_empty(),
        }
    }
}

impl From<Value> for ParameterValue {
    fn from(value: Value) -> Self {
        ParameterValue::Value(value)
    }
}

impl From<String> for ParameterValue {
    fn from(expr: String) -> Self {
        ParameterValue::Expression(expr)
    }
}

impl From<&str> for ParameterValue {
    fn from(expr: &str) -> Self {
        ParameterValue::Expression(expr.to_string())
    }
}

// Removed: impl From<serde_json::Value> for ParameterValue
// Use nebula_value::JsonValueExt trait instead:
// use nebula_value::JsonValueExt;
// let param_val = json_value.to_nebula_value_or_null().into();

impl From<ParameterValue> for Value {
    fn from(param_value: ParameterValue) -> Self {
        match param_value {
            ParameterValue::Value(v) => v,
            ParameterValue::Expression(expr) => Value::text(expr),
            ParameterValue::Routing(_) => Value::text("routing_value"),
            ParameterValue::Mode(mode_val) => mode_val.value.clone(),
            ParameterValue::Expirable(exp_val) => {
                if exp_val.is_expired() {
                    Value::Null
                } else {
                    exp_val.value.clone()
                }
            }
            ParameterValue::List(list_val) => {
                // Convert Vec<nebula_value::Value> to Array using from_nebula_values
                Value::Array(nebula_value::Array::from_nebula_values(list_val.items.clone()))
            }
            ParameterValue::Object(obj_val) => {
                // Object uses serde_json::Value internally, construct from iterator
                Value::Object(obj_val.values.clone().into_iter().collect())
            }
        }
    }
}

impl From<RoutingValue> for ParameterValue {
    fn from(routing_value: RoutingValue) -> Self {
        ParameterValue::Routing(routing_value)
    }
}

impl From<ModeValue> for ParameterValue {
    fn from(mode_value: ModeValue) -> Self {
        ParameterValue::Mode(mode_value)
    }
}

impl From<ExpirableValue> for ParameterValue {
    fn from(expirable_value: ExpirableValue) -> Self {
        ParameterValue::Expirable(expirable_value)
    }
}

impl From<ListValue> for ParameterValue {
    fn from(list_value: ListValue) -> Self {
        ParameterValue::List(list_value)
    }
}

impl From<ObjectValue> for ParameterValue {
    fn from(object_value: ObjectValue) -> Self {
        ParameterValue::Object(object_value)
    }
}

// Convenient Into implementations for common types
impl From<bool> for ParameterValue {
    fn from(b: bool) -> Self {
        ParameterValue::Value(Value::boolean(b))
    }
}

impl From<i64> for ParameterValue {
    fn from(i: i64) -> Self {
        ParameterValue::Value(Value::integer(i))
    }
}

impl From<i32> for ParameterValue {
    fn from(i: i32) -> Self {
        ParameterValue::Value(Value::integer(i as i64))
    }
}

impl From<f64> for ParameterValue {
    fn from(f: f64) -> Self {
        ParameterValue::Value(Value::float(f))
    }
}

impl From<f32> for ParameterValue {
    fn from(f: f32) -> Self {
        ParameterValue::Value(Value::float(f as f64))
    }
}

// nebula_value scalar types
impl From<nebula_value::Text> for ParameterValue {
    fn from(t: nebula_value::Text) -> Self {
        ParameterValue::Value(Value::Text(t))
    }
}

impl From<nebula_value::Integer> for ParameterValue {
    fn from(i: nebula_value::Integer) -> Self {
        ParameterValue::Value(Value::Integer(i))
    }
}

impl From<nebula_value::Float> for ParameterValue {
    fn from(f: nebula_value::Float) -> Self {
        ParameterValue::Value(Value::Float(f))
    }
}

impl From<nebula_value::Bytes> for ParameterValue {
    fn from(b: nebula_value::Bytes) -> Self {
        ParameterValue::Value(Value::Bytes(b))
    }
}

impl From<nebula_value::Array> for ParameterValue {
    fn from(a: nebula_value::Array) -> Self {
        ParameterValue::Value(Value::Array(a))
    }
}

impl From<nebula_value::Object> for ParameterValue {
    fn from(o: nebula_value::Object) -> Self {
        ParameterValue::Value(Value::Object(o))
    }
}

// Note: Conversion functions removed - use nebula_value::JsonValueExt trait instead
// Import with: use nebula_value::JsonValueExt;
