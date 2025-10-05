use bon::Builder;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, HasValue, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterType, ParameterValidation, ParameterValue, Validatable,
};

// Default Time-To-Live in seconds (1 hour)
const DEFAULT_TTL: u64 = 3600;

/// Parameter with expirable values that automatically expire after a TTL
/// Acts as a container that wraps another parameter with expiration logic
#[derive(Serialize)]
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
    pub children: Option<Box<dyn ParameterType>>,
}

/// Configuration options for expirable parameters
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
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

impl ExpirableValue {
    /// Creates a new ExpirableValue with the specified TTL in seconds
    pub fn new(value: nebula_value::Value, ttl: u64) -> Self {
        let now = Utc::now();
        let expires_at = now + Duration::seconds(ttl as i64);
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
        self.expires_at = Utc::now() + Duration::seconds(ttl as i64);
    }

    /// Create a new ExpirableValue with a string value
    pub fn new_string(value: impl Into<String>, ttl: u64) -> Self {
        Self::new(nebula_value::Value::text(value.into()), ttl)
    }

    /// Create a new ExpirableValue with a boolean value
    pub fn new_bool(value: bool, ttl: u64) -> Self {
        Self::new(nebula_value::Value::boolean(value), ttl)
    }

    /// Create a new ExpirableValue with an integer value
    pub fn new_int(value: i64, ttl: u64) -> Self {
        Self::new(nebula_value::Value::integer(value), ttl)
    }

    /// Create a new ExpirableValue from ParameterValue
    pub fn from_parameter_value(param_value: &ParameterValue, ttl: u64) -> Self {
        let nebula_val = match param_value {
            ParameterValue::Value(v) => v.clone(),
            ParameterValue::Expression(expr) => nebula_value::Value::text(expr.clone()),
            ParameterValue::Routing(_) => nebula_value::Value::text("routing_value"),
            ParameterValue::Mode(mode_val) => mode_val.value.clone(),
            ParameterValue::Expirable(exp_val) => exp_val.value.clone(),
            ParameterValue::List(list_val) => {
                // Convert Vec<nebula_value::Value> to Vec<serde_json::Value> for Array
                let json_items: Vec<serde_json::Value> = list_val
                    .items
                    .iter()
                    .filter_map(|v| serde_json::to_value(v).ok())
                    .collect();
                nebula_value::Value::Array(nebula_value::Array::from(json_items))
            }
            ParameterValue::Object(_obj_val) => nebula_value::Value::text("object_value"),
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

impl ParameterType for ExpirableParameter {
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

    fn get_value(&self) -> Option<&Self::Value> {
        // Check if value is expired and handle auto-clear
        if let Some(value) = &self.value {
            if value.is_expired() {
                if let Some(options) = &self.options {
                    if options.auto_clear_expired {
                        return None; // Act as if no value exists
                    }
                }
            }
        }
        self.value.as_ref()
    }

    fn get_value_mut(&mut self) -> Option<&mut Self::Value> {
        // Check if value is expired and handle auto-clear
        if let Some(value) = &self.value {
            if value.is_expired() {
                if let Some(options) = &self.options {
                    if options.auto_clear_expired {
                        self.value = None;
                        return None;
                    }
                }
            }
        }
        self.value.as_mut()
    }

    fn set_value_unchecked(&mut self, mut value: Self::Value) -> Result<(), ParameterError> {
        // Auto-refresh if enabled
        if let Some(options) = &self.options {
            if options.auto_refresh {
                value.refresh(options.ttl);
            }
        }
        self.value = Some(value);
        Ok(())
    }

    fn default_value(&self) -> Option<&Self::Value> {
        self.default.as_ref()
    }

    fn clear_value(&mut self) {
        self.value = None;
    }

    fn get_parameter_value(&self) -> Option<ParameterValue> {
        self.value
            .as_ref()
            .map(|exp_val| ParameterValue::Expirable(exp_val.clone()))
    }

    fn set_parameter_value(
        &mut self,
        value: impl Into<ParameterValue>,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            ParameterValue::Expirable(exp_val) => {
                self.value = Some(exp_val);
                Ok(())
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected expirable value".to_string(),
            }),
        }
    }
}

impl Validatable for ExpirableParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn value_to_nebula_value(&self, value: &Self::Value) -> nebula_value::Value {
        crate::core::json_to_nebula(&serde_json::to_value(value).unwrap()).unwrap_or(serde_json::Value::Null)
    }

    fn is_empty_value(&self, value: &Self::Value) -> bool {
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
        child: Option<Box<dyn ParameterType>>,
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
    pub fn child(&self) -> Option<&Box<dyn ParameterType>> {
        self.children.as_ref()
    }

    /// Set the child parameter
    pub fn set_child(&mut self, child: Option<Box<dyn ParameterType>>) {
        self.children = child;
    }

    /// Check if the current value has expired
    pub fn is_expired(&self) -> bool {
        self.value.as_ref().map(|v| v.is_expired()).unwrap_or(true)
    }

    /// Get the remaining TTL in seconds
    pub fn ttl(&self) -> Option<u64> {
        self.value
            .as_ref()
            .and_then(|v| if v.is_expired() { None } else { Some(v.ttl()) })
    }

    /// Get the age of the current value in seconds
    pub fn age(&self) -> Option<u64> {
        self.value.as_ref().map(|v| v.age())
    }

    /// Check if the value is expiring soon
    pub fn is_expiring_soon(&self) -> bool {
        if let Some(value) = &self.value {
            if let Some(options) = &self.options {
                if let Some(threshold) = options.warning_threshold {
                    return value.is_expiring_soon(threshold);
                }
            }
        }
        false
    }

    /// Refresh the current value's expiration time
    pub fn refresh_ttl(&mut self) -> Result<(), ParameterError> {
        if let Some(value) = &mut self.value {
            let ttl = self.options.as_ref().map(|o| o.ttl).unwrap_or(DEFAULT_TTL);
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
        if let Some(value) = self.get_value() {
            if !value.is_expired() {
                Some(&value.value)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Get the TTL configuration
    pub fn get_ttl_config(&self) -> u64 {
        self.options.as_ref().map(|o| o.ttl).unwrap_or(DEFAULT_TTL)
    }
}
