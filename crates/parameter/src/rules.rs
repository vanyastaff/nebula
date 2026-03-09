//! Declarative validation rules.

/// Declarative validation rule attached to a field.
///
/// Static rules are evaluated at schema-validation time against the raw value.
/// Rules marked as **deferred** (`is_deferred() == true`) are skipped at
/// schema time and must be forwarded to the runtime execution layer.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum Rule {
    /// String must match the regular expression.
    Pattern {
        /// Regular expression pattern.
        pattern: String,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// String must be at least `min` characters.
    MinLength {
        /// Minimum character count (inclusive).
        min: usize,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// String must be at most `max` characters.
    MaxLength {
        /// Maximum character count (inclusive).
        max: usize,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Number must be ≥ `min`.
    Min {
        /// Lower bound (inclusive).
        min: serde_json::Number,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Number must be ≤ `max`.
    Max {
        /// Upper bound (inclusive).
        max: serde_json::Number,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Value must be one of the given options.
    OneOf {
        /// Allowed values.
        values: Vec<serde_json::Value>,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Collection must contain at least `min` items.
    MinItems {
        /// Minimum item count (inclusive).
        min: usize,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Collection must contain at most `max` items.
    MaxItems {
        /// Maximum item count (inclusive).
        max: usize,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Each list item must have a unique value for the given sub-field key.
    ///
    /// **Deferred**: not evaluated at schema-validation time.
    /// The runtime crate resolves it using expression context.
    UniqueBy {
        /// Sub-field key path within each item (e.g. `"name"`).
        key: String,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Custom expression-based validation.
    ///
    /// **Deferred**: not evaluated at schema-validation time.
    /// The runtime crate evaluates the expression against live context.
    Custom {
        /// Expression string forwarded to the runtime evaluator.
        expression: String,
        /// Optional custom error message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
}

impl Rule {
    /// Returns `true` if this rule requires runtime expression context to evaluate.
    ///
    /// Deferred rules are skipped during static schema validation and must be
    /// forwarded to the runtime execution layer.
    #[must_use]
    pub fn is_deferred(&self) -> bool {
        matches!(self, Self::UniqueBy { .. } | Self::Custom { .. })
    }
}
