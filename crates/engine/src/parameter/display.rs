use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::parameter::condition::ParameterCondition;
use crate::types::Key;

/// Errors that can occur during parameter display evaluation
#[derive(Debug, Error, Clone, PartialEq)]
pub enum ParameterDisplayError {
    #[error("Required property '{0}' is missing")]
    MissingProperty(Key),

    #[error("Property '{property}' failed condition check: {message}")]
    ConditionFailed { property: Key, message: String },

    #[error("Invalid parameter display configuration: {0}")]
    InvalidConfiguration(String),
}

/// A structure representing the display settings for a parameter.
/// It holds collections of conditions (hide and show) keyed by property name.
/// If a property matches a hide condition, the parameter should not be
/// displayed; if properties do not satisfy any of the show conditions, the
/// parameter is hidden. If neither are specified, the parameter is displayed by
/// default.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ParameterDisplay {
    /// A mapping from property keys to a list of conditions that hide the
    /// parameter.
    #[serde(skip_serializing_if = "Option::is_none")]
    hide: Option<HashMap<Key, Vec<ParameterCondition>>>,

    /// A mapping from property keys to a list of conditions that show the
    /// parameter.
    #[serde(skip_serializing_if = "Option::is_none")]
    show: Option<HashMap<Key, Vec<ParameterCondition>>>,
}

impl ParameterDisplay {
    /// Creates a new ParameterDisplay with no conditions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new ParameterDisplay builder.
    pub fn builder() -> ParameterDisplayBuilder {
        ParameterDisplayBuilder::new()
    }

    /// Determines whether the parameter should be displayed based on the
    /// provided properties.
    ///
    /// # Arguments
    ///
    /// * `properties` - A HashMap mapping property keys to JSON values used for
    ///   condition evaluation.
    ///
    /// # Returns
    ///
    /// * `true` if the parameter should be displayed;
    /// * `false` otherwise.
    pub fn should_display(&self, properties: &HashMap<Key, Value>) -> bool {
        self.validate_display(properties).is_ok()
    }

    /// Validates the display conditions and returns detailed error information
    /// if the parameter should not be displayed.
    ///
    /// # Arguments
    ///
    /// * `properties` - A HashMap mapping property keys to JSON values used for
    ///   condition evaluation.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the parameter should be displayed;
    /// * `Err(ParameterDisplayError)` with details about why the parameter
    ///   should not be displayed.
    pub fn validate_display(
        &self,
        properties: &HashMap<Key, Value>,
    ) -> Result<(), ParameterDisplayError> {
        // If neither hide nor show conditions are set, display by default.
        if self.hide.is_none() && self.show.is_none() {
            return Ok(());
        }

        // Evaluate hide conditions: if any condition is satisfied, do not display.
        if let Some(hide_conditions) = &self.hide {
            for (property_key, conditions) in hide_conditions {
                if let Some(property_value) = properties.get(property_key) {
                    for cond in conditions {
                        if cond.check(property_value).is_ok() {
                            return Err(ParameterDisplayError::ConditionFailed {
                                property: property_key.clone(),
                                message: format!(
                                    "Hide condition '{}' was satisfied",
                                    cond.as_ref()
                                ),
                            });
                        }
                    }
                }
            }
        }

        // Evaluate show conditions: all specified properties must satisfy at least one
        // condition.
        if let Some(show_conditions) = &self.show {
            for (property_key, conditions) in show_conditions {
                if let Some(property_value) = properties.get(property_key) {
                    // If none of the conditions are satisfied, hide the parameter.
                    if !conditions
                        .iter()
                        .any(|cond| cond.check(property_value).is_ok())
                    {
                        return Err(ParameterDisplayError::ConditionFailed {
                            property: property_key.clone(),
                            message: format!(
                                "None of the {} show conditions were satisfied",
                                conditions.len()
                            ),
                        });
                    }
                } else {
                    // Missing property in the provided map means the condition fails.
                    return Err(ParameterDisplayError::MissingProperty(property_key.clone()));
                }
            }
        }

        Ok(())
    }

    /// Adds a new hide condition for the given property key.
    ///
    /// # Arguments
    ///
    /// * `property_key` - The key of the property used for condition
    ///   evaluation.
    /// * `condition` - The condition to add.
    ///
    /// # Returns
    ///
    /// * `&mut Self` for method chaining.
    pub fn add_hide_condition(
        &mut self,
        property_key: Key,
        condition: ParameterCondition,
    ) -> &mut Self {
        let hide_conditions = self.hide.get_or_insert_with(HashMap::new);
        hide_conditions
            .entry(property_key.into())
            .or_insert_with(Vec::new)
            .push(condition);
        self
    }

