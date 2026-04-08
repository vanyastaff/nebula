//! Lazy-parsed node output with zero-copy sharing and cached Value.

use std::sync::Arc;
use std::sync::OnceLock;

use serde_json::Value;
use serde_json::value::RawValue;

/// Lazy-parsed node output — raw JSON shared via Arc, parsed on demand.
#[derive(Debug, Clone)]
pub struct NodeOutput {
    raw: Arc<Box<RawValue>>,
    parsed: OnceLock<Value>,
}

impl NodeOutput {
    /// Create from `Value`.
    pub fn from_value(value: &Value) -> Self {
        // Reason: serde_json::Value is always serializable to a JSON string.
        let json_str =
            serde_json::to_string(value).expect("serde_json::Value is always valid JSON");
        // Reason: the string was just produced by serde_json — always valid.
        let raw_box = RawValue::from_string(json_str)
            .expect("string from serde_json::to_string is valid JSON");
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
            // Reason: RawValue guarantees its content is valid JSON.
            serde_json::from_str(self.raw.get()).expect("RawValue content is guaranteed valid JSON")
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
