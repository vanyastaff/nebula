use chrono::{DateTime, Duration, Utc};
use derive_builder::Builder;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Parameter, ParameterDisplay, ParameterError, ParameterMetadata, ParameterType,
    ParameterValidation, ParameterValue,
};

// Default Time-To-Live in seconds (1 hour)
const DEFAULT_TTL: u64 = 3600;

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
#[builder(pattern = "owned", setter(strip_option))]
pub struct ExpirableParameter {
    /// The wrapped parameter that will have expirable values
    #[builder(setter)]
    pub parameter: Box<ParameterType>,

    /// Configuration options for the expirable behavior
    #[builder(default, setter(strip_option))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<ExpirableParameterOptions>,
}

/// Configuration options for expirable parameters
///
/// These options control how the expiration functionality behaves,
/// including the TTL (Time-To-Live) duration for values.
#[derive(Debug, Clone, Builder, Serialize, Deserialize, Default)]
#[builder(pattern = "owned", setter(strip_option), default)]
pub struct ExpirableParameterOptions {
    /// Time-To-Live in seconds before values expire
    ///
    /// Defaults to 3600 seconds (1 hour) if not specified
    #[builder(default = "DEFAULT_TTL")]
    pub ttl: u64,
}

impl Parameter for ExpirableParameter {
    /// Returns the metadata associated with this parameter
    ///
    /// This delegates to the wrapped parameter's metadata
    fn metadata(&self) -> &ParameterMetadata {
        self.parameter.as_ref().metadata()
    }

    /// Gets the current value of the parameter, if any
    ///
    /// This method will check if the value has expired and return None if it
    /// has. Otherwise, it delegates to the wrapped parameter.
    fn get_value(&self) -> Option<&ParameterValue> {
        // Get the value from the inner parameter
        let inner_value = self.parameter.as_ref().get_value()?;

        // Use `ref` to borrow the inner value in the pattern
        if let ParameterValue::Expirable(ref exp_value) = *inner_value {
            // Try to deserialize the ExpirableValue
            if let Ok(expirable) = serde_json::from_value::<ExpirableValue>(exp_value.clone()) {
                if expirable.is_expired() {
                    return None; // Value has expired
                }
            }
        }

        // If we reach this point, either it's not Expirable, or it hasn't expired
        Some(inner_value)
    }

    /// Sets a new value for the parameter
    ///
    /// This method wraps the provided value in an ExpirableValue with the
    /// configured TTL before setting it on the wrapped parameter.
    fn set_value(&mut self, value: ParameterValue) -> Result<(), ParameterError> {
        // Prepare the expirable value based on the input
        let expirable_value = match value {
            // If already an Expirable value, use it as is
            ParameterValue::Expirable(_) => value,

            // Otherwise, wrap it in an ExpirableValue
            other => {
                // Get TTL from options or use default
                let ttl = self
                    .options
                    .as_ref()
                    .map(|opt| opt.ttl as i64)
                    .unwrap_or(DEFAULT_TTL as i64);

                // Create an ExpirableValue
                let expirable = ExpirableValue::new(
                    match serde_json::to_value(&other) {
                        Ok(value) => value,
                        Err(err) => {
                            return Err(ParameterError::SerializationError(err));
                        }
                    },
                    ttl,
                );

                // Create a ParameterValue::Expirable
                let serialized = match serde_json::to_value(expirable) {
                    Ok(value) => value,
                    Err(err) => {
                        return Err(ParameterError::SerializationError(err));
                    }
                };

                ParameterValue::Expirable(serialized)
            }
        };

        // Set the value on the inner parameter
        self.parameter.as_mut().set_value(expirable_value)
    }

    /// Returns the validation rules for this parameter, if any
    ///
    /// This delegates to the wrapped parameter's validation
    fn validation(&self) -> Option<&ParameterValidation> {
        self.parameter.as_ref().validation()
    }

    /// Returns the display settings for this parameter, if any
    ///
    /// This delegates to the wrapped parameter's display settings
    fn display(&self) -> Option<&ParameterDisplay> {
        self.parameter.as_ref().display()
    }
}

/// A value with an expiration timestamp
///
/// This wrapper adds an expiration timestamp to any JSON value.
/// After the expiration time, the value is considered invalid.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExpirableValue {
    /// The actual value that will expire
    pub value: Value,

    /// The timestamp when this value expires
    pub expires_at: DateTime<Utc>,
}

impl ExpirableValue {
    /// Creates a new ExpirableValue with the specified TTL in seconds
    ///
    /// # Arguments
    /// * `value` - The value to store
    /// * `ttl` - Time-To-Live in seconds
    ///
    /// # Returns
    /// A new ExpirableValue that will expire after the specified TTL
    pub fn new(value: Value, ttl: i64) -> Self {
        let now = Utc::now();
        let expires_at = now + Duration::seconds(ttl);
        Self { value, expires_at }
    }

    /// Checks if the value has expired
    ///
    /// # Returns
    /// * `true` if the current time is greater than or equal to `expires_at`
    /// * `false` otherwise
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    /// Gets the remaining Time-To-Live in seconds
    ///
    /// # Returns
    /// The number of seconds until expiration, or 0 if already expired
    pub fn ttl(&self) -> i64 {
        if self.is_expired() {
            0
        } else {
            (self.expires_at - Utc::now()).num_seconds().max(0)
        }
    }
}

// Implementation of builder pattern methods for ExpirableParameter
impl ExpirableParameter {
    /// Returns a new ExpirableParameterBuilder
    ///
    /// This can be used to construct an ExpirableParameter with custom options.
    pub fn builder() -> ExpirableParameterBuilder {
        ExpirableParameterBuilder::default()
    }

