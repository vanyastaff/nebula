use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::core::traits::Expressible;
use crate::core::{
    Displayable, HasValue, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
};
use nebula_expression::MaybeExpression;
use nebula_value::Value;

// Default Time-To-Live in seconds (1 hour)
const DEFAULT_TTL: u64 = 3600;

/// Parameter with expirable values that automatically expire after a TTL
/// Acts as a container that wraps another parameter with expiration logic
#[derive(Serialize, bon::Builder)]
pub struct ExpirableParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<ExpirableValue>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<ExpirableValue>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<ExpirableParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,

    /// The child parameter that this expirable parameter wraps
    #[serde(skip)]
    pub children: Option<Box<dyn Parameter>>,
}

/// Configuration options for expirable parameters
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct ExpirableParameterOptions {
    /// Time-To-Live in seconds before values expire
    #[serde(default = "default_ttl")]
    pub ttl: u64,

    /// Whether to auto-refresh values on access
    #[builder(default)]
    #[serde(default)]
    pub auto_refresh: bool,

    /// Whether expired values should be cleared automatically
    #[serde(default = "default_auto_clear")]
    pub auto_clear_expired: bool,

    /// Warning threshold in seconds before expiration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_threshold: Option<u64>,
}

fn default_ttl() -> u64 {
    DEFAULT_TTL
}

fn default_auto_clear() -> bool {
    true
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

/// Maximum TTL value to prevent overflow (about 292 billion years, but we cap at i64::MAX seconds)
const MAX_TTL_SECONDS: u64 = i64::MAX as u64;

impl ExpirableValue {
    /// Creates a new `ExpirableValue` with the specified TTL in seconds
    pub fn new(value: nebula_value::Value, ttl: u64) -> Self {
        let now = Utc::now();
        // Saturate TTL to prevent i64 overflow
        let safe_ttl = ttl.min(MAX_TTL_SECONDS) as i64;
        let expires_at = now + Duration::seconds(safe_ttl);
        Self {
            value,
            expires_at,
            created_at: now,
        }
    }

    /// Checks if the value has expired
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    /// Gets the remaining Time-To-Live in seconds
    pub fn ttl(&self) -> u64 {
        if self.is_expired() {
            0
        } else {
            (self.expires_at - Utc::now()).num_seconds().max(0) as u64
        }
    }

    /// Gets the age of this value in seconds
    pub fn age(&self) -> u64 {
        (Utc::now() - self.created_at).num_seconds().max(0) as u64
    }

    /// Checks if the value is approaching expiration
    pub fn is_expiring_soon(&self, threshold_seconds: u64) -> bool {
        !self.is_expired() && self.ttl() <= threshold_seconds
    }

    /// Refreshes the expiration time with new TTL
    pub fn refresh(&mut self, ttl: u64) {
        // Saturate TTL to prevent i64 overflow
        let safe_ttl = ttl.min(MAX_TTL_SECONDS) as i64;
        self.expires_at = Utc::now() + Duration::seconds(safe_ttl);
    }

    /// Create a new `ExpirableValue` with a string value
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

    /// Create a new `ExpirableValue` from `ParameterValue` (`MaybeExpression`<Value>)
    pub fn from_parameter_value(param_value: &MaybeExpression<Value>, ttl: u64) -> Self {
        let nebula_val = match param_value {
            MaybeExpression::Value(v) => v.clone(),
            MaybeExpression::Expression(expr) => nebula_value::Value::text(&expr.source),
        };
        Self::new(nebula_val, ttl)
    }
}

// Manual Debug implementation since we skip trait objects
impl std::fmt::Debug for ExpirableParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExpirableParameter")
            .field("metadata", &self.metadata)
            .field("value", &self.value)
            .field("default", &self.default)
            .field("options", &self.options)
            .field("display", &self.display)
            .field("validation", &self.validation)
            .field("children", &"Option<Box<dyn ParameterType>>")
            .finish()
    }
}

impl Parameter for ExpirableParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Expirable
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for ExpirableParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ExpirableParameter({})", self.metadata.name)
    }
}

impl HasValue for ExpirableParameter {
    type Value = ExpirableValue;

    fn get(&self) -> Option<&Self::Value> {
        // Check if value is expired and handle auto-clear
        if let Some(value) = &self.value
            && value.is_expired()
            && let Some(options) = &self.options
            && options.auto_clear_expired
        {
            return None; // Act as if no value exists
        }
        self.value.as_ref()
    }

