//! Static option definition for select fields.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One option in a `SelectField`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectOption {
    /// Stored value.
    pub value: Value,
    /// Display label.
    pub label: String,
    /// Optional longer description shown as hint text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether this option is selectable.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

impl SelectOption {
    /// Create a new select option with value and label.
    pub fn new(value: impl Into<Value>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            description: None,
            disabled: false,
        }
    }

    /// Attach a description to this option.
    #[must_use]
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Mark this option as disabled (not selectable).
    #[must_use]
    pub const fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn builder_defaults() {
        let o = SelectOption::new(json!("gh"), "GitHub");
        assert_eq!(o.value, json!("gh"));
        assert_eq!(o.label, "GitHub");
        assert!(o.description.is_none());
        assert!(!o.disabled);
    }

    #[test]
    fn roundtrip_omits_defaults() {
        let o = SelectOption::new(json!(1), "one");
        let s = serde_json::to_string(&o).unwrap();
        assert!(!s.contains("description"));
        assert!(!s.contains("disabled"));
    }
}
