// Файл: parameter/types/group.rs
use derive_builder::Builder;
use derive_more::{Deref, DerefMut};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::parameter::Parameter;
use crate::parameter::collection::ParameterCollection;
use crate::parameter::display::ParameterDisplay;
use crate::parameter::error::ParameterError;
use crate::parameter::metadata::ParameterMetadata;
use crate::parameter::value::ParameterValue;

/// Parameter for grouping related parameters into a single logical unit.
///
/// `GroupParameter` allows multiple parameters to be bundled together to
/// represent a complex data structure, such as an address (with street, city,
/// zip), a person (with name, age, email), or any other composite entity.
///
/// The group parameter itself can have a value, which is a JSON object
/// containing values for each child parameter. When a value is set on the group
/// parameter, it distributes the values to the appropriate child parameters.
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
#[builder(setter(strip_option))]
pub struct GroupParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<ParameterValue>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<GroupParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Collection of parameters in this group
    pub parameters: ParameterCollection,
}

impl GroupParameter {
    /// Creates a new builder for the GroupParameter
    pub fn builder() -> GroupParameterBuilder {
        GroupParameterBuilder::default()
    }

    /// Collects values from all child parameters and creates a JSON object.
    ///
    /// This method gathers the values from all child parameters in the
    /// collection and assembles them into a single JSON object, where each
    /// key corresponds to a parameter key and each value is the parameter's
    /// value.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(ParameterValue))` - A parameter value containing a JSON
    ///   object with all the child parameter values.
    /// * `Ok(None)` - If none of the child parameters have values set.
    /// * `Err(ParameterError)` - If there was an error retrieving values from
    ///   child parameters.
    pub fn collect_values(&self) -> Result<Option<ParameterValue>, ParameterError> {
        let mut object = Map::new();
        let mut has_values = false;

        // Iterate through all parameters in the collection
        for key in self.parameters.keys() {
            // Get the value if present
            if let Some(value) = self.parameters.get(&key)?.get_value() {
                if let ParameterValue::Value(json_value) = value {
                    // Add the value to our object using the key's string representation
                    object.insert(key.to_string(), json_value.clone());
                    has_values = true;
                }
            }
        }

        if has_values {
            // Create a parameter value from the collected values
            Ok(Some(ParameterValue::Value(Value::Object(object))))
        } else {
            Ok(None)
        }
    }
}

/// Configuration options for a group parameter.
///
/// These options control the behavior of a group parameter, particularly
/// with respect to whether it can have multiple instances (e.g., multiple
/// addresses, multiple phone numbers, etc.).
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
#[builder(setter(strip_option))]
pub struct GroupParameterOptions {
    /// Whether this group can be cloned to create multiple instances
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_multiple: Option<bool>,

    /// Minimum number of instances required (when allow_multiple is true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_instances: Option<usize>,

    /// Maximum number of instances allowed (when allow_multiple is true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_instances: Option<usize>,
}

impl Parameter for GroupParameter {
    /// Returns the metadata for this group parameter.
    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }

    /// Gets the value of this group parameter.
    ///
    /// The value of a group parameter is typically a JSON object containing
    /// values for each child parameter.
    fn get_value(&self) -> Option<&ParameterValue> {
        self.value.as_ref()
    }

    /// Sets the value of this group parameter.
    ///
    /// When a value is set on a group parameter, it distributes the values to
    /// the appropriate child parameters. The value must be a JSON object where
    /// each key corresponds to a child parameter key.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to set must be a JSON object
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If the value was successfully set and distributed to child
    ///   parameters
    /// * `Err(ParameterError)` - If the value is not a JSON object or there was
    ///   an error setting values on child parameters
    fn set_value(&mut self, value: ParameterValue) -> Result<(), ParameterError> {
        match &value {
            ParameterValue::Group(group) => {
                // Distribute values to child parameters
                for (key_str, val) in &group.value {
                    // Try to find a parameter with this key
                    let key = crate::types::Key::new(key_str)?;
                    if self.parameters.contains_key(&key) {
                        // Set the value on the child parameter
                        self.parameters
                            .set_value(&key, ParameterValue::Value(val.clone()))?;
                    }
                }
                // Store the original value as well
                self.value = Some(value);
                Ok(())
            }
            ParameterValue::Value(Value::Object(object)) => {
                // Create a GroupValue from the object and recursively call set_value
                let group_value = GroupValue::from(object.clone());
                self.set_value(ParameterValue::Group(group_value))
            }
            _ => {
                // If it's not a group or object, we can't distribute it
                Err(ParameterError::InvalidType {
                    key: self.metadata().key.clone(),
                    expected_type: "Group or Object".to_string(),
                    actual_details: "GroupParameter requires a Group value or Object value to \
                                     distribute to child parameters"
                        .to_string(),
                })
            }
        }
    }

    /// Gets the display settings for this group parameter.
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }
}

// GroupValue struct with Deref and DerefMut already implemented via derive_more
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, Deref, DerefMut)]
#[serde(rename_all = "lowercase")]
pub struct GroupValue {
    pub value: Map<String, Value>,
}

impl GroupValue {
    pub fn new() -> Self {
        Self { value: Map::new() }
    }
}

impl From<Map<String, Value>> for GroupValue {
    fn from(value: Map<String, Value>) -> Self {
        Self { value }
    }
}
