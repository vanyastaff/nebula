use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use crate::parameter::types::ExpirableValue;
use crate::types::Key;
use crate::{ParameterCollection, ParameterError, ParameterValue};

/// Store for parameters with change tracking
#[derive(Debug, Clone)]
pub struct ParameterStore {
    /// The underlying parameter collection
    parameters: Arc<RwLock<ParameterCollection>>,

    /// Cache of current parameter values for quick access
    value_cache: HashMap<Key, ParameterValue>,

    /// Set of parameters that have been modified
    modified_parameters: HashSet<Key>,

    /// Temporary overrides that don't modify the underlying collection
    overrides: HashMap<Key, ParameterValue>,
}

impl ParameterStore {
    /// Creates a new parameter store from a parameter collection
    pub fn new(parameters: ParameterCollection) -> Self {
        // Initialize the value cache
        let mut value_cache = HashMap::new();
        for key in parameters.keys() {
            if let Ok(param) = parameters.get(&key) {
                if let Some(value) = param.get_value() {
                    value_cache.insert(key.clone(), value.clone());
                }
            }
        }

        Self {
            parameters: Arc::new(RwLock::new(parameters)),
            value_cache,
            modified_parameters: HashSet::new(),
            overrides: HashMap::new(),
        }
    }

    /// Gets a parameter value by key
    pub fn get<T>(&self, key: &Key) -> Result<T, ParameterError>
    where
        T: for<'de> Deserialize<'de>,
    {
        // Check overrides first
        if let Some(value) = self.overrides.get(key) {
            return Self::deserialize_value(value, key);
        }

        // Then check the value cache
        if let Some(value) = self.value_cache.get(key) {
            return Self::deserialize_value(value, key);
        }

        // Not found in cache or overrides, check the actual collection
        let parameters = self.parameters.read().unwrap();
        let param = parameters.get(key)?;

        if let Some(value) = param.get_value() {
            // Add to the cache for future access
            let mut value_cache = self.value_cache.clone();
            value_cache.insert(key.clone(), value.clone());

            return Self::deserialize_value(value, key);
        }

        Err(ParameterError::NotFound(key.clone()))
    }

    /// Gets a parameter value by key string
    pub fn get_by_str<T>(&self, key_str: &str) -> Result<T, ParameterError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let key = Key::new(key_str)?;
        self.get(&key)
    }

    /// Sets a parameter value
    pub fn set<T>(&mut self, key: &Key, value: T) -> Result<(), ParameterError>
    where
        T: Serialize,
    {
        let param_value = Self::serialize_value(value)?;

        // Update the underlying parameter collection
        {
            let mut parameters = self.parameters.write().unwrap();
            let param = parameters.get_mut(key)?;
            param.set_value(param_value.clone())?;
        }

        // Update cache and mark as modified
        self.value_cache.insert(key.clone(), param_value);
        self.modified_parameters.insert(key.clone());

        // Remove any override for this key
        self.overrides.remove(key);

        Ok(())
    }

    /// Sets a parameter value by string key
    pub fn set_by_str<T>(&mut self, key_str: &str, value: T) -> Result<(), ParameterError>
    where
        T: Serialize,
    {
        let key = Key::new(key_str)?;
        self.set(&key, value)
    }

    /// Sets a temporary override that doesn't modify the underlying collection
    pub fn set_override<T>(&mut self, key: &Key, value: T) -> Result<(), ParameterError>
    where
        T: Serialize,
    {
        let param_value = Self::serialize_value(value)?;
        self.overrides.insert(key.clone(), param_value);
        Ok(())
    }

    /// Sets a temporary override by string key
    pub fn set_override_by_str<T>(&mut self, key_str: &str, value: T) -> Result<(), ParameterError>
    where
        T: Serialize,
    {
        let key = Key::new(key_str)?;
        self.set_override(&key, value)
    }

    /// Clears a temporary override
    pub fn clear_override(&mut self, key: &Key) {
        self.overrides.remove(key);
    }

    /// Clears all temporary overrides
    pub fn clear_all_overrides(&mut self) {
        self.overrides.clear();
    }

    /// Gets the set of modified parameter keys
    pub fn get_modified_parameters(&self) -> &HashSet<Key> {
        &self.modified_parameters
    }

    /// Checks if a parameter has been modified
    pub fn is_modified(&self, key: &Key) -> bool {
        self.modified_parameters.contains(key)
    }

    /// Checks if any parameters have been modified
    pub fn has_modifications(&self) -> bool {
        !self.modified_parameters.is_empty()
    }

    /// Clears the modified status of all parameters
    pub fn clear_modified_status(&mut self) {
        self.modified_parameters.clear();
    }

    /// Gets a snapshot of all parameter values
    pub fn get_values_snapshot(&self) -> HashMap<Key, ParameterValue> {
        let mut snapshot = self.value_cache.clone();

        // Apply overrides
        for (key, value) in &self.overrides {
            snapshot.insert(key.clone(), value.clone());
        }

        snapshot
    }

    /// Gets the underlying parameter collection
    pub fn get_parameter_collection(&self) -> Arc<RwLock<ParameterCollection>> {
        self.parameters.clone()
    }

    /// Helper to serialize a value to ParameterValue
    fn serialize_value<T: Serialize>(value: T) -> Result<ParameterValue, ParameterError> {
        let json_value =
            serde_json::to_value(value).map_err(|e| ParameterError::SerializationError(e))?;

        Ok(ParameterValue::Value(json_value))
    }

    /// Helper to deserialize a value from ParameterValue
    fn deserialize_value<T>(value: &ParameterValue, key: &Key) -> Result<T, ParameterError>
    where
        T: for<'de> Deserialize<'de>,
    {
        match value {
            ParameterValue::Value(json_value) => serde_json::from_value(json_value.clone())
                .map_err(|e| ParameterError::DeserializationError {
                    key: key.clone(),
                    error: format!("Failed to deserialize value: {}", e),
                }),
            ParameterValue::Expression(expr) => Err(ParameterError::DeserializationError {
                key: key.clone(),
                error: format!("Cannot deserialize from Expression type: {}", expr),
            }),
            ParameterValue::Expirable(json_value) => {
                // Try to deserialize the ExpirableValue
                let expirable: ExpirableValue = serde_json::from_value(json_value.clone())
                    .map_err(|e| ParameterError::DeserializationError {
                        key: key.clone(),
                        error: format!("Failed to deserialize ExpirableValue: {}", e),
                    })?;

                // Check if the value has expired
                if expirable.is_expired() {
                    return Err(ParameterError::DeserializationError {
                        key: key.clone(),
                        error: "Value has expired".to_string(),
                    });
                }

                // Deserialize the actual value
                serde_json::from_value(expirable.value.clone()).map_err(|e| {
                    ParameterError::DeserializationError {
                        key: key.clone(),
                        error: format!("Failed to deserialize expirable value content: {}", e),
                    }
                })
            }
            ParameterValue::Mode(mode_value) => {
                // Deserialize from the mode value
                serde_json::from_value(mode_value.value.clone()).map_err(|e| {
                    ParameterError::DeserializationError {
                        key: key.clone(),
                        error: format!("Failed to deserialize from Mode value: {}", e),
                    }
                })
            }
            ParameterValue::Group(group_value) => {
                // For group values, try to deserialize the whole group
                let json = serde_json::to_value(group_value.deref())
                    .map_err(|e| ParameterError::SerializationError(e))?;
                serde_json::from_value(json).map_err(|e| ParameterError::DeserializationError {
                    key: key.clone(),
                    error: format!("Failed to deserialize from Group value: {}", e),
                })
            }
        }
    }
}
