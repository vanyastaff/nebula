
use serde::{Deserialize, Serialize};
use nebula_value::Value;
use crate::types::{RoutingValue, ModeValue, ExpirableValue, ObjectValue};

/// Value for list parameters containing array of child parameter values
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ListValue {
    /// Array of values from child parameters
    pub items: Vec<nebula_value::Value>,
}

impl ListValue {
    /// Create a new ListValue
    pub fn new(items: Vec<nebula_value::Value>) -> Self {
        Self { items }
    }

    /// Create an empty ListValue
    pub fn empty() -> Self {
        Self { items: Vec::new() }
    }

    /// Add an item to the list
    pub fn push(&mut self, item: nebula_value::Value) {
        self.items.push(item);
    }

    /// Get item count
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Check if the list is empty
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

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
                Value::String(s) => s.is_empty(),
                Value::Array(a) => a.is_empty(),
                Value::Object(o) => o.is_empty(),
                _ => false,
            },
            ParameterValue::Expression(expr) => expr.is_empty(),
            ParameterValue::Routing(_) => false, // Routing values are never considered empty
            ParameterValue::Mode(mode_val) => mode_val.value.is_empty(),
            ParameterValue::Expirable(exp_val) => {
                exp_val.is_expired() || match &exp_val.value {
                    Value::String(s) => s.as_str().trim().is_empty(),
                    Value::Null => true,
                    Value::Array(a) => a.is_empty(),
                    Value::Object(o) => o.is_empty(),
                    _ => false,
                }
            },
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

impl From<serde_json::Value> for ParameterValue {
    fn from(json_value: serde_json::Value) -> Self {
        let nebula_value = match json_value {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(b.into()),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i.into())
                } else if let Some(f) = n.as_f64() {
                    Value::Float(f.into())
                } else {
                    Value::Null
                }
            },
            serde_json::Value::String(s) => Value::String(s.into()),
            serde_json::Value::Array(arr) => {
                let nebula_arr: Vec<Value> = arr.into_iter()
                    .map(|v| ParameterValue::from(v).into())
                    .filter_map(|pv| match pv {
                        ParameterValue::Value(v) => Some(v),
                        _ => None,
                    })
                    .collect();
                Value::Array(nebula_arr.into())
            },
            serde_json::Value::Object(obj) => {
                let nebula_obj: Vec<(String, Value)> = obj.into_iter()
                    .filter_map(|(k, v)| {
                        if let ParameterValue::Value(nv) = ParameterValue::from(v) {
                            Some((k, nv))
                        } else {
                            None
                        }
                    })
                    .collect();
                Value::Object(nebula_obj.into())
            }
        };
        ParameterValue::Value(nebula_value)
    }
}

impl From<ParameterValue> for Value {
    fn from(param_value: ParameterValue) -> Self {
        match param_value {
            ParameterValue::Value(v) => v,
            ParameterValue::Expression(expr) => Value::String(expr.into()),
            ParameterValue::Routing(_) => Value::String("routing_value".into()),
            ParameterValue::Mode(mode_val) => mode_val.value.clone(),
            ParameterValue::Expirable(exp_val) => {
                if exp_val.is_expired() {
                    Value::Null
                } else {
                    exp_val.value.clone()
                }
            },
            ParameterValue::List(list_val) => {
                Value::Array(list_val.items.clone().into())
            },
            ParameterValue::Object(obj_val) => {
                let obj_map: Vec<(String, Value)> = obj_val.values.iter()
                    .map(|(k, v)| (k.clone(), convert_json_to_nebula_value(v)))
                    .collect();
                Value::Object(obj_map.into())
            },
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

// Helper function to convert serde_json::Value to nebula_value::Value
fn convert_json_to_nebula_value(json_value: &serde_json::Value) -> nebula_value::Value {
    match json_value {
        serde_json::Value::Null => nebula_value::Value::Null,
        serde_json::Value::Bool(b) => nebula_value::Value::Bool((*b).into()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                nebula_value::Value::Int(i.into())
            } else if let Some(f) = n.as_f64() {
                nebula_value::Value::Float(f.into())
            } else {
                nebula_value::Value::Null
            }
        },
        serde_json::Value::String(s) => nebula_value::Value::String(s.clone().into()),
        serde_json::Value::Array(arr) => {
            let nebula_arr: Vec<nebula_value::Value> = arr.iter()
                .map(convert_json_to_nebula_value)
                .collect();
            nebula_value::Value::Array(nebula_arr.into())
        },
        serde_json::Value::Object(obj) => {
            let nebula_obj: Vec<(String, nebula_value::Value)> = obj.iter()
                .map(|(k, v)| (k.clone(), convert_json_to_nebula_value(v)))
                .collect();
            nebula_value::Value::Object(nebula_obj.into())
        }
    }
}
