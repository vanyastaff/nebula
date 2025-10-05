use crate::core::ParameterValue;
use crate::core::condition::ParameterCondition;
use nebula_core::ParameterKey;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Context for evaluating display conditions
#[derive(Debug, Clone)]
pub struct DisplayContext {
    /// Current values of other parameters
    pub values: HashMap<ParameterKey, ParameterValue>,
}

impl DisplayContext {
    pub fn new(values: HashMap<ParameterKey, ParameterValue>) -> Self {
        Self { values }
    }
}

/// Display configuration for parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ParameterDisplay {
    /// Conditions that must be true to show the parameter
    pub show: Option<HashMap<ParameterKey, Vec<ParameterCondition>>>,

    /// Conditions that must be false to show the parameter (if true, parameter is hidden)
    pub hide: Option<HashMap<ParameterKey, Vec<ParameterCondition>>>,
}

impl ParameterDisplay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for parameter display
    pub fn builder() -> ParameterDisplayBuilder {
        ParameterDisplayBuilder::new()
    }

    /// Check if the parameter should be displayed given the current context
    pub fn should_display(&self, properties: &HashMap<ParameterKey, ParameterValue>) -> bool {
        // Check hide conditions first - if any are met, hide the parameter
        if self.should_hide(properties) {
            return false;
        }

        // Check show conditions - all must be met to show the parameter
        self.should_show(properties)
    }

    /// Check if any hide conditions are met
    fn should_hide(&self, properties: &HashMap<ParameterKey, ParameterValue>) -> bool {
        if let Some(hide_conditions) = &self.hide {
            for (key, conditions) in hide_conditions {
                if let Some(value) = properties.get(key) {
                    // If any hide condition is true, hide the parameter
                    if conditions.iter().any(|c| c.evaluate(value)) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Check if all show conditions are met
    fn should_show(&self, properties: &HashMap<ParameterKey, ParameterValue>) -> bool {
        if let Some(show_conditions) = &self.show {
            for (key, conditions) in show_conditions {
                if let Some(value) = properties.get(key) {
                    // All show conditions must be true
                    if !conditions.iter().all(|c| c.evaluate(value)) {
                        return false;
                    }
                } else {
                    // If property doesn't exist, condition fails
                    return false;
                }
            }
        }
        true
    }

    /// Validate display conditions and return detailed error if hidden
    pub fn validate_display(
        &self,
        properties: &HashMap<ParameterKey, ParameterValue>,
    ) -> Result<(), ParameterDisplayError> {
        if !self.should_display(properties) {
            return Err(ParameterDisplayError::Hidden {
                reason: "Display conditions not met".to_string(),
            });
        }
        Ok(())
    }

    /// Check if this display configuration is empty (no conditions)
    #[inline]
    pub fn is_empty(&self) -> bool {
        let show_empty = match &self.show {
            Some(show_conditions) => show_conditions.is_empty(),
            None => true,
        };

        let hide_empty = match &self.hide {
            Some(hide_conditions) => hide_conditions.is_empty(),
            None => true,
        };

        show_empty && hide_empty
    }

    /// Get all property keys that this display depends on
    pub fn get_dependencies(&self) -> Vec<ParameterKey> {
        let mut deps = Vec::new();

        if let Some(hide) = &self.hide {
            deps.extend(hide.keys().cloned());
        }

        if let Some(show) = &self.show {
            deps.extend(show.keys().cloned());
        }

        deps.sort();
        deps.dedup();
        deps
    }

    /// Add a show condition
    pub fn add_show_condition(&mut self, property: ParameterKey, condition: ParameterCondition) {
        self.ensure_show_conditions()
            .entry(property)
            .or_insert_with(Vec::new)
            .push(condition);
    }

    /// Add a hide condition
    pub fn add_hide_condition(&mut self, property: ParameterKey, condition: ParameterCondition) {
        self.ensure_hide_conditions()
            .entry(property)
            .or_insert_with(Vec::new)
            .push(condition);
    }

    /// Ensure show conditions map exists and return a mutable reference
    fn ensure_show_conditions(&mut self) -> &mut HashMap<ParameterKey, Vec<ParameterCondition>> {
        self.show.get_or_insert_with(HashMap::new)
    }

    /// Ensure hide conditions map exists and return a mutable reference
    fn ensure_hide_conditions(&mut self) -> &mut HashMap<ParameterKey, Vec<ParameterCondition>> {
        self.hide.get_or_insert_with(HashMap::new)
    }

    /// Merge with another display configuration
    pub fn merge(mut self, other: ParameterDisplay) -> Self {
        // Merge hide conditions
        if let Some(other_hide) = other.hide {
            let hide_conditions = self.ensure_hide_conditions();
            for (key, conditions) in other_hide {
                hide_conditions
                    .entry(key)
                    .or_insert_with(Vec::new)
                    .extend(conditions);
            }
        }

        // Merge show conditions
        if let Some(other_show) = other.show {
            let show_conditions = self.ensure_show_conditions();
            for (key, conditions) in other_show {
                show_conditions
                    .entry(key)
                    .or_insert_with(Vec::new)
                    .extend(conditions);
            }
        }

        self
    }

    /// Clear all hide and show conditions
    pub fn clear_conditions(&mut self) -> &mut Self {
        self.hide = None;
        self.show = None;
        self
    }

    /// Returns a count of how many conditions are in this display
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

/// Builder for creating ParameterDisplay instances with a fluent API
#[derive(Debug, Default)]
pub struct ParameterDisplayBuilder {
    display: ParameterDisplay,
}

impl ParameterDisplayBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a hide condition
    pub fn hide_when(
        mut self,
        property_key: impl Into<ParameterKey>,
        condition: ParameterCondition,
    ) -> Self {
        self.display
            .add_hide_condition(property_key.into(), condition);
        self
    }

    /// Add a show condition
    pub fn show_when(
        mut self,
        property_key: impl Into<ParameterKey>,
        condition: ParameterCondition,
    ) -> Self {
        self.display
            .add_show_condition(property_key.into(), condition);
        self
    }

    /// Add a show condition with equality check
    pub fn show_when_equals<T: Into<ParameterValue>>(
        self,
        property_key: impl Into<ParameterKey>,
        value: T,
    ) -> Self {
        self.show_when(property_key, ParameterCondition::Eq(value.into()))
    }

    /// Add a hide condition with equality check
    pub fn hide_when_equals<T: Into<ParameterValue>>(
        self,
        property_key: impl Into<ParameterKey>,
        value: T,
    ) -> Self {
        self.hide_when(property_key, ParameterCondition::Eq(value.into()))
    }

    /// Add multiple show conditions (any of them can be true)
    pub fn show_when_any(mut self, conditions: Vec<(ParameterKey, ParameterCondition)>) -> Self {
        for (key, condition) in conditions {
            self.display.add_show_condition(key, condition);
        }
        self
    }

    /// Build the final ParameterDisplay
    pub fn build(self) -> ParameterDisplay {
        self.display
    }
}

/// Error type for display operations
#[derive(Debug, thiserror::Error)]
pub enum ParameterDisplayError {
    #[error("Parameter is hidden: {reason}")]
    Hidden { reason: String },

    #[error("Display condition evaluation failed: {reason}")]
    EvaluationError { reason: String },
}
