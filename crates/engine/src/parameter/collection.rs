use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::{Key, KeyParseError};
use crate::{Parameter, ParameterError, ParameterType, ParameterValue};

/// Errors that can occur when working with the parameter collection
#[derive(Debug, Error)]
pub enum ParameterCollectionError {
    /// A parameter with the same key already exists in the collection
    #[error("Parameter with a key '{0}' already exists")]
    DuplicateKey(Key),

    /// The requested parameter was not found in the collection
    #[error("Parameter with a key '{0}' not found")]
    NotFound(Key),

    /// Error occurred while parsing or creating a key
    #[error("Key error: {0}")]
    KeyError(#[from] KeyParseError),

    /// Error occurred within a parameter's implementation
    #[error("Parameter error: {0}")]
    ParameterError(#[from] ParameterError),

    /// Type error when trying to downcast a parameter
    #[error("Type error for a key '{key}': expected {expected}")]
    TypeError { key: Key, expected: String },
}

/// A collection of parameters accessible by key
///
/// `ParameterCollection` provides a central storage for all parameters in the
/// system. Parameters are stored with their normalized keys for easy access.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ParameterCollection {
    /// The map of parameter keys to their references
    parameters: HashMap<Key, ParameterType>,
}

impl ParameterCollection {
    /// Creates a new empty parameter collection
    pub fn new() -> Self {
        Self {
            parameters: HashMap::new(),
        }
    }

    /// Adds a parameter to the collection
    ///
    /// # Arguments
    ///
    /// * `parameter` - The parameter to add
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the parameter was successfully added
    /// * `Err(ParameterCollectionError::DuplicateKey)` if a parameter with the
    ///   same key already exists
    pub fn add(&mut self, parameter: ParameterType) -> Result<(), ParameterCollectionError> {
        let key = parameter.metadata().key.clone();

        if self.parameters.contains_key(&key) {
            return Err(ParameterCollectionError::DuplicateKey(key));
        }

        self.parameters.insert(key, parameter);
        Ok(())
    }

    /// Gets a reference to a parameter by its key
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the parameter to retrieve
    ///
    /// # Returns
    ///
    /// * `Ok(&dyn Parameter)` if the parameter was found
    /// * `Err(ParameterCollectionError::NotFound)` if the parameter was not
    ///   found
    pub fn get(&self, key: &Key) -> Result<&dyn Parameter, ParameterCollectionError> {
        self.parameters
            .get(key)
            .map(|param| param as &dyn Parameter)
            .ok_or_else(|| ParameterCollectionError::NotFound(key.clone()))
    }

    /// Gets a mutable reference to a parameter by its key
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the parameter to retrieve
    ///
    /// # Returns
    ///
    /// * `Ok(&mut dyn Parameter)` if the parameter was found
    /// * `Err(ParameterCollectionError::NotFound)` if the parameter was not
    ///   found
    pub fn get_mut(&mut self, key: &Key) -> Result<&mut dyn Parameter, ParameterCollectionError> {
        self.parameters
            .get_mut(key)
            .map(|param| param as &mut dyn Parameter)
            .ok_or_else(|| ParameterCollectionError::NotFound(key.clone()))
    }

    /// Gets a specific parameter type by its key
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the parameter to retrieve
    ///
    /// # Returns
    ///
    /// * `Ok(&ParameterType)` if the parameter was found
    /// * `Err(ParameterCollectionError::NotFound)` if the parameter was not
    ///   found
    pub fn get_param_type(&self, key: &Key) -> Result<&ParameterType, ParameterCollectionError> {
        self.parameters
            .get(key)
            .ok_or_else(|| ParameterCollectionError::NotFound(key.clone()))
    }

    /// Gets a mutable reference to a specific parameter type by its key
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the parameter to retrieve
    ///
    /// # Returns
    ///
    /// * `Ok(&mut ParameterType)` if the parameter was found
    /// * `Err(ParameterCollectionError::NotFound)` if the parameter was not
    ///   found
    pub fn get_param_type_mut(
        &mut self,
        key: &Key,
    ) -> Result<&mut ParameterType, ParameterCollectionError> {
        self.parameters
            .get_mut(key)
            .ok_or_else(|| ParameterCollectionError::NotFound(key.clone()))
    }

