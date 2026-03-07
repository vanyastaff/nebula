use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::subtype::NumberSubtype;
use crate::validation::ValidationRule;

/// Options specific to number parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NumberOptions {
    /// Minimum allowed value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,

    /// Maximum allowed value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,

    /// Step increment for UI spinners.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,

    /// Number of decimal places to display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<u8>,
}

/// A numeric input parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NumberParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<f64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<NumberOptions>,

    /// Semantic subtype for this number parameter
    #[serde(default, skip_serializing_if = "is_default_number_subtype")]
    pub subtype: NumberSubtype,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl NumberParameter {
    /// Create a new number parameter with minimal fields.
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            default: None,
            options: None,
            subtype: NumberSubtype::default(),
            display: None,
            validation: Vec::new(),
        }
    }

    /// Set default value (builder-style).
    #[must_use]
    pub fn default_value(mut self, value: f64) -> Self {
        self.default = Some(value);
        self
    }

    /// Set minimum value constraint.
    #[must_use]
    pub fn min(mut self, value: f64) -> Self {
        self.options
            .get_or_insert_with(|| NumberOptions {
                min: None,
                max: None,
                step: None,
                precision: None,
            })
            .min = Some(value);
        ValidationRule::replace_in(&mut self.validation, ValidationRule::min(value));
        self
    }

    /// Set maximum value constraint.
    #[must_use]
    pub fn max(mut self, value: f64) -> Self {
        self.options
            .get_or_insert_with(|| NumberOptions {
                min: None,
                max: None,
                step: None,
                precision: None,
            })
            .max = Some(value);
        ValidationRule::replace_in(&mut self.validation, ValidationRule::max(value));
        self
    }

    /// Set both min and max constraints (range).
    #[must_use]
    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.options
            .get_or_insert_with(|| NumberOptions {
                min: None,
                max: None,
                step: None,
                precision: None,
            })
            .min = Some(min);
        if let Some(options) = self.options.as_mut() {
            options.max = Some(max);
        }
        ValidationRule::replace_in(&mut self.validation, ValidationRule::min(min));
        ValidationRule::replace_in(&mut self.validation, ValidationRule::max(max));
        self
    }

    /// Set step increment for UI spinners.
    #[must_use]
    pub fn step(mut self, value: f64) -> Self {
        self.options
            .get_or_insert_with(|| NumberOptions {
                min: None,
                max: None,
                step: None,
                precision: None,
            })
            .step = Some(value);
        self
    }

    /// Set precision (number of decimal places).
    #[must_use]
    pub fn precision(mut self, places: u8) -> Self {
        self.options
            .get_or_insert_with(|| NumberOptions {
                min: None,
                max: None,
                step: None,
                precision: None,
            })
            .precision = Some(places);
        self
    }

    /// Set semantic subtype for this number parameter.
    #[must_use]
    pub fn subtype(mut self, subtype: NumberSubtype) -> Self {
        self.subtype = subtype;

        // Auto-apply default constraints if available
        if let Some((min, max)) = subtype.default_constraints() {
            let opts = self.options.get_or_insert_with(|| NumberOptions {
                min: None,
                max: None,
                step: None,
                precision: None,
            });
            if opts.min.is_none() {
                opts.min = Some(min);
                ValidationRule::replace_in(&mut self.validation, ValidationRule::min(min));
            }
            if opts.max.is_none() {
                opts.max = Some(max);
                ValidationRule::replace_in(&mut self.validation, ValidationRule::max(max));
            }
        }

        self
    }

    /// Convenience: create a percentage parameter (0-100).
    #[must_use]
    pub fn percentage(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self::new(key, name)
            .subtype(NumberSubtype::Percentage)
            .range(0.0, 100.0)
    }

    /// Convenience: create a port number parameter (1-65535).
    #[must_use]
    pub fn port(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self::new(key, name)
            .subtype(NumberSubtype::Port)
            .range(1.0, 65535.0)
            .precision(0)
    }

    /// Convenience: create an opacity parameter (0-100).
    #[must_use]
    pub fn opacity(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self::new(key, name)
            .subtype(NumberSubtype::Opacity)
            .range(0.0, 100.0)
    }
}

// Helper for serde skip_serializing_if
fn is_default_number_subtype(subtype: &NumberSubtype) -> bool {
    *subtype == NumberSubtype::default()
}

impl crate::common::ParameterType for NumberParameter {
    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }

    fn metadata_mut(&mut self) -> &mut ParameterMetadata {
        &mut self.metadata
    }

    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn display_mut(&mut self) -> &mut Option<ParameterDisplay> {
        &mut self.display
    }

    fn validation_rules(&self) -> &[ValidationRule] {
        &self.validation
    }

    fn validation_rules_mut(&mut self) -> &mut Vec<ValidationRule> {
        &mut self.validation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_minimal_number() {
        let p = NumberParameter::new("port", "Port");
        assert_eq!(p.metadata.key, "port");
        assert!(p.default.is_none());
    }

    #[test]
    fn serde_round_trip() {
        let p = NumberParameter {
            metadata: ParameterMetadata::new("timeout", "Timeout (s)"),
            default: Some(30.0),
            options: Some(NumberOptions {
                min: Some(1.0),
                max: Some(300.0),
                step: Some(1.0),
                precision: Some(0),
            }),
            subtype: NumberSubtype::None,
            display: None,
            validation: vec![ValidationRule::min(1.0), ValidationRule::max(300.0)],
        };

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: NumberParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "timeout");
        assert_eq!(deserialized.default, Some(30.0));
        assert_eq!(deserialized.validation.len(), 2);
    }

    #[test]
    fn range_replaces_existing_min_and_max_rules() {
        let parameter = NumberParameter::new("port", "Port")
            .min(1.0)
            .max(10.0)
            .range(2.0, 20.0);

        assert_eq!(parameter.validation.len(), 2);
        assert!(parameter.validation.contains(&ValidationRule::min(2.0)));
        assert!(parameter.validation.contains(&ValidationRule::max(20.0)));
    }
}
