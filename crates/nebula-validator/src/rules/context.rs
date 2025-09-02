//! Rule execution context

use serde::{Serialize, Deserialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Value types that can be stored in context
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContextValue {
    /// Null value
    Null,
    /// Boolean value
    Bool(bool),
    /// Integer value
    Integer(i64),
    /// Float value
    Float(f64),
    /// String value
    String(String),
    /// JSON value
    Json(Value),
    /// Array of context values
    Array(Vec<ContextValue>),
    /// Map of context values
    Map(HashMap<String, ContextValue>),
}

impl From<bool> for ContextValue {
    fn from(v: bool) -> Self {
        ContextValue::Bool(v)
    }
}

impl From<i64> for ContextValue {
    fn from(v: i64) -> Self {
        ContextValue::Integer(v)
    }
}

impl From<f64> for ContextValue {
    fn from(v: f64) -> Self {
        ContextValue::Float(v)
    }
}

impl From<String> for ContextValue {
    fn from(v: String) -> Self {
        ContextValue::String(v)
    }
}

impl From<&str> for ContextValue {
    fn from(v: &str) -> Self {
        ContextValue::String(v.to_string())
    }
}

impl From<Value> for ContextValue {
    fn from(v: Value) -> Self {
        ContextValue::Json(v)
    }
}

/// Rule execution context
#[derive(Debug, Clone, Default)]
pub struct RuleContext {
    /// Context values
    values: Arc<HashMap<String, ContextValue>>,
    /// Parent context (for nested contexts)
    parent: Option<Box<RuleContext>>,
}

impl RuleContext {
    /// Create a new empty context
    pub fn new() -> Self {
        Self {
            values: Arc::new(HashMap::new()),
            parent: None,
        }
    }
    
    /// Create a builder
    pub fn builder() -> ContextBuilder {
        ContextBuilder::new()
    }
    
    /// Create a child context
    pub fn child(&self) -> Self {
        Self {
            values: Arc::new(HashMap::new()),
            parent: Some(Box::new(self.clone())),
        }
    }
    
    /// Set a value in the context
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<ContextValue>) {
        Arc::make_mut(&mut self.values).insert(key.into(), value.into());
    }
    
    /// Get a value from the context
    pub fn get(&self, key: &str) -> Option<&ContextValue> {
        self.values.get(key).or_else(|| {
            self.parent.as_ref().and_then(|p| p.get(key))
        })
    }
    
    /// Get a typed value
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            ContextValue::Bool(v) => Some(*v),
            _ => None,
        }
    }
    
    /// Get an integer value
    pub fn get_integer(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            ContextValue::Integer(v) => Some(*v),
            _ => None,
        }
    }
    
    /// Get a float value
    pub fn get_float(&self, key: &str) -> Option<f64> {
        match self.get(key)? {
            ContextValue::Float(v) => Some(*v),
            ContextValue::Integer(v) => Some(*v as f64),
            _ => None,
        }
    }
    
    /// Get a string value
    pub fn get_string(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            ContextValue::String(v) => Some(v.as_str()),
            _ => None,
        }
    }
    
    /// Get a JSON value
    pub fn get_json(&self, key: &str) -> Option<&Value> {
        match self.get(key)? {
            ContextValue::Json(v) => Some(v),
            _ => None,
        }
    }
    
    /// Check if a key exists
    pub fn contains_key(&self, key: &str) -> bool {
        self.values.contains_key(key) || 
            self.parent.as_ref().map_or(false, |p| p.contains_key(key))
    }
    
    /// Remove a value
    pub fn remove(&mut self, key: &str) -> Option<ContextValue> {
        Arc::make_mut(&mut self.values).remove(key)
    }
    
    /// Clear all values (keeps parent)
    pub fn clear(&mut self) {
        Arc::make_mut(&mut self.values).clear();
    }
    
    /// Get all keys
    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<_> = self.values.keys().cloned().collect();
        if let Some(ref parent) = self.parent {
            keys.extend(parent.keys());
        }
        keys.sort();
        keys.dedup();
        keys
    }
    
    /// Merge another context into this one
    pub fn merge(&mut self, other: &RuleContext) {
        let values = Arc::make_mut(&mut self.values);
        for (key, value) in other.values.iter() {
            values.insert(key.clone(), value.clone());
        }
    }
    
    /// Convert to JSON value
    pub fn to_json(&self) -> Value {
        let mut map = serde_json::Map::new();
        
        // Add parent values first
        if let Some(ref parent) = self.parent {
            if let Value::Object(parent_map) = parent.to_json() {
                map.extend(parent_map);
            }
        }
        
        // Add current values (override parent)
        for (key, value) in self.values.iter() {
            map.insert(key.clone(), self.context_value_to_json(value));
        }
        
        Value::Object(map)
    }
    
    fn context_value_to_json(&self, value: &ContextValue) -> Value {
        match value {
            ContextValue::Null => Value::Null,
            ContextValue::Bool(v) => Value::Bool(*v),
            ContextValue::Integer(v) => Value::Number((*v).into()),
            ContextValue::Float(v) => Value::Number(
                serde_json::Number::from_f64(*v).unwrap_or_else(|| 0.into())
            ),
            ContextValue::String(v) => Value::String(v.clone()),
            ContextValue::Json(v) => v.clone(),
            ContextValue::Array(v) => {
                Value::Array(v.iter().map(|cv| self.context_value_to_json(cv)).collect())
            },
            ContextValue::Map(v) => {
                let map: serde_json::Map<_, _> = v.iter()
                    .map(|(k, cv)| (k.clone(), self.context_value_to_json(cv)))
                    .collect();
                Value::Object(map)
            },
        }
    }
}

/// Builder for RuleContext
pub struct ContextBuilder {
    values: HashMap<String, ContextValue>,
    parent: Option<RuleContext>,
}

impl ContextBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
            parent: None,
        }
    }
    
    /// Set parent context
    pub fn parent(mut self, parent: RuleContext) -> Self {
        self.parent = Some(parent);
        self
    }
    
    /// Add a value
    pub fn value(mut self, key: impl Into<String>, value: impl Into<ContextValue>) -> Self {
        self.values.insert(key.into(), value.into());
        self
    }
    
    /// Add multiple values from a map
    pub fn values<I, K, V>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<ContextValue>,
    {
        for (key, value) in values {
            self.values.insert(key.into(), value.into());
        }
        self
    }
    
    /// Add values from JSON
    pub fn from_json(mut self, json: Value) -> Self {
        if let Value::Object(map) = json {
            for (key, value) in map {
                self.values.insert(key, ContextValue::Json(value));
            }
        }
        self
    }
    
    /// Build the context
    pub fn build(self) -> RuleContext {
        let mut context = RuleContext {
            values: Arc::new(self.values),
            parent: self.parent.map(Box::new),
        };
        context
    }
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}