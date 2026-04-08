//! Lazy-parsed output: Arc<RawValue> + cached Value.

use std::sync::Arc;
use std::sync::OnceLock;

use serde_json::value::RawValue;
use serde_json::Value;

/// Lazy-parsed output via Box<RawValue>, cached after first parse.
#[derive(Debug, Clone)]
pub struct NodeOutput {
    raw: Arc<Box<RawValue>>,
    parsed: OnceLock<Value>,
}

impl NodeOutput {
    /// Create from `Value`.
    pub fn from_value(value: &Value) -> Self {
        let json_str = serde_json::to_string(value)
            .expect("Value serialization to JSON should not fail");
        let raw_box = RawValue::from_string(json_str)
            .expect("RawValue construction should not fail");
        Self {
            raw: Arc::new(raw_box),
            parsed: OnceLock::new(),
        }
    }

    /// Raw JSON bytes.
    #[must_use]
    pub fn as_raw(&self) -> &RawValue {
        &self.raw
    }

    /// Parsed `Value` (cached).
    #[must_use]
    pub fn as_value(&self) -> &Value {
        self.parsed.get_or_init(|| {
            serde_json::from_str(self.raw.get())
                .expect("RawValue deserialization should not fail")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip() {
        let v = json!({"name": "test", "count": 42});
        let output = NodeOutput::from_value(&v);
        assert_eq!(output.as_value(), &v);
    }

    #[test]
    fn raw_access() {
        let v = json!({"key": "value"});
        let output = NodeOutput::from_value(&v);
        assert!(output.as_raw().get().contains("key"));
    }

    #[test]
    fn cached() {
        let v = json!({"x": 1});
        let output = NodeOutput::from_value(&v);
        let v1 = output.as_value();
        let v2 = output.as_value();
        assert!(std::ptr::eq(v1, v2));
    }
}
