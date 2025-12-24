//! Type-safe parameter collection with dependency tracking

use std::collections::HashMap;

use nebula_core::ParameterKey;

use crate::core::{Displayable, Parameter};

/// A type-safe collection of parameters with dependency tracking
#[derive(Default)]
pub struct ParameterCollection {
    /// Storage for all parameters (schema only, no values)
    parameters: HashMap<ParameterKey, Box<dyn Parameter>>,

    /// Dependency graph (`parameter_key` -> `depends_on_keys`)
    dependencies: HashMap<ParameterKey, Vec<ParameterKey>>,
}

impl ParameterCollection {
    /// Create a new empty parameter collection
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a parameter to the collection
    pub fn add<P>(&mut self, param: P) -> &mut Self
    where
        P: Parameter + Displayable + 'static,
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
    #[must_use = "builder methods must be chained or built"]
    pub fn with<P>(mut self, param: P) -> Self
    where
        P: Parameter + Displayable + 'static,
    {
        self.add(param);
        self
    }

    /// Get a parameter by key with type safety
    pub fn get<P>(&self, key: impl Into<ParameterKey>) -> Option<&P>
    where
        P: Parameter + 'static,
    {
        self.parameters.get(&key.into())?.downcast_ref::<P>()
    }

    /// Get a mutable parameter by key with type safety
    pub fn get_mut<P>(&mut self, key: impl Into<ParameterKey>) -> Option<&mut P>
    where
        P: Parameter + 'static,
    {
        self.parameters.get_mut(&key.into())?.downcast_mut::<P>()
    }

    /// Check if a parameter exists
    pub fn contains(&self, key: impl Into<ParameterKey>) -> bool {
        self.parameters.contains_key(&key.into())
    }

    /// Remove a parameter from the collection
    pub fn remove(&mut self, key: impl Into<ParameterKey>) -> Option<Box<dyn Parameter>> {
        let key = key.into();
        self.dependencies.remove(&key);
        self.parameters.remove(&key)
    }

    /// Get all parameter keys
    pub fn keys(&self) -> impl Iterator<Item = &ParameterKey> {
        self.parameters.keys()
    }

    /// Get the number of parameters
    #[must_use]
    pub fn len(&self) -> usize {
        self.parameters.len()
    }

    /// Check if the collection is empty
    #[must_use]
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

    /// Get parameters in topological order (dependencies first)
    pub fn topological_sort(&self) -> Vec<ParameterKey> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TextParameter;

    /// Helper to create a ParameterKey for tests
    fn key(s: &str) -> ParameterKey {
        ParameterKey::new(s).expect("invalid test key")
    }

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
            .metadata(
                crate::core::ParameterMetadata::builder()
                    .key("test")
                    .name("Test")
                    .description("")
                    .build()
                    .unwrap(),
            )
            .build();

        collection.add(param);

        assert_eq!(collection.len(), 1);
        assert!(collection.contains(key("test")));
    }

    #[test]
    fn test_collection_with() {
        let collection = ParameterCollection::new()
            .with(
                TextParameter::builder()
                    .metadata(
                        crate::core::ParameterMetadata::builder()
                            .key("test1")
                            .name("Test 1")
                            .description("")
                            .build()
                            .unwrap(),
                    )
                    .build(),
            )
            .with(
                TextParameter::builder()
                    .metadata(
                        crate::core::ParameterMetadata::builder()
                            .key("test2")
                            .name("Test 2")
                            .description("")
                            .build()
                            .unwrap(),
                    )
                    .build(),
            );

        assert_eq!(collection.len(), 2);
    }

    #[test]
    fn test_collection_get_typed() {
        let mut collection = ParameterCollection::new();

        collection.add(
            TextParameter::builder()
                .metadata(
                    crate::core::ParameterMetadata::builder()
                        .key("test")
                        .name("Test")
                        .description("")
                        .build()
                        .unwrap(),
                )
                .build(),
        );

        let param: Option<&TextParameter> = collection.get(key("test"));
        assert!(param.is_some());
    }

    #[test]
    fn test_snapshot_restore_with_parameter_values() {
        use crate::core::values::ParameterValues;
        use nebula_value::Value;

        let mut values = ParameterValues::new();
        values.set(key("test"), Value::text("initial"));

        // Take snapshot
        let snapshot = values.snapshot();

        // Modify value
        values.set(key("test"), Value::text("modified"));
        assert_eq!(
            values.get(key("test")).unwrap().as_text().unwrap().as_str(),
            "modified"
        );

        // Restore
        values.restore(&snapshot);
        assert_eq!(
            values.get(key("test")).unwrap().as_text().unwrap().as_str(),
            "initial"
        );
    }
}