    /// Adds a new show condition for the given property key.
    ///
    /// # Arguments
    ///
    /// * `property_key` - The key of the property used for condition
    ///   evaluation.
    /// * `condition` - The condition to add.
    ///
    /// # Returns
    ///
    /// * `&mut Self` for method chaining.
    pub fn add_show_condition(
        &mut self,
        property_key: Key,
        condition: ParameterCondition,
    ) -> &mut Self {
        let show_conditions = self.show.get_or_insert_with(HashMap::new);
        show_conditions
            .entry(property_key.into())
            .or_insert_with(Vec::new)
            .push(condition);
        self
    }

    /// Clears all hide and show conditions.
    ///
    /// # Returns
    ///
    /// * `&mut Self` for method chaining.
    pub fn clear_conditions(&mut self) -> &mut Self {
        self.hide = None;
        self.show = None;
        self
    }

    /// Checks if the display has any conditions (hide or show).
    ///
    /// # Returns
    ///
    /// * `true` if no conditions are set, `false` otherwise.
    pub fn is_empty(&self) -> bool {
        self.hide.is_none() && self.show.is_none()
    }

    /// Returns a count of how many conditions are in this display.
    ///
    /// # Returns
    ///
    /// * The total number of conditions
    pub fn condition_count(&self) -> usize {
        let hide_count = self
            .hide
            .as_ref()
            .map(|h| h.values().map(|v| v.len()).sum())
            .unwrap_or(0);

        let show_count = self
            .show
            .as_ref()
            .map(|s| s.values().map(|v| v.len()).sum())
            .unwrap_or(0);

        hide_count + show_count
    }
}

/// A builder for creating ParameterDisplay instances with a fluent API.
#[derive(Debug, Default)]
pub struct ParameterDisplayBuilder {
    display: ParameterDisplay,
}

impl ParameterDisplayBuilder {
    /// Creates a new ParameterDisplayBuilder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a condition to hide the parameter when the property satisfies it.
    ///
    /// # Arguments
    ///
    /// * `property_key` - The key of the property to check.
    /// * `condition` - The condition to evaluate.
    ///
    /// # Returns
    ///
    /// * `Self` for method chaining.
    pub fn hide_when(mut self, property_key: Key, condition: ParameterCondition) -> Self {
        self.display.add_hide_condition(property_key, condition);
        self
    }

    /// Adds a condition to show the parameter when the property satisfies it.
    ///
    /// # Arguments
    ///
    /// * `property_key` - The key of the property to check.
    /// * `condition` - The condition to evaluate.
    ///
    /// # Returns
    ///
    /// * `Self` for method chaining.
    pub fn show_when(mut self, property_key: Key, condition: ParameterCondition) -> Self {
        self.display.add_show_condition(property_key, condition);
        self
    }

    /// Shows the parameter only when the property equals the specified value.
    ///
    /// # Arguments
    ///
    /// * `property_key` - The key of the property to check.
    /// * `value` - The value to compare with.
    ///
    /// # Returns
    ///
    /// * `Self` for method chaining.
    pub fn show_when_equals<T: Into<Value>>(self, property_key: Key, value: T) -> Self {
        self.show_when(property_key, ParameterCondition::Eq(value.into()))
    }

    /// Hides the parameter when the property equals the specified value.
    ///
    /// # Arguments
    ///
    /// * `property_key` - The key of the property to check.
    /// * `value` - The value to compare with.
    ///
    /// # Returns
    ///
    /// * `Self` for method chaining.
    pub fn hide_when_equals<T: Into<Value>>(self, property_key: Key, value: T) -> Self {
        self.hide_when(property_key, ParameterCondition::Eq(value.into()))
    }

    /// Shows the parameter when any of the provided property-condition pairs
    /// are satisfied.
    ///
    /// # Arguments
    ///
    /// * `conditions` - A vector of (property_key, condition) pairs.
    ///
    /// # Returns
    ///
    /// * `Self` for method chaining.
    pub fn show_when_any(mut self, conditions: Vec<(Key, ParameterCondition)>) -> Self {
        for (key, cond) in conditions {
            self.display.add_show_condition(key, cond);
        }
        self
    }

