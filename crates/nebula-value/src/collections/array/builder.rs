//! Builder pattern for Array construction
//!
//! Provides a fluent API for building arrays with validation.

use crate::collections::Array;
use crate::core::NebulaError;
use crate::core::error::{ValueErrorExt, ValueResult};
use crate::core::limits::ValueLimits;

// TEMP: using serde_json::Value as placeholder
type ValueItem = serde_json::Value;

/// Builder for creating Array with validation and limits
///
/// # Examples
///
/// ```
/// use nebula_value::collections::array::ArrayBuilder;
///
/// let array = ArrayBuilder::new()
///     .push(serde_json::json!(1))
///     .push(serde_json::json!(2))
///     .push(serde_json::json!(3))
///     .build()
///     .unwrap();
///
/// assert_eq!(array.len(), 3);
/// ```
#[derive(Debug, Clone)]
pub struct ArrayBuilder {
    items: Vec<ValueItem>,
    limits: Option<ValueLimits>,
}

impl ArrayBuilder {
    /// Create a new empty builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            limits: None,
        }
    }

    /// Create a builder with initial capacity
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            items: Vec::with_capacity(capacity),
            limits: None,
        }
    }

    /// Set value limits for validation
    #[must_use = "builder methods return a new instance"]
    pub fn with_limits(mut self, limits: ValueLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// Add an item to the array
    #[must_use = "builder methods return a new instance"]
    pub fn push(mut self, item: ValueItem) -> Self {
        self.items.push(item);
        self
    }

    /// Add an item with validation
    pub fn try_push(mut self, item: ValueItem) -> ValueResult<Self> {
        if let Some(ref limits) = self.limits {
            limits.check_array_length(self.items.len() + 1)?;
        }
        self.items.push(item);
        Ok(self)
    }

    /// Add multiple items
    #[must_use = "builder methods return a new instance"]
    pub fn extend<I>(mut self, items: I) -> Self
    where
        I: IntoIterator<Item = ValueItem>,
    {
        self.items.extend(items);
        self
    }

    /// Add multiple items with validation
    pub fn try_extend<I>(mut self, items: I) -> ValueResult<Self>
    where
        I: IntoIterator<Item = ValueItem>,
    {
        let items: Vec<_> = items.into_iter().collect();

        if let Some(ref limits) = self.limits {
            limits.check_array_length(self.items.len() + items.len())?;
        }

        self.items.extend(items);
        Ok(self)
    }

    /// Insert an item at a specific index
    pub fn insert(mut self, index: usize, item: ValueItem) -> ValueResult<Self> {
        if index > self.items.len() {
            return Err(NebulaError::value_index_out_of_bounds(
                index,
                self.items.len(),
            ));
        }

        if let Some(ref limits) = self.limits {
            limits.check_array_length(self.items.len() + 1)?;
        }

        self.items.insert(index, item);
        Ok(self)
    }

    /// Remove an item at a specific index
    pub fn remove(mut self, index: usize) -> ValueResult<Self> {
        if index >= self.items.len() {
            return Err(NebulaError::value_index_out_of_bounds(
                index,
                self.items.len(),
            ));
        }

        self.items.remove(index);
        Ok(self)
    }

    /// Clear all items
    #[must_use = "builder methods return a new instance"]
    pub fn clear(mut self) -> Self {
        self.items.clear();
        self
    }

    /// Get the current number of items
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Check if the builder is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Build the final Array
    pub fn build(self) -> ValueResult<Array> {
        if let Some(ref limits) = self.limits {
            limits.check_array_length(self.items.len())?;
        }

        Ok(Array::from_iter(self.items))
    }

    /// Build without validation (unsafe)
    #[must_use]
    pub fn build_unchecked(self) -> Array {
        Array::from_iter(self.items)
    }
}

impl Default for ArrayBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience macro for building arrays
///
/// # Examples
///
/// ```ignore
/// use nebula_value::array;
///
/// let arr = array![1, 2, 3];
/// assert_eq!(arr.len(), 3);
/// ```
#[macro_export]
macro_rules! array {
    () => {
        $crate::collections::Array::new()
    };
    ($($item:expr),+ $(,)?) => {
        $crate::collections::array::ArrayBuilder::new()
            $(.push(serde_json::json!($item)))+
            .build()
            .expect("Array construction failed")
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_empty() {
        let array = ArrayBuilder::new().build().unwrap();
        assert_eq!(array.len(), 0);
        assert!(array.is_empty());
    }

    #[test]
    fn test_builder_push() {
        let array = ArrayBuilder::new()
            .push(serde_json::json!(1))
            .push(serde_json::json!(2))
            .push(serde_json::json!(3))
            .build()
            .unwrap();

        assert_eq!(array.len(), 3);
        assert_eq!(array.get(0), Some(&serde_json::json!(1)));
        assert_eq!(array.get(1), Some(&serde_json::json!(2)));
        assert_eq!(array.get(2), Some(&serde_json::json!(3)));
    }

    #[test]
    fn test_builder_with_capacity() {
        let builder = ArrayBuilder::with_capacity(10);
        assert_eq!(builder.len(), 0);
    }

    #[test]
    fn test_builder_extend() {
        let array = ArrayBuilder::new()
            .extend(vec![
                serde_json::json!(1),
                serde_json::json!(2),
                serde_json::json!(3),
            ])
            .build()
            .unwrap();

        assert_eq!(array.len(), 3);
    }

    #[test]
    fn test_builder_with_limits() {
        let limits = ValueLimits {
            max_array_length: 2,
            ..Default::default()
        };

        let result = ArrayBuilder::new()
            .with_limits(limits)
            .push(serde_json::json!(1))
            .push(serde_json::json!(2))
            .push(serde_json::json!(3))
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_builder_try_push_exceeds_limit() {
        let limits = ValueLimits {
            max_array_length: 2,
            ..Default::default()
        };

        let result = ArrayBuilder::new()
            .with_limits(limits)
            .try_push(serde_json::json!(1))
            .unwrap()
            .try_push(serde_json::json!(2))
            .unwrap()
            .try_push(serde_json::json!(3));

        assert!(result.is_err());
    }

    #[test]
    fn test_builder_insert() {
        let array = ArrayBuilder::new()
            .push(serde_json::json!(1))
            .push(serde_json::json!(3))
            .insert(1, serde_json::json!(2))
            .unwrap()
            .build()
            .unwrap();

        assert_eq!(array.len(), 3);
        assert_eq!(array.get(0), Some(&serde_json::json!(1)));
        assert_eq!(array.get(1), Some(&serde_json::json!(2)));
        assert_eq!(array.get(2), Some(&serde_json::json!(3)));
    }

    #[test]
    fn test_builder_remove() {
        let array = ArrayBuilder::new()
            .push(serde_json::json!(1))
            .push(serde_json::json!(2))
            .push(serde_json::json!(3))
            .remove(1)
            .unwrap()
            .build()
            .unwrap();

        assert_eq!(array.len(), 2);
        assert_eq!(array.get(0), Some(&serde_json::json!(1)));
        assert_eq!(array.get(1), Some(&serde_json::json!(3)));
    }

    #[test]
    fn test_builder_clear() {
        let array = ArrayBuilder::new()
            .push(serde_json::json!(1))
            .push(serde_json::json!(2))
            .clear()
            .build()
            .unwrap();

        assert_eq!(array.len(), 0);
    }
}
