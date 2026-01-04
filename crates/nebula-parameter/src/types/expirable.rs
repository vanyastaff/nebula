use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::ParameterError;
use crate::core::{
    Describable, Displayable, Parameter, ParameterDisplay, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_expression::MaybeExpression;
use nebula_value::{Value, ValueKind};

/// Default Time-To-Live in seconds (1 hour)
const DEFAULT_TTL: u64 = 3600;

/// Maximum TTL value to prevent overflow (about 292 billion years, but we cap at i64::MAX seconds)
const MAX_TTL_SECONDS: u64 = i64::MAX as u64;

fn default_ttl() -> u64 {
    DEFAULT_TTL
}

fn default_auto_clear() -> bool {
    true
}

/// Configuration options for expirable parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpirableParameterOptions {
    /// Time-To-Live in seconds before values expire
    #[serde(default = "default_ttl")]
    pub ttl: u64,

    /// Whether to auto-refresh values on access
    #[serde(default)]
    pub auto_refresh: bool,

    /// Whether expired values should be cleared automatically
    #[serde(default = "default_auto_clear")]
    pub auto_clear_expired: bool,

    /// Warning threshold in seconds before expiration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning_threshold: Option<u64>,
}

impl Default for ExpirableParameterOptions {
    fn default() -> Self {
        Self {
            ttl: DEFAULT_TTL,
            auto_refresh: false,
            auto_clear_expired: true,
            warning_threshold: None,
        }
    }
}

/// Builder for ExpirableParameterOptions
#[derive(Debug, Default)]
pub struct ExpirableParameterOptionsBuilder {
    ttl: u64,
    auto_refresh: bool,
    auto_clear_expired: bool,
    warning_threshold: Option<u64>,
}

impl ExpirableParameterOptionsBuilder {
    /// Create a new options builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            ttl: DEFAULT_TTL,
            auto_refresh: false,
            auto_clear_expired: true,
            warning_threshold: None,
        }
    }

    /// Set the TTL in seconds
    #[must_use]
    pub fn ttl(mut self, ttl: u64) -> Self {
        self.ttl = ttl;
        self
    }

    /// Set the TTL in minutes
    #[must_use]
    pub fn ttl_minutes(mut self, minutes: u64) -> Self {
        self.ttl = minutes * 60;
        self
    }

    /// Set the TTL in hours
    #[must_use]
    pub fn ttl_hours(mut self, hours: u64) -> Self {
        self.ttl = hours * 3600;
        self
    }

    /// Set whether to auto-refresh values on access
    #[must_use]
    pub fn auto_refresh(mut self, auto_refresh: bool) -> Self {
        self.auto_refresh = auto_refresh;
        self
    }

    /// Set whether expired values should be cleared automatically
    #[must_use]
    pub fn auto_clear_expired(mut self, auto_clear: bool) -> Self {
        self.auto_clear_expired = auto_clear;
        self
    }

    /// Set the warning threshold in seconds
    #[must_use]
    pub fn warning_threshold(mut self, threshold: u64) -> Self {
        self.warning_threshold = Some(threshold);
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> ExpirableParameterOptions {
        ExpirableParameterOptions {
            ttl: self.ttl,
            auto_refresh: self.auto_refresh,
            auto_clear_expired: self.auto_clear_expired,
            warning_threshold: self.warning_threshold,
        }
    }
}

impl ExpirableParameterOptions {
    /// Create a new options builder
    #[must_use]
    pub fn builder() -> ExpirableParameterOptionsBuilder {
        ExpirableParameterOptionsBuilder::new()
    }
}

/// A value with an expiration timestamp
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ExpirableValue {
    /// The actual value that will expire
    pub value: nebula_value::Value,

    /// The timestamp when this value expires
    pub expires_at: DateTime<Utc>,

    /// The timestamp when this value was created
    pub created_at: DateTime<Utc>,
}

impl From<ExpirableValue> for nebula_value::Value {
    fn from(expirable: ExpirableValue) -> Self {
        use crate::ValueRefExt;
        let mut obj = serde_json::Map::new();
        obj.insert("value".to_string(), expirable.value.to_json());
        obj.insert(
            "expires_at".to_string(),
            nebula_value::Value::text(expirable.expires_at.to_rfc3339()).to_json(),
        );
        obj.insert(
            "created_at".to_string(),
            nebula_value::Value::text(expirable.created_at.to_rfc3339()).to_json(),
        );

        use crate::JsonValueExt;
        serde_json::Value::Object(obj)
            .to_nebula_value()
            .unwrap_or(nebula_value::Value::Null)
    }
}