    /// Creates a new ExpirableParameter wrapping the specified parameter
    ///
    /// # Arguments
    /// * `parameter` - The parameter to wrap
    ///
    /// # Returns
    /// A new ExpirableParameter with default TTL settings (1 hour)
    pub fn new(parameter: ParameterType) -> Self {
        Self {
            parameter: Box::new(parameter),
            options: None,
        }
    }

    /// Creates a new ExpirableParameter with a custom TTL
    ///
    /// # Arguments
    /// * `parameter` - The parameter to wrap
    /// * `ttl` - Custom Time-To-Live in seconds
    ///
    /// # Returns
    /// A new ExpirableParameter with the specified TTL
    pub fn with_ttl(parameter: ParameterType, ttl: u64) -> Self {
        Self {
            parameter: Box::new(parameter),
            options: Some(ExpirableParameterOptions { ttl }),
        }
    }

    /// Gets the remaining Time-To-Live in seconds
    ///
    /// # Returns
    /// * `Some(i64)` with seconds until expiration
    /// * `None` if no value is set or it has already expired
    pub fn ttl(&self) -> Option<i64> {
        if let Some(ParameterValue::Expirable(exp_json)) = self.parameter.as_ref().get_value() {
            match serde_json::from_value::<ExpirableValue>(exp_json.clone()) {
                Ok(expirable) => Some(expirable.ttl()),
                Err(_) => None,
            }
        } else {
            None
        }
    }

    /// Checks if the current value has expired
    ///
    /// # Returns
    /// * `true` if there is no value, it failed to deserialize, or it has
    ///   expired
    /// * `false` if there is a valid non-expired value
    pub fn is_expired(&self) -> bool {
        if let Some(ParameterValue::Expirable(exp_json)) = self.parameter.as_ref().get_value() {
            match serde_json::from_value::<ExpirableValue>(exp_json.clone()) {
                Ok(expirable) => expirable.is_expired(),
                Err(_) => true,
            }
        } else {
            true // If no value or not Expirable, consider it expired
        }
    }

    /// Refreshes the expiration time of the current value
    ///
    /// This method updates the expiration time to be the current time plus TTL,
    /// effectively extending the lifetime of the value.
    ///
    /// # Returns
    /// * `Ok(())` if successful or if no value is set
    /// * `Err(ParameterError)` if an error occurred during the update
    pub fn refresh_ttl(&mut self) -> Result<(), ParameterError> {
        // Make a clone of the value to avoid borrowing issues
        let value_option = self.parameter.as_ref().get_value().cloned();

        if let Some(ParameterValue::Expirable(exp_json)) = value_option {
            let expirable = match serde_json::from_value::<ExpirableValue>(exp_json.clone()) {
                Ok(value) => value,
                Err(err) => {
                    return Err(ParameterError::DeserializationError {
                        key: self.metadata().key.clone(),
                        error: err.to_string(),
                    });
                }
            };

            let ttl = self
                .options
                .as_ref()
                .map(|opt| opt.ttl as i64)
                .unwrap_or(DEFAULT_TTL as i64);

            // Create a new ExpirableValue with the same value but updated expiration time
            let new_expirable = ExpirableValue::new(expirable.value.clone(), ttl);

            // Update the value
            let new_value = ParameterValue::Expirable(match serde_json::to_value(new_expirable) {
                Ok(value) => value,
                Err(err) => {
                    return Err(ParameterError::SerializationError(err));
                }
            });

            return self.parameter.as_mut().set_value(new_value);
        }

        // If no value or not Expirable, do nothing
        Ok(())
    }

    /// Gets the actual value without the expirable wrapper
    ///
    /// # Returns
    /// * `Some(Value)` with the actual value if it exists and has not expired
    /// * `None` if no value is set, it has expired, or it failed to deserialize
    pub fn get_actual_value(&self) -> Option<Value> {
        if let Some(ParameterValue::Expirable(exp_json)) = self.parameter.as_ref().get_value() {
            match serde_json::from_value::<ExpirableValue>(exp_json.clone()) {
                Ok(expirable) => {
                    if !expirable.is_expired() {
                        return Some(expirable.value.clone());
                    }
                }
                Err(_) => return None,
            }
        }
        None
    }

    /// Gets the actual value converted to a specific type
    ///
    /// # Type Parameters
    /// * `T` - The type to convert the value to, must implement
    ///   DeserializeOwned
    ///
    /// # Returns
    /// * `Some(T)` if the value exists, has not expired, and could be converted
    /// * `None` otherwise
    pub fn get_actual_value_as<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
        self.get_actual_value()
            .and_then(|value| match serde_json::from_value(value) {
                Ok(typed_value) => Some(typed_value),
                Err(_) => None,
            })
    }

    /// Gets a reference to the inner parameter
    ///
    /// # Returns
    /// A reference to the wrapped parameter
    pub fn inner(&self) -> &ParameterType {
        self.parameter.as_ref()
    }

    /// Gets a mutable reference to the inner parameter
    ///
    /// # Returns
    /// A mutable reference to the wrapped parameter
    pub fn inner_mut(&mut self) -> &mut ParameterType {
        self.parameter.as_mut()
    }
}

// Implementation of builder pattern for ExpirableParameterOptions
impl ExpirableParameterOptions {
    /// Returns a new ExpirableParameterOptionsBuilder
    ///
    /// This can be used to construct ExpirableParameterOptions with custom
    /// settings.
    pub fn builder() -> ExpirableParameterOptionsBuilder {
        ExpirableParameterOptionsBuilder::default()
    }
}