    /// Builds the final ParameterDisplay instance.
    ///
    /// # Returns
    ///
    /// * The configured ParameterDisplay.
    pub fn build(self) -> ParameterDisplay {
        self.display
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_empty_display() {
        let display = ParameterDisplay::new();
        let properties = HashMap::new();

        // Empty display should always return true
        assert!(display.should_display(&properties));
        assert!(display.is_empty());
        assert_eq!(display.condition_count(), 0);
    }

    #[test]
    fn test_hide_conditions() {
        let mut display = ParameterDisplay::new();
        display.add_hide_condition(
            "type".try_into().unwrap(),
            ParameterCondition::Eq(json!("secret")),
        );

        let mut properties = HashMap::new();

        // No properties, should display (hide condition not triggered)
        assert!(display.should_display(&properties));

        // Property doesn't match condition, should display
        properties.insert("type".try_into().unwrap(), json!("normal"));
        assert!(display.should_display(&properties));

        // Property matches condition, should not display
        properties.insert("type".try_into().unwrap(), json!("secret"));
        assert!(!display.should_display(&properties));

        // Validate that we get the correct error
        let result = display.validate_display(&properties);
        assert!(matches!(
            result,
            Err(ParameterDisplayError::ConditionFailed { property, .. }) if property == "type"
        ));
    }

    #[test]
    fn test_show_conditions() {
        let mut display = ParameterDisplay::new();
        display.add_show_condition(
            "level".try_into().unwrap(),
            ParameterCondition::Gte(json!(3)),
        );

        let mut properties = HashMap::new();

        // No properties, missing required property
        assert!(!display.should_display(&properties));
        let result = display.validate_display(&properties);
        assert!(matches!(
            result,
            Err(ParameterDisplayError::MissingProperty(prop)) if prop == "level"
        ));

        // Property doesn't satisfy condition, should not display
        properties.insert("level".try_into().unwrap(), json!(2));
        assert!(!display.should_display(&properties));

        // Property satisfies condition, should display
        properties.insert("level".try_into().unwrap(), json!(5));
        assert!(display.should_display(&properties));
    }

    #[test]
    fn test_complex_conditions() {
        // Create a display that shows when:
        // - "mode" is "advanced" AND
        // - "level" is >= 5 OR "admin" is true
        let display = ParameterDisplay::builder()
            .show_when_equals("mode".try_into().unwrap(), "advanced")
            .show_when(
                "level".try_into().unwrap(),
                ParameterCondition::Gte(json!(5)),
            )
            .show_when(
                "admin".try_into().unwrap(),
                ParameterCondition::Eq(json!(true)),
            )
            .build();

        let mut props = HashMap::new();
        props.insert("mode".to_string().try_into().unwrap(), json!("advanced"));
        props.insert("level".to_string().try_into().unwrap(), json!(3));
        props.insert("admin".to_string().try_into().unwrap(), json!(false));

        // All required properties exist, but conditions not satisfied
        assert!(!display.should_display(&props));

        // Make level >= 5, should now display
        props.insert("level".try_into().unwrap(), json!(5));
        assert!(display.should_display(&props));

        // Reset level but make admin true, should display
        props.insert("level".try_into().unwrap(), json!(3));
        props.insert("admin".try_into().unwrap(), json!(true));
        assert!(display.should_display(&props));

        // Wrong mode, should not display regardless of other properties
        props.insert("mode".try_into().unwrap(), json!("basic"));
        assert!(!display.should_display(&props));
    }

    #[test]
    fn test_builder_pattern() {
        let display = ParameterDisplay::builder()
            .hide_when(
                "debug".try_into().unwrap(),
                ParameterCondition::Eq(json!(false)),
            )
            .show_when(
                "user_type".try_into().unwrap(),
                ParameterCondition::Eq(json!("admin")),
            )
            .build();

        assert_eq!(display.condition_count(), 2);
        assert!(!display.is_empty());

        // Test hide_when_equals and show_when_equals
        let display = ParameterDisplay::builder()
            .hide_when_equals("visible".try_into().unwrap(), false)
            .show_when_equals("role".try_into().unwrap(), "developer")
            .build();

        assert_eq!(display.condition_count(), 2);
    }

    #[test]
    fn test_method_chaining() {
        let mut display = ParameterDisplay::new();
        display
            .add_hide_condition(
                "debug".try_into().unwrap(),
                ParameterCondition::Eq(json!(false)),
            )
            .add_show_condition(
                "level".try_into().unwrap(),
                ParameterCondition::Gte(json!(3)),
            )
            .add_show_condition(
                "mode".try_into().unwrap(),
                ParameterCondition::Eq(json!("advanced")),
            );

        assert_eq!(display.condition_count(), 3);

        // Clear conditions and add new ones
        display.clear_conditions().add_hide_condition(
            "test".try_into().unwrap(),
            ParameterCondition::Eq(json!(true)),
        );

        assert_eq!(display.condition_count(), 1);
    }
}