    fn get_mut(&mut self) -> Option<&mut Self::Value> {
        // Check if value is expired and handle auto-clear
        if let Some(value) = &self.value
            && value.is_expired()
            && let Some(options) = &self.options
            && options.auto_clear_expired
        {
            self.value = None;
            return None;
        }
        self.value.as_mut()
    }

    fn set(&mut self, mut value: Self::Value) -> Result<(), ParameterError> {
        // Auto-refresh if enabled
        if let Some(options) = &self.options
            && options.auto_refresh
        {
            value.refresh(options.ttl);
        }
        self.value = Some(value);
        Ok(())
    }

    fn default(&self) -> Option<&Self::Value> {
        self.default.as_ref()
    }

    fn clear(&mut self) {
        self.value = None;
    }
}

#[async_trait::async_trait]
impl Expressible for ExpirableParameter {
    fn to_expression(&self) -> Option<MaybeExpression<Value>> {
        // Convert ExpirableValue to MaybeExpression<Value>
        // Return the inner value if not expired, otherwise None
        self.value.as_ref().and_then(|exp_val| {
            if exp_val.is_expired() {
                None
            } else {
                Some(MaybeExpression::Value(exp_val.value.clone()))
            }
        })
    }

    fn from_expression(
        &mut self,
        value: impl Into<MaybeExpression<Value>> + Send,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        // Get TTL from options or use default
        let ttl = self.options.as_ref().map_or(3600, |opts| opts.ttl); // Default 1 hour

        let exp_val = ExpirableValue::from_parameter_value(&value, ttl);
        self.value = Some(exp_val);
        Ok(())
    }
}

impl Validatable for ExpirableParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }
    fn is_empty(&self, value: &Self::Value) -> bool {
        value.is_expired()
            || match &value.value {
                nebula_value::Value::Text(s) => s.as_str().trim().is_empty(),
                nebula_value::Value::Null => true,
                nebula_value::Value::Array(a) => a.is_empty(),
                nebula_value::Value::Object(o) => o.is_empty(),
                _ => false,
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

impl ExpirableParameter {
    /// Create a new expirable parameter
    pub fn new(
        key: &str,
        name: &str,
        description: &str,
        child: Option<Box<dyn Parameter>>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            metadata: ParameterMetadata {
                key: nebula_core::ParameterKey::new(key)?,
                name: name.to_string(),
                description: description.to_string(),
                required: false,
                placeholder: Some("Expirable value...".to_string()),
                hint: Some("Value will expire after TTL".to_string()),
            },
            value: None,
            default: None,
            options: Some(ExpirableParameterOptions::default()),
            display: None,
            validation: None,
            children: child,
        })
    }

    /// Get the child parameter
    pub fn child(&self) -> Option<&Box<dyn Parameter>> {
        self.children.as_ref()
    }

    /// Set the child parameter
    pub fn set_child(&mut self, child: Option<Box<dyn Parameter>>) {
        self.children = child;
    }

    /// Check if the current value has expired
    pub fn is_expired(&self) -> bool {
        self.value.as_ref().is_none_or(ExpirableValue::is_expired)
    }

    /// Get the remaining TTL in seconds
    pub fn ttl(&self) -> Option<u64> {
        self.value
            .as_ref()
            .and_then(|v| if v.is_expired() { None } else { Some(v.ttl()) })
    }

    /// Get the age of the current value in seconds
    pub fn age(&self) -> Option<u64> {
        self.value.as_ref().map(ExpirableValue::age)
    }

    /// Check if the value is expiring soon
    pub fn is_expiring_soon(&self) -> bool {
        if let Some(value) = &self.value
            && let Some(options) = &self.options
            && let Some(threshold) = options.warning_threshold
        {
            return value.is_expiring_soon(threshold);
        }
        false
    }

    /// Refresh the current value's expiration time
    #[must_use = "operation result must be checked"]
    pub fn refresh_ttl(&mut self) -> Result<(), ParameterError> {
        if let Some(value) = &mut self.value {
            let ttl = self.options.as_ref().map_or(DEFAULT_TTL, |o| o.ttl);
            value.refresh(ttl);
            Ok(())
        } else {
            Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "No value to refresh".to_string(),
            })
        }
    }

    /// Get the actual value if not expired
    pub fn get_actual_value(&self) -> Option<&nebula_value::Value> {
        if let Some(value) = self.get() {
            if value.is_expired() {
                None
            } else {
                Some(&value.value)
            }
        } else {
            None
        }
    }

    /// Get the TTL configuration
    pub fn get_ttl_config(&self) -> u64 {
        self.options.as_ref().map_or(DEFAULT_TTL, |o| o.ttl)
    }
}