impl ExpirableValue {
    /// Creates a new `ExpirableValue` with the specified TTL in seconds
    #[must_use]
    pub fn new(value: nebula_value::Value, ttl: u64) -> Self {
        let now = Utc::now();
        let safe_ttl = ttl.min(MAX_TTL_SECONDS) as i64;
        let expires_at = now + Duration::seconds(safe_ttl);
        Self {
            value,
            expires_at,
            created_at: now,
        }
    }

    /// Checks if the value has expired
    #[must_use]
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    /// Gets the remaining Time-To-Live in seconds
    #[must_use]
    pub fn ttl(&self) -> u64 {
        if self.is_expired() {
            0
        } else {
            (self.expires_at - Utc::now()).num_seconds().max(0) as u64
        }
    }

    /// Gets the age of this value in seconds
    #[must_use]
    pub fn age(&self) -> u64 {
        (Utc::now() - self.created_at).num_seconds().max(0) as u64
    }

    /// Checks if the value is approaching expiration
    #[must_use]
    pub fn is_expiring_soon(&self, threshold_seconds: u64) -> bool {
        !self.is_expired() && self.ttl() <= threshold_seconds
    }

    /// Refreshes the expiration time with new TTL
    pub fn refresh(&mut self, ttl: u64) {
        let safe_ttl = ttl.min(MAX_TTL_SECONDS) as i64;
        self.expires_at = Utc::now() + Duration::seconds(safe_ttl);
    }

    /// Create a new `ExpirableValue` with a string value
    #[must_use]
    pub fn new_string(value: impl Into<String>, ttl: u64) -> Self {
        Self::new(nebula_value::Value::text(value.into()), ttl)
    }

    /// Create a new `ExpirableValue` with a boolean value
    #[must_use]
    pub fn new_bool(value: bool, ttl: u64) -> Self {
        Self::new(nebula_value::Value::boolean(value), ttl)
    }

    /// Create a new `ExpirableValue` with an integer value
    #[must_use]
    pub fn new_int(value: i64, ttl: u64) -> Self {
        Self::new(nebula_value::Value::integer(value), ttl)
    }

    /// Create a new `ExpirableValue` from `ParameterValue` (`MaybeExpression<Value>`)
    #[must_use]
    pub fn from_parameter_value(param_value: &MaybeExpression<Value>, ttl: u64) -> Self {
        let nebula_val = match param_value {
            MaybeExpression::Value(v) => v.clone(),
            MaybeExpression::Expression(expr) => nebula_value::Value::text(&expr.source),
        };
        Self::new(nebula_val, ttl)
    }
}

/// Parameter with expirable values that automatically expire after a TTL.
///
/// Acts as a container that wraps another parameter with expiration logic.
#[derive(Serialize)]
pub struct ExpirableParameter {
    /// Parameter metadata (flattened for cleaner JSON)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default expirable value
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ExpirableValue>,

    /// Configuration options
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ExpirableParameterOptions>,

    /// The child parameter that this expirable parameter wraps
    #[serde(skip)]
    pub children: Option<Box<dyn Parameter>>,

    /// Display configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

impl fmt::Debug for ExpirableParameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExpirableParameter")
            .field("metadata", &self.metadata)
            .field("default", &self.default)
            .field("options", &self.options)
            .field("children", &"Option<Box<dyn Parameter>>")
            .field("display", &self.display)
            .field("validation", &self.validation)
            .finish()
    }
}

