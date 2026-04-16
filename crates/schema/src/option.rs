use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Static option for select fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectOption {
    /// Stored value.
    pub value: Value,
    /// Display label.
    pub label: String,
}

impl SelectOption {
    /// Create a new select option.
    pub fn new(value: Value, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
        }
    }
}
