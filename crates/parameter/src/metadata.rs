use super::{Condition, Rule};

/// Shared field metadata.
///
/// Flattened into each [`crate::schema::Field`] variant with `#[serde(flatten)]`.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FieldMetadata {
    /// Stable field identifier; must be unique within a schema.
    pub id: String,
    /// User-facing label.
    pub label: String,
    /// Longer descriptive text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Input placeholder hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Short contextual tooltip content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Default JSON value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// Whether the field is required.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
    /// Whether the field is secret/masked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub secret: bool,
    /// Whether the field accepts expression-backed values under runtime policy.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub expression: bool,
    /// Validation rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    /// Show this field only when the condition is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<Condition>,
    /// Require this field only when the condition is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_when: Option<Condition>,
    /// Disable this field when the condition is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_when: Option<Condition>,
}

impl FieldMetadata {
    /// Creates metadata with a stable field id.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ..Self::default()
        }
    }

    /// Sets the user-facing label.
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = label.into();
    }

    /// Sets the description tooltip.
    pub fn set_description(&mut self, description: impl Into<String>) {
        self.description = Some(description.into());
    }

    /// Sets placeholder text.
    pub fn set_placeholder(&mut self, placeholder: impl Into<String>) {
        self.placeholder = Some(placeholder.into());
    }

    /// Sets short hint text.
    pub fn set_hint(&mut self, hint: impl Into<String>) {
        self.hint = Some(hint.into());
    }

    /// Marks metadata as required.
    pub fn set_required(&mut self, required: bool) {
        self.required = required;
    }

    /// Marks metadata as secret/masked.
    pub fn set_secret(&mut self, secret: bool) {
        self.secret = secret;
    }

    /// Sets default JSON value.
    pub fn set_default(&mut self, value: serde_json::Value) {
        self.default = Some(value);
    }

    /// Appends a declarative validation rule.
    pub fn add_rule(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    /// Sets visibility condition.
    pub fn set_visible_when(&mut self, condition: Condition) {
        self.visible_when = Some(condition);
    }

    /// Sets conditional-required rule.
    pub fn set_required_when(&mut self, condition: Condition) {
        self.required_when = Some(condition);
    }

    /// Sets disabled/read-only condition.
    pub fn set_disabled_when(&mut self, condition: Condition) {
        self.disabled_when = Some(condition);
    }
}
