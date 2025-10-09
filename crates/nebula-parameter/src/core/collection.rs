//! Type-safe parameter collection with dependency tracking

use std::any::Any;
use std::collections::HashMap;

use nebula_core::ParameterKey;
use nebula_value::Value;

use crate::core::{ParameterError, ParameterValue};

/// A type-safe collection of parameters with dependency tracking
#[derive(Default)]
pub struct ParameterCollection {
    /// Storage for all parameters
    parameters: HashMap<ParameterKey, Box<dyn ParameterValue>>,

    /// Dependency graph (parameter_key -> depends_on_keys)
    dependencies: HashMap<ParameterKey, Vec<ParameterKey>>,
}

impl ParameterCollection {
    /// Create a new empty parameter collection
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a parameter to the collection
    pub fn add<P>(&mut self, param: P) -> &mut Self
    where
        P: ParameterValue + 'static,
    {
        let key = param.metadata().key.clone();

        // Extract dependencies from display rules if present
        if let Some(display) = param.display() {
            let deps = display.get_dependencies();
            if !deps.is_empty() {
                self.dependencies.insert(key.clone(), deps);
            }
        }

        self.parameters.insert(key, Box::new(param));
        self
    }

    /// Add a parameter using builder-style chaining
    pub fn with<P>(mut self, param: P) -> Self
    where
        P: ParameterValue + 'static,
    {
        self.add(param);
        self
    }

    /// Get a parameter by key with type safety
    pub fn get<P>(&self, key: impl Into<ParameterKey>) -> Option<&P>
    where
        P: ParameterValue + 'static,
    {
        self.parameters
            .get(&key.into())?
            .as_any()
            .downcast_ref::<P>()
    }

    /// Get a mutable parameter by key with type safety
    pub fn get_mut<P>(&mut self, key: impl Into<ParameterKey>) -> Option<&mut P>
    where
        P: ParameterValue + 'static,
    {
        self.parameters
            .get_mut(&key.into())?
            .as_any_mut()
            .downcast_mut::<P>()
    }

    /// Get a parameter's value (type-erased)
    pub fn value(&self, key: impl Into<ParameterKey>) -> Option<Value> {
        self.parameters.get(&key.into())?.get_erased()
    }

    /// Get a typed value from a parameter
    pub fn typed_value<T>(&self, key: impl Into<ParameterKey>) -> Result<T, ParameterError>
    where
        T: TryFrom<Value>,
        T::Error: std::fmt::Display,
    {
        let key_obj = key.into();
        let value = self
            .value(key_obj.clone())
            .ok_or_else(|| ParameterError::not_found(key_obj.clone()))?;

        value.try_into().map_err(|e| {
            ParameterError::type_error(key_obj, std::any::type_name::<T>(), e.to_string())
        })
    }

    /// Check if a parameter exists
    pub fn contains(&self, key: impl Into<ParameterKey>) -> bool {
        self.parameters.contains_key(&key.into())
    }

    /// Remove a parameter from the collection
    pub fn remove(&mut self, key: impl Into<ParameterKey>) -> Option<Box<dyn ParameterValue>> {
        let key = key.into();
        self.dependencies.remove(&key);
        self.parameters.remove(&key)
    }

    /// Get all parameter keys
    pub fn keys(&self) -> impl Iterator<Item = &ParameterKey> {
        self.parameters.keys()
    }

    /// Get the number of parameters
    pub fn len(&self) -> usize {
        self.parameters.len()
    }

    /// Check if the collection is empty
    pub fn is_empty(&self) -> bool {
        self.parameters.is_empty()
    }

    /// Clear all parameters
    pub fn clear(&mut self) {
        self.parameters.clear();
        self.dependencies.clear();
    }

    /// Get dependencies for a parameter
    pub fn get_dependencies(&self, key: impl Into<ParameterKey>) -> Vec<ParameterKey> {
        self.dependencies
            .get(&key.into())
            .cloned()
            .unwrap_or_default()
    }