    /// Gets a typed reference to a parameter with downcasting
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the parameter to retrieve
    ///
    /// # Returns
    ///
    /// * `Ok(&P)` if the parameter was found and is of the requested type
    /// * `Err(ParameterCollectionError::NotFound)` if the parameter was not
    ///   found
    /// * `Err(ParameterCollectionError::TypeError)` if the parameter is not of
    ///   the requested type
    pub fn get_as<P: Parameter + 'static>(
        &self,
        key: &Key,
    ) -> Result<&P, ParameterCollectionError> {
        let param = self.get(key)?;

        param
            .downcast_ref::<P>()
            .ok_or_else(|| ParameterCollectionError::TypeError {
                key: key.clone(),
                expected: std::any::type_name::<P>().to_string(),
            })
    }

    /// Gets a typed mutable reference to a parameter with downcasting
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the parameter to retrieve
    ///
    /// # Returns
    ///
    /// * `Ok(&mut P)` if the parameter was found and is of the requested type
    /// * `Err(ParameterCollectionError::NotFound)` if the parameter was not
    ///   found
    /// * `Err(ParameterCollectionError::TypeError)` if the parameter is not of
    ///   the requested type
    pub fn get_as_mut<P: Parameter + 'static>(
        &mut self,
        key: &Key,
    ) -> Result<&mut P, ParameterCollectionError> {
        let param = self.get_mut(key)?;

        param
            .downcast_mut::<P>()
            .ok_or_else(|| ParameterCollectionError::TypeError {
                key: key.clone(),
                expected: std::any::type_name::<P>().to_string(),
            })
    }

    /// Gets a reference to a parameter by string key, converting it to a Key
    ///
    /// # Arguments
    ///
    /// * `key_str` - The string key of the parameter to retrieve
    ///
    /// # Returns
    ///
    /// * `Ok(&dyn Parameter)` if the parameter was found
    /// * `Err(ParameterCollectionError::KeyError)` if the key is invalid
    /// * `Err(ParameterCollectionError::NotFound)` if the parameter was not
    ///   found
    pub fn get_by_str(&self, key_str: &str) -> Result<&dyn Parameter, ParameterCollectionError> {
        let key = Key::new(key_str)?;
        self.get(&key)
    }

    /// Gets a mutable reference to a parameter by string key, converting it to
    /// a Key
    ///
    /// # Arguments
    ///
    /// * `key_str` - The string key of the parameter to retrieve
    ///
    /// # Returns
    ///
    /// * `Ok(&mut dyn Parameter)` if the parameter was found
    /// * `Err(ParameterCollectionError::KeyError)` if the key is invalid
    /// * `Err(ParameterCollectionError::NotFound)` if the parameter was not
    ///   found
    pub fn get_mut_by_str(
        &mut self,
        key_str: &str,
    ) -> Result<&mut dyn Parameter, ParameterCollectionError> {
        let key = Key::new(key_str)?;
        self.get_mut(&key)
    }

    /// Gets the value of a parameter
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the parameter
    ///
    /// # Returns
    ///
    /// * `Ok(Option<ParameterValue>)` - The parameter value (Option since a
    ///   parameter may not have a value)
    /// * `Err(ParameterCollectionError)` - Error getting the parameter
    pub fn get_value(&self, key: &Key) -> Result<Option<ParameterValue>, ParameterCollectionError> {
        let param = self.get(key)?;

        // Clone the parameter value if it exists
        let value = param.get_value().cloned();
        Ok(value)
    }

    /// Sets the value of a parameter
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the parameter
    /// * `value` - The new value to set
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the value was set successfully
    /// * `Err(ParameterCollectionError)` if an error occurred
    pub fn set_value(
        &mut self,
        key: &Key,
        value: ParameterValue,
    ) -> Result<(), ParameterCollectionError> {
        let param = self.get_mut(key)?;
        param.set_value(value)?;
        Ok(())
    }

    /// Removes a parameter from the collection
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the parameter to remove
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the parameter was removed
    /// * `Err(ParameterCollectionError::NotFound)` if the parameter was not
    ///   found
    pub fn remove(&mut self, key: &Key) -> Result<(), ParameterCollectionError> {
        if self.parameters.remove(key).is_none() {
            return Err(ParameterCollectionError::NotFound(key.clone()));
        }
        Ok(())
    }

    /// Checks if the collection contains a parameter with the given key
    ///
    /// # Arguments
    ///
    /// * `key` - The key to check
    ///
    /// # Returns
    ///
    /// * `true` if the parameter exists, otherwise `false`
    pub fn contains_key(&self, key: &Key) -> bool {
        self.parameters.contains_key(key)
    }

    /// Checks if the collection contains a parameter with the given string key
    ///
    /// # Arguments
    ///
    /// * `key_str` - The string key to check
    ///
    /// # Returns
    ///
    /// * `Ok(bool)` indicating if the parameter exists
    /// * `Err(ParameterCollectionError::KeyError)` if the key is invalid
    pub fn contains_key_str(&self, key_str: &str) -> Result<bool, ParameterCollectionError> {
        let key = Key::new(key_str)?;
        Ok(self.contains_key(&key))
    }

    /// Returns the number of parameters in the collection
    pub fn len(&self) -> usize {
        self.parameters.len()
    }

    /// Checks if the collection is empty
    pub fn is_empty(&self) -> bool {
        self.parameters.is_empty()
    }

    /// Returns all parameter keys in the collection
    pub fn keys(&self) -> Vec<Key> {
        self.parameters.keys().cloned().collect()
    }

    /// Applies a function to each parameter in the collection
    ///
    /// # Arguments
    ///
    /// * `f` - The function to apply to each (key, parameter) pair
    pub fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(&Key, &dyn Parameter),
    {
        for (key, param) in &self.parameters {
            f(key, param as &dyn Parameter);
        }
    }

    /// Applies a mutable function to each parameter in the collection
    ///
    /// # Arguments
    ///
    /// * `f` - The function to apply to each (key, parameter) pair
    pub fn for_each_mut<F>(&mut self, mut f: F)
    where
        F: FnMut(&Key, &mut dyn Parameter),
    {
        for (key, param) in &mut self.parameters {
            f(key, param as &mut dyn Parameter);
        }
    }

    /// Creates a snapshot of all parameter values
    ///
    /// # Returns
    ///
    /// * A map of parameter keys to their current values
    pub fn snapshot(&self) -> HashMap<Key, Option<ParameterValue>> {
        let mut result = HashMap::new();

        for (key, param) in &self.parameters {
            result.insert(key.clone(), param.get_value().cloned());
        }

        result
    }

    /// Loads parameter values from a snapshot
    ///
    /// # Arguments
    ///
    /// * `snapshot` - The snapshot to load values from
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the snapshot was loaded successfully
    /// * `Err(ParameterCollectionError)` if an error occurred
    pub fn load_snapshot(
        &mut self,
        snapshot: &HashMap<Key, Option<ParameterValue>>,
    ) -> Result<(), ParameterCollectionError> {
        for (key, value_opt) in snapshot {
            if self.contains_key(key) {
                if let Some(value) = value_opt {
                    self.set_value(key, value.clone())?;
                }
            }
        }

        Ok(())
    }

    /// Executes a function with a typed reference to a parameter
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the parameter to retrieve
    /// * `f` - The function to execute on the parameter if it's of the correct
    ///   type
    ///
    /// # Returns
    ///
    /// * `Ok(R)` where R is the return value of the function
    /// * `Err(ParameterCollectionError::NotFound)` if the parameter was not
    ///   found
    /// * `Err(ParameterCollectionError::TypeError)` if the parameter is not of
    ///   the requested type
    pub fn with_parameter<P, F, R>(&self, key: &Key, f: F) -> Result<R, ParameterCollectionError>
    where
        P: Parameter + 'static,
        F: FnOnce(&P) -> R,
    {
        let param = self.get_as::<P>(key)?;
        Ok(f(param))
    }

    /// Executes a function with a typed mutable reference to a parameter
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the parameter to retrieve
    /// * `f` - The function to execute on the parameter if it's of the correct
    ///   type
    ///
    /// # Returns
    ///
    /// * `Ok(R)` where R is the return value of the function
    /// * `Err(ParameterCollectionError::NotFound)` if the parameter was not
    ///   found
    /// * `Err(ParameterCollectionError::TypeError)` if the parameter is not of
    ///   the requested type
    pub fn with_parameter_mut<P, F, R>(
        &mut self,
        key: &Key,
        f: F,
    ) -> Result<R, ParameterCollectionError>
    where
        P: Parameter + 'static,
        F: FnOnce(&mut P) -> R,
    {
        let param = self.get_as_mut::<P>(key)?;
        Ok(f(param))
    }
}
