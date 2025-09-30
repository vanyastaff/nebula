//! ArrayBuilder - efficient construction of arrays with batch operations
//!
//! This module provides a builder pattern for Array that avoids O(n²)
//! performance issues when creating arrays incrementally.

use crate::types::{Array, ArrayError};
use crate::{Value, ValueLimits};

/// Builder for efficient array construction
///
/// # Performance
///
/// Using ArrayBuilder is significantly more efficient than repeated push operations:
/// - **With builder**: O(n) - single allocation and copy
/// - **Without builder**: O(n²) - copy-on-write for each push
///
/// # Example
///
/// ```
/// use nebula_value::{ArrayBuilder, Value};
///
/// let array = ArrayBuilder::new()
///     .push(Value::int(1))
///     .push(Value::int(2))
///     .push(Value::int(3))
///     .build();
///
/// assert_eq!(array.len(), 3);
/// ```
#[derive(Debug, Clone)]
pub struct ArrayBuilder {
    items: Vec<Value>,
    limits: Option<ValueLimits>,
}

impl Default for ArrayBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ArrayBuilder {
    /// Create a new array builder
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            limits: None,
        }
    }

    /// Create a builder with capacity hint
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            items: Vec::with_capacity(capacity),
            limits: None,
        }
    }

    /// Set value limits for validation
    pub fn with_limits(mut self, limits: ValueLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// Push a single value
    pub fn push(mut self, value: Value) -> Self {
        self.items.push(value);
        self
    }

    /// Try to push a value, checking limits
    pub fn try_push(mut self, value: Value) -> Result<Self, ArrayError> {
        if let Some(limits) = &self.limits {
            limits.check_array_length(self.items.len() + 1)
                .map_err(|e| ArrayError::LimitExceeded { msg: e.to_string() })?;
        }
        self.items.push(value);
        Ok(self)
    }

    /// Push multiple values at once
    pub fn extend<I: IntoIterator<Item = Value>>(mut self, iter: I) -> Self {
        self.items.extend(iter);
        self
    }

    /// Try to extend with limit checking
    pub fn try_extend<I: IntoIterator<Item = Value>>(mut self, iter: I) -> Result<Self, ArrayError> {
        let iter = iter.into_iter();
        let (lower, _) = iter.size_hint();

        if let Some(limits) = &self.limits {
            limits.check_array_length(self.items.len() + lower)
                .map_err(|e| ArrayError::LimitExceeded { msg: e.to_string() })?;
        }

        self.items.extend(iter);
        Ok(self)
    }

    /// Reserve additional capacity
    pub fn reserve(mut self, additional: usize) -> Self {
        self.items.reserve(additional);
        self
    }

    /// Get current length
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Build the final array
    pub fn build(self) -> Array {
        Array::new(self.items)
    }

    /// Try to build with final validation
    pub fn try_build(self) -> Result<Array, ArrayError> {
        if let Some(limits) = &self.limits {
            limits.check_array_length(self.items.len())
                .map_err(|e| ArrayError::LimitExceeded { msg: e.to_string() })?;
        }
        Ok(Array::new(self.items))
    }
}

impl From<Vec<Value>> for ArrayBuilder {
    fn from(items: Vec<Value>) -> Self {
        Self {
            items,
            limits: None,
        }
    }
}

impl FromIterator<Value> for ArrayBuilder {
    fn from_iter<I: IntoIterator<Item = Value>>(iter: I) -> Self {
        Self {
            items: iter.into_iter().collect(),
            limits: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_basic() {
        let array = ArrayBuilder::new()
            .push(Value::int(1))
            .push(Value::int(2))
            .push(Value::int(3))
            .build();

        assert_eq!(array.len(), 3);
        assert_eq!(array.get(0).unwrap().as_i64(), Some(1));
    }

    #[test]
    fn test_builder_with_capacity() {
        let builder = ArrayBuilder::with_capacity(100);
        assert_eq!(builder.len(), 0);
    }

    #[test]
    fn test_builder_extend() {
        let values = vec![Value::int(1), Value::int(2), Value::int(3)];
        let array = ArrayBuilder::new()
            .extend(values)
            .build();

        assert_eq!(array.len(), 3);
    }

    #[test]
    fn test_builder_with_limits() {
        let limits = ValueLimits::strict();
        let result = ArrayBuilder::new()
            .with_limits(limits)
            .extend((0..5000).map(Value::int))
            .try_build();

        assert!(result.is_ok());

        // Exceeding limit
        let result = ArrayBuilder::new()
            .with_limits(limits)
            .extend((0..15000).map(Value::int))
            .try_build();

        assert!(result.is_err());
    }

    #[test]
    fn test_from_vec() {
        let vec = vec![Value::int(1), Value::int(2)];
        let builder = ArrayBuilder::from(vec);
        assert_eq!(builder.len(), 2);
    }

    #[test]
    fn test_from_iter() {
        let builder: ArrayBuilder = (0..5).map(Value::int).collect();
        assert_eq!(builder.len(), 5);
    }
}