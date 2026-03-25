use serde::{Deserialize, Serialize};

/// A single option in a select or multi-select parameter.
///
/// `SelectOption` intentionally does **not** derive `Eq` because the `value`
/// field is [`serde_json::Value`], which contains floating-point numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectOption {
    /// The value produced when this option is selected.
    pub value: serde_json::Value,

    /// Human-readable display label.
    pub label: String,

    /// Optional tooltip or help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Whether this option is shown but not selectable.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,

    /// Optional icon identifier (e.g. a URL or icon key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

impl SelectOption {
    /// Creates a new enabled option with no description or icon.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_parameter::option::SelectOption;
    ///
    /// let opt = SelectOption::new(serde_json::json!("us-east-1"), "US East");
    /// assert_eq!(opt.label, "US East");
    /// assert!(!opt.disabled);
    /// ```
    #[must_use]
    pub fn new(value: serde_json::Value, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
            description: None,
            disabled: false,
            icon: None,
        }
    }

    /// Sets a human-readable description (fluent builder).
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_parameter::option::SelectOption;
    ///
    /// let opt = SelectOption::new(serde_json::json!("json"), "JSON")
    ///     .description("JavaScript Object Notation");
    /// assert_eq!(opt.description.as_deref(), Some("JavaScript Object Notation"));
    /// ```
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets an icon identifier (fluent builder).
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_parameter::option::SelectOption;
    ///
    /// let opt = SelectOption::new(serde_json::json!("slack"), "Slack")
    ///     .icon("slack-icon");
    /// assert_eq!(opt.icon.as_deref(), Some("slack-icon"));
    /// ```
    #[must_use]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Marks this option as disabled (fluent builder).
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_parameter::option::SelectOption;
    ///
    /// let opt = SelectOption::new(serde_json::json!("beta"), "Beta Feature")
    ///     .disabled();
    /// assert!(opt.disabled);
    /// ```
    #[must_use]
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_option() {
        let opt = SelectOption::new(serde_json::json!("us-east-1"), "US East");
        assert_eq!(opt.label, "US East");
        assert_eq!(opt.value, serde_json::json!("us-east-1"));
        assert!(opt.description.is_none());
        assert!(!opt.disabled);
        assert!(opt.icon.is_none());
    }

    #[test]
    fn fluent_builders() {
        let opt = SelectOption::new(serde_json::json!("beta"), "Beta Feature")
            .description("Not yet available")
            .icon("warning")
            .disabled();

        assert_eq!(opt.description.as_deref(), Some("Not yet available"));
        assert_eq!(opt.icon.as_deref(), Some("warning"));
        assert!(opt.disabled);
    }

    #[test]
    fn option_equality() {
        let a = SelectOption::new(serde_json::json!(1), "A");
        let b = SelectOption::new(serde_json::json!(1), "A");
        assert_eq!(a, b);

        let c = SelectOption::new(serde_json::json!(2), "A");
        assert_ne!(a, c);
    }

    #[test]
    fn serde_round_trip_minimal() {
        let opt = SelectOption::new(serde_json::json!("application/json"), "JSON");
        let json = serde_json::to_string(&opt).unwrap();
        let deserialized: SelectOption = serde_json::from_str(&json).unwrap();
        assert_eq!(opt, deserialized);
    }

    #[test]
    fn serde_round_trip_full() {
        let opt = SelectOption::new(serde_json::json!("application/json"), "JSON")
            .description("JSON format")
            .icon("file-json")
            .disabled();

        let json = serde_json::to_string(&opt).unwrap();
        let deserialized: SelectOption = serde_json::from_str(&json).unwrap();
        assert_eq!(opt, deserialized);
    }

    #[test]
    fn optional_fields_omitted_from_json() {
        let opt = SelectOption::new(serde_json::json!(1), "K");
        let json = serde_json::to_string(&opt).unwrap();
        assert!(!json.contains("description"));
        assert!(!json.contains("disabled"));
        assert!(!json.contains("icon"));
    }

    #[test]
    fn disabled_option_serialized() {
        let opt = SelectOption::new(serde_json::json!("beta"), "Beta Feature").disabled();
        let json = serde_json::to_string(&opt).unwrap();
        assert!(json.contains("\"disabled\":true"));
    }

    #[test]
    fn icon_field_round_trip() {
        let opt = SelectOption::new(serde_json::json!("slack"), "Slack").icon("slack-logo");
        let json = serde_json::to_string(&opt).unwrap();
        assert!(json.contains("\"icon\":\"slack-logo\""));

        let deserialized: SelectOption = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.icon.as_deref(), Some("slack-logo"));
    }

    #[test]
    fn deserialize_without_optional_fields() {
        let json = r#"{"value":"x","label":"X"}"#;
        let opt: SelectOption = serde_json::from_str(json).unwrap();
        assert!(opt.description.is_none());
        assert!(!opt.disabled);
        assert!(opt.icon.is_none());
    }
}