/// Builder for ExpirableParameter
#[derive(Default)]
pub struct ExpirableParameterBuilder {
    key: Option<String>,
    name: Option<String>,
    description: Option<String>,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    default: Option<ExpirableValue>,
    options: Option<ExpirableParameterOptions>,
    children: Option<Box<dyn Parameter>>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl ExpirableParameterBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        <Self as Default>::default()
    }

    /// Set the parameter key (required)
    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the parameter name (required)
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the parameter description
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set whether the parameter is required
    #[must_use]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Set the placeholder text
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set the hint text
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Set the default expirable value
    #[must_use]
    pub fn default(mut self, default: ExpirableValue) -> Self {
        self.default = Some(default);
        self
    }

    /// Set the TTL in seconds
    #[must_use]
    pub fn ttl(mut self, ttl: u64) -> Self {
        let options = self
            .options
            .get_or_insert_with(ExpirableParameterOptions::default);
        options.ttl = ttl;
        self
    }

    /// Set the TTL in minutes
    #[must_use]
    pub fn ttl_minutes(mut self, minutes: u64) -> Self {
        let options = self
            .options
            .get_or_insert_with(ExpirableParameterOptions::default);
        options.ttl = minutes * 60;
        self
    }

    /// Set the TTL in hours
    #[must_use]
    pub fn ttl_hours(mut self, hours: u64) -> Self {
        let options = self
            .options
            .get_or_insert_with(ExpirableParameterOptions::default);
        options.ttl = hours * 3600;
        self
    }

    /// Set whether to auto-refresh values
    #[must_use]
    pub fn auto_refresh(mut self, auto_refresh: bool) -> Self {
        let options = self
            .options
            .get_or_insert_with(ExpirableParameterOptions::default);
        options.auto_refresh = auto_refresh;
        self
    }

    /// Set whether to auto-clear expired values
    #[must_use]
    pub fn auto_clear_expired(mut self, auto_clear: bool) -> Self {
        let options = self
            .options
            .get_or_insert_with(ExpirableParameterOptions::default);
        options.auto_clear_expired = auto_clear;
        self
    }

    /// Set the warning threshold in seconds
    #[must_use]
    pub fn warning_threshold(mut self, threshold: u64) -> Self {
        let options = self
            .options
            .get_or_insert_with(ExpirableParameterOptions::default);
        options.warning_threshold = Some(threshold);
        self
    }

    /// Set the options
    #[must_use]
    pub fn options(mut self, options: ExpirableParameterOptions) -> Self {
        self.options = Some(options);
        self
    }

    /// Set the child parameter
    #[must_use]
    pub fn child(mut self, child: Box<dyn Parameter>) -> Self {
        self.children = Some(child);
        self
    }

    /// Set the display configuration
    #[must_use]
    pub fn display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }

    /// Set the validation rules
    #[must_use]
    pub fn validation(mut self, validation: ParameterValidation) -> Self {
        self.validation = Some(validation);
        self
    }

    /// Build the ExpirableParameter
    ///
    /// # Errors
    ///
    /// Returns an error if required fields are missing or invalid
    pub fn build(self) -> Result<ExpirableParameter, ParameterError> {
        let metadata = ParameterMetadata::builder()
            .key(
                self.key
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "key".into(),
                    })?,
            )
            .name(
                self.name
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "name".into(),
                    })?,
            )
            .description(self.description.unwrap_or_default())
            .required(self.required)
            .maybe_placeholder(self.placeholder)
            .maybe_hint(self.hint)
            .build()?;

        Ok(ExpirableParameter {
            metadata,
            default: self.default,
            options: self.options,
            children: self.children,
            display: self.display,
            validation: self.validation,
        })
    }
}

impl ExpirableParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> ExpirableParameterBuilder {
        ExpirableParameterBuilder::new()
    }

    /// Get the child parameter
    #[must_use]
    pub fn child(&self) -> Option<&dyn Parameter> {
        self.children.as_deref()
    }

    /// Set the child parameter
    pub fn set_child(&mut self, child: Option<Box<dyn Parameter>>) {
        self.children = child;
    }

    /// Get the TTL configuration
    #[must_use]
    pub fn get_ttl_config(&self) -> u64 {
        self.options.as_ref().map_or(DEFAULT_TTL, |o| o.ttl)
    }

    /// Check if auto-refresh is enabled
    #[must_use]
    pub fn is_auto_refresh(&self) -> bool {
        self.options.as_ref().is_some_and(|o| o.auto_refresh)
    }

    /// Check if auto-clear expired is enabled
    #[must_use]
    pub fn is_auto_clear_expired(&self) -> bool {
        self.options.as_ref().is_none_or(|o| o.auto_clear_expired)
    }

    /// Get the warning threshold
    #[must_use]
    pub fn warning_threshold(&self) -> Option<u64> {
        self.options.as_ref()?.warning_threshold
    }
}

impl Describable for ExpirableParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Expirable
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl fmt::Display for ExpirableParameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ExpirableParameter({})", self.metadata.name)
    }
}

