//! Generic number parameter with trait-based subtypes.

use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::subtype::traits::{NumberSubtype, Numeric};
use crate::validation::ValidationRule;

/// Options for number parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(bound(
    serialize = "T: Serialize",
    deserialize = "T: serde::de::DeserializeOwned"
))]
pub struct NumberOptions<T: Numeric = f64> {
    /// Minimum allowed value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<T>,
    /// Maximum allowed value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<T>,
    /// Suggested increment step for UI controls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<T>,
    /// Number of decimal places suggested for UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<u8>,
}

impl<T: Numeric> Default for NumberOptions<T> {
    fn default() -> Self {
        Self {
            min: None,
            max: None,
            step: None,
            precision: None,
        }
    }
}

/// Generic number parameter with type-safe subtype.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    serialize = "S: Serialize, S::Value: Serialize",
    deserialize = "S: serde::de::DeserializeOwned, S::Value: serde::de::DeserializeOwned"
))]
pub struct Number<S: NumberSubtype> {
    /// Common parameter metadata (`key`, `name`, `description`, flags).
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default numeric value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<S::Value>,

    /// Number-specific options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<NumberOptions<S::Value>>,

    /// Semantic subtype.
    #[serde(rename = "subtype")]
    pub subtype: S,

    /// UI display rules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules applied to this parameter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl<S: NumberSubtype> Number<S> {
    /// Creates a new number parameter.
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            default: None,
            options: None,
            subtype: S::default(),
            display: None,
            validation: Vec::new(),
        }
    }

    /// Creates a builder.
    #[must_use]
    pub fn builder(key: impl Into<String>) -> NumberBuilder<S> {
        NumberBuilder::new(key)
    }

    /// Returns the subtype.
    pub fn subtype(&self) -> &S {
        &self.subtype
    }
}

/// Builder for generic number parameters.
#[derive(Debug, Clone)]
pub struct NumberBuilder<S: NumberSubtype> {
    key: String,
    name: Option<String>,
    description: Option<String>,
    default: Option<S::Value>,
    options: NumberOptions<S::Value>,
    subtype: S,
    required: bool,
    validation: Vec<ValidationRule>,
}

impl<S: NumberSubtype> NumberBuilder<S> {
    /// Creates a new builder.
    pub fn new(key: impl Into<String>) -> Self {
        let subtype = S::default();
        let mut builder = Self {
            key: key.into(),
            name: None,
            description: None,
            default: None,
            options: NumberOptions::default(),
            subtype,
            required: false,
            validation: Vec::new(),
        };

        if S::Value::is_integer() {
            builder.options.precision = Some(0);
        }

        if let Some((min, max)) = S::default_range() {
            builder.options.min = Some(min);
            builder.options.max = Some(max);
            ValidationRule::replace_in(&mut builder.validation, ValidationRule::min(min.to_f64()));
            ValidationRule::replace_in(&mut builder.validation, ValidationRule::max(max.to_f64()));
        }

        if let Some(step) = S::default_step() {
            builder.options.step = Some(step);
        }

        builder
    }

    /// Sets the display label.
    #[must_use]
    pub fn label(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the description.
    #[must_use]
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Sets the default value.
    #[must_use]
    pub fn default_value(mut self, value: S::Value) -> Self {
        self.default = Some(value);
        self
    }

    /// Marks as required.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Sets minimum value.
    #[must_use]
    pub fn min(mut self, value: S::Value) -> Self {
        self.options.min = Some(value);
        ValidationRule::replace_in(&mut self.validation, ValidationRule::min(value.to_f64()));
        self
    }

    /// Sets maximum value.
    #[must_use]
    pub fn max(mut self, value: S::Value) -> Self {
        self.options.max = Some(value);
        ValidationRule::replace_in(&mut self.validation, ValidationRule::max(value.to_f64()));
        self
    }

    /// Sets step for UI.
    #[must_use]
    pub fn step(mut self, value: S::Value) -> Self {
        self.options.step = Some(value);
        self
    }

    /// Sets decimal precision.
    #[must_use]
    pub fn precision(mut self, value: u8) -> Self {
        self.options.precision = Some(value);
        self
    }

    /// Builds the parameter.
    pub fn build(self) -> Number<S> {
        let key = self.key;
        let name = self.name.unwrap_or_else(|| key.clone());
        let mut metadata = ParameterMetadata::new(key, name);
        metadata.description = self.description;
        metadata.required = self.required;

        Number {
            metadata,
            default: self.default,
            options: has_number_options(&self.options).then_some(self.options),
            subtype: self.subtype,
            display: None,
            validation: self.validation,
        }
    }
}

fn has_number_options<T: Numeric>(options: &NumberOptions<T>) -> bool {
    options.min.is_some()
        || options.max.is_some()
        || options.step.is_some()
        || options.precision.is_some()
}

impl<S: NumberSubtype> crate::common::ParameterType for Number<S> {
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
    use crate::def::ParameterDef;
    use crate::subtype::std_subtypes::Port;

    #[test]
    fn min_and_max_replace_existing_rules() {
        let number = Number::<Port>::builder("port")
            .min(10)
            .max(20)
            .min(100)
            .max(200)
            .build();

        assert_eq!(number.validation.len(), 2);
        assert!(number.validation.contains(&ValidationRule::min(100.0)));
        assert!(number.validation.contains(&ValidationRule::max(200.0)));
    }

    #[test]
    fn integer_subtypes_use_integer_storage() {
        let number = Number::<Port>::builder("port").default_value(8080).build();

        assert_eq!(number.default, Some(8080));

        let options = number
            .options
            .expect("integer subtypes should expose options");
        assert_eq!(options.min, Some(1));
        assert_eq!(options.max, Some(65535));
        assert_eq!(options.step, Some(1));
        assert_eq!(options.precision, Some(0));
    }

    #[test]
    fn typed_number_converts_to_legacy_number_definition() {
        let def: ParameterDef = Number::<Port>::builder("port")
            .label("Port")
            .default_value(5432)
            .build()
            .into();

        let ParameterDef::Number(port) = def else {
            panic!("expected number parameter");
        };

        assert_eq!(port.default, Some(5432.0));
        assert_eq!(port.options.and_then(|options| options.step), Some(1.0));
    }
}
