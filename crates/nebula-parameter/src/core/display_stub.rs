//! Temporary stubs for display system (to be rewritten)

use nebula_core::ParameterKey;
use nebula_value::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Temporary stub for DisplayContext
#[derive(Debug, Clone, Default)]
pub struct DisplayContext {
    /// Map of parameter keys to their current values
    pub values: HashMap<ParameterKey, Value>,
}

impl DisplayContext {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn with_value(mut self, key: impl Into<ParameterKey>, value: Value) -> Self {
        self.values.insert(key.into(), value);
        self
    }

    pub fn get(&self, key: &ParameterKey) -> Option<&Value> {
        self.values.get(key)
    }
}

/// Temporary stub for ParameterDisplay
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParameterDisplay {
    // Placeholder - empty struct for now
    #[serde(skip)]
    _phantom: (),
}

impl ParameterDisplay {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn should_display(&self, _context: &HashMap<ParameterKey, Value>) -> bool {
        true
    }

    pub fn get_dependencies(&self) -> Vec<ParameterKey> {
        Vec::new()
    }

    pub fn is_empty(&self) -> bool {
        true
    }
}

/// Temporary stub for ParameterCondition
#[derive(Debug, Clone)]
pub struct ParameterCondition {
    // Placeholder
}

impl ParameterCondition {
    pub fn equals(_value: Value) -> Self {
        Self {}
    }
}