impl Validatable for ExpirableParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::Object)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        if let Some(obj) = value.as_object() {
            // Check if expired
            if let Some(expires_at) = obj.get("expires_at")
                && let Some(timestamp_str) = expires_at.as_text()
                && let Ok(timestamp) = DateTime::parse_from_rfc3339(timestamp_str.as_str())
                && timestamp.with_timezone(&Utc) <= Utc::now()
            {
                return true;
            }
            // Check if inner value is empty
            if let Some(inner_value) = obj.get("value") {
                match inner_value {
                    nebula_value::Value::Text(s) => s.as_str().trim().is_empty(),
                    nebula_value::Value::Null => true,
                    nebula_value::Value::Array(a) => a.is_empty(),
                    nebula_value::Value::Object(o) => o.is_empty(),
                    _ => false,
                }
            } else {
                true
            }
        } else {
            value.is_null()
        }
    }
}

impl Displayable for ExpirableParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration as StdDuration;

    #[test]
    fn test_expirable_parameter_builder() {
        let param = ExpirableParameter::builder()
            .key("cache_token")
            .name("Cached Token")
            .description("A token that expires after some time")
            .ttl(3600)
            .auto_refresh(true)
            .warning_threshold(300)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "cache_token");
        assert_eq!(param.metadata.name, "Cached Token");
        assert_eq!(param.get_ttl_config(), 3600);
        assert!(param.is_auto_refresh());
        assert_eq!(param.warning_threshold(), Some(300));
    }

    #[test]
    fn test_expirable_parameter_missing_key() {
        let result = ExpirableParameter::builder().name("Test").build();

        assert!(result.is_err());
    }

    #[test]
    fn test_expirable_value_creation() {
        let value = ExpirableValue::new(nebula_value::Value::text("test"), 3600);
        assert!(!value.is_expired());
        assert!(value.ttl() > 0);
        assert!(value.ttl() <= 3600);
    }

    #[test]
    fn test_expirable_value_expiration() {
        let value = ExpirableValue::new(nebula_value::Value::text("test"), 1);
        assert!(!value.is_expired());

        // Wait for expiration (with a small buffer)
        sleep(StdDuration::from_millis(1100));
        assert!(value.is_expired());
        assert_eq!(value.ttl(), 0);
    }

    #[test]
    fn test_expirable_value_refresh() {
        let mut value = ExpirableValue::new(nebula_value::Value::text("test"), 1);
        sleep(StdDuration::from_millis(500));

        value.refresh(3600);
        assert!(!value.is_expired());
        assert!(value.ttl() > 3500);
    }

    #[test]
    fn test_expirable_value_is_expiring_soon() {
        let value = ExpirableValue::new(nebula_value::Value::text("test"), 5);
        assert!(value.is_expiring_soon(10));
        assert!(!value.is_expiring_soon(1));
    }

    #[test]
    fn test_expirable_value_convenience_constructors() {
        let string_val = ExpirableValue::new_string("hello", 3600);
        assert!(matches!(string_val.value, nebula_value::Value::Text(_)));

        let bool_val = ExpirableValue::new_bool(true, 3600);
        assert!(matches!(bool_val.value, nebula_value::Value::Boolean(_)));

        let int_val = ExpirableValue::new_int(42, 3600);
        assert!(matches!(int_val.value, nebula_value::Value::Integer(_)));
    }

    #[test]
    fn test_expirable_options_builder() {
        let options = ExpirableParameterOptions::builder()
            .ttl_hours(2)
            .auto_refresh(true)
            .auto_clear_expired(false)
            .warning_threshold(600)
            .build();

        assert_eq!(options.ttl, 7200);
        assert!(options.auto_refresh);
        assert!(!options.auto_clear_expired);
        assert_eq!(options.warning_threshold, Some(600));
    }

    #[test]
    fn test_ttl_convenience_methods() {
        let param = ExpirableParameter::builder()
            .key("test")
            .name("Test")
            .ttl_hours(1)
            .build()
            .unwrap();

        assert_eq!(param.get_ttl_config(), 3600);

        let param2 = ExpirableParameter::builder()
            .key("test2")
            .name("Test 2")
            .ttl_minutes(30)
            .build()
            .unwrap();

        assert_eq!(param2.get_ttl_config(), 1800);
    }
}