    /// Get all parameters that depend on the given key
    pub fn get_dependents(&self, key: impl Into<ParameterKey>) -> Vec<ParameterKey> {
        let target_key = key.into();
        self.dependencies
            .iter()
            .filter_map(|(k, deps)| {
                if deps.contains(&target_key) {
                    Some(k.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Validate all parameters in the collection
    pub async fn validate_all(&self) -> ValidationResult {
        let mut errors = Vec::new();

        // Validate in topological order (dependencies first)
        for key in self.topological_sort() {
            if let Some(param) = self.parameters.get(&key) {
                if let Err(e) = param.validate_erased().await {
                    errors.push((key.clone(), e));
                }
            }
        }

        if errors.is_empty() {
            ValidationResult::Valid
        } else {
            ValidationResult::Invalid(errors)
        }
    }

    /// Get parameters in topological order (dependencies first)
    fn topological_sort(&self) -> Vec<ParameterKey> {
        let mut result = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut temp_mark = std::collections::HashSet::new();

        for key in self.parameters.keys() {
            if !visited.contains(key) {
                self.visit_node(key, &mut visited, &mut temp_mark, &mut result);
            }
        }

        result
    }

    fn visit_node(
        &self,
        key: &ParameterKey,
        visited: &mut std::collections::HashSet<ParameterKey>,
        temp_mark: &mut std::collections::HashSet<ParameterKey>,
        result: &mut Vec<ParameterKey>,
    ) {
        if temp_mark.contains(key) {
            // Cycle detected - skip this node
            return;
        }

        if visited.contains(key) {
            return;
        }

        temp_mark.insert(key.clone());

        // Visit dependencies first
        if let Some(deps) = self.dependencies.get(key) {
            for dep in deps {
                if self.parameters.contains_key(dep) {
                    self.visit_node(dep, visited, temp_mark, result);
                }
            }
        }

        temp_mark.remove(key);
        visited.insert(key.clone());
        result.push(key.clone());
    }

    /// Create a snapshot of all parameter values
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            values: self
                .parameters
                .iter()
                .map(|(k, v)| (k.clone(), v.get_erased()))
                .collect(),
        }
    }

    /// Restore parameter values from a snapshot
    pub fn restore(&mut self, snapshot: &Snapshot) -> Result<(), ParameterError> {
        for (key, value) in &snapshot.values {
            if let Some(param) = self.parameters.get_mut(key) {
                if let Some(v) = value {
                    param.set_erased(v.clone())?;
                } else {
                    param.clear_erased();
                }
            }
        }
        Ok(())
    }
}

/// Result of validating all parameters
#[derive(Debug)]
pub enum ValidationResult {
    /// All parameters are valid
    Valid,
    /// Some parameters failed validation
    Invalid(Vec<(ParameterKey, ParameterError)>),
}

impl ValidationResult {
    /// Check if validation passed
    pub fn is_valid(&self) -> bool {
        matches!(self, ValidationResult::Valid)
    }

    /// Get validation errors
    pub fn errors(&self) -> Option<&[(ParameterKey, ParameterError)]> {
        match self {
            ValidationResult::Valid => None,
            ValidationResult::Invalid(errors) => Some(errors),
        }
    }
}

/// Snapshot of parameter values for undo/redo
#[derive(Debug, Clone)]
pub struct Snapshot {
    values: HashMap<ParameterKey, Option<Value>>,
}

impl Snapshot {
    /// Create an empty snapshot
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    /// Get the number of captured values
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Check if snapshot is empty
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

impl Default for Snapshot {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TextParameter;

    #[test]
    fn test_collection_new() {
        let collection = ParameterCollection::new();
        assert!(collection.is_empty());
        assert_eq!(collection.len(), 0);
    }

    #[test]
    fn test_collection_add() {
        let mut collection = ParameterCollection::new();

        let param = TextParameter::builder()
            .metadata(crate::core::ParameterMetadata::new("test", "Test"))
            .build();

        collection.add(param);

        assert_eq!(collection.len(), 1);
        assert!(collection.contains("test"));
    }

    #[test]
    fn test_collection_with() {
        let collection = ParameterCollection::new()
            .with(
                TextParameter::builder()
                    .metadata(crate::core::ParameterMetadata::new("test1", "Test 1"))
                    .build(),
            )
            .with(
                TextParameter::builder()
                    .metadata(crate::core::ParameterMetadata::new("test2", "Test 2"))
                    .build(),
            );

        assert_eq!(collection.len(), 2);
    }

    #[test]
    fn test_collection_get_typed() {
        let mut collection = ParameterCollection::new();

        collection.add(
            TextParameter::builder()
                .metadata(crate::core::ParameterMetadata::new("test", "Test"))
                .value(Some(nebula_value::Text::from("hello")))
                .build(),
        );

        let param: Option<&TextParameter> = collection.get("test");
        assert!(param.is_some());
    }

    #[test]
    fn test_snapshot_restore() {
        let mut collection = ParameterCollection::new();

        let mut param = TextParameter::builder()
            .metadata(crate::core::ParameterMetadata::new("test", "Test"))
            .value(Some(nebula_value::Text::from("initial")))
            .build();

        collection.add(param);

        // Take snapshot
        let snapshot = collection.snapshot();

        // Modify value
        if let Some(p) = collection.get_mut::<TextParameter>("test") {
            let _ = p.set(nebula_value::Text::from("modified"));
        }

        // Restore
        collection.restore(&snapshot).unwrap();

        let value = collection.value("test").unwrap();
        assert_eq!(value.as_text().unwrap().as_str(), "initial");
    }
}
