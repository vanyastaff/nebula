//! Array type for nebula-value
//!
//! This module provides an Array type that:
//! - Length limits for `DoS` protection
//! - O(log n) operations for most operations
//! - Thread-safe via Arc
//! - Uses persistent data structures (`im::Vector`) for efficient cloning
pub mod builder;

pub use builder::ArrayBuilder;

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Index;

use im::Vector;

use crate::core::Value;
use crate::core::limits::ValueLimits;
use crate::core::{ValueError, ValueResult};

/// Type alias for items stored in arrays
pub type ValueItem = Value;

/// Persistent array with efficient structural sharing
///
/// Uses `im::Vector` internally which provides:
/// - O(log n) push/pop/get/set
/// - Efficient cloning via structural sharing
/// - Thread-safe immutable operations
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Array {
    inner: Vector<ValueItem>,
}

impl Array {
    /// Create an empty array
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Vector::new(),
        }
    }

    /// Create from a Vec
    #[must_use]
    pub fn from_vec(vec: Vec<ValueItem>) -> Self {
        Self {
            inner: Vector::from(vec),
        }
    }

    /// Create from an iterator of `nebula_value::Value` items
    #[cfg(feature = "serde")]
    pub fn from_nebula_values<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = crate::Value>,
    {
        let items: Vec<ValueItem> = iter.into_iter().collect();
        Self::from_vec(items)
    }

    /// Create with length validation
    ///
    /// # Errors
    ///
    /// Returns `ValueError::LimitExceeded` if array length exceeds `max_array_length`
    pub fn with_limits(vec: Vec<ValueItem>, limits: &ValueLimits) -> ValueResult<Self> {
        limits.check_array_length(vec.len())?;
        Ok(Self::from_vec(vec))
    }

    /// Get the length
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if empty
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get element at index
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&ValueItem> {
        self.inner.get(index)
    }

    /// Get element at index or error
    ///
    /// # Errors
    ///
    /// Returns `ValueError::IndexOutOfBounds` if `index >= len()`
    pub fn try_get(&self, index: usize) -> ValueResult<&ValueItem> {
        self.get(index)
            .ok_or_else(|| ValueError::index_out_of_bounds(index, self.len()))
    }

    /// Push an element (returns new Array, original unchanged)
    #[must_use = "immutable methods return a new instance"]
    pub fn push(&self, value: impl Into<ValueItem>) -> Self {
        let mut new_vec = self.inner.clone();
        new_vec.push_back(value.into());
        Self { inner: new_vec }
    }

    /// Push with limit check
    ///
    /// # Errors
    ///
    /// Returns `ValueError::LimitExceeded` if array length would exceed `max_array_length`
    pub fn push_with_limit(
        &self,
        value: impl Into<ValueItem>,
        limits: &ValueLimits,
    ) -> ValueResult<Self> {
        limits.check_array_length(self.len() + 1)?;
        Ok(self.push(value))
    }

    /// Pop last element (returns new Array and popped value)
    #[must_use = "immutable methods return a new instance"]
    pub fn pop(&self) -> Option<(Self, ValueItem)> {
        let mut new_vec = self.inner.clone();
        new_vec.pop_back().map(|val| (Self { inner: new_vec }, val))
    }

    /// Set element at index (returns new Array)
    ///
    /// # Errors
    ///
    /// Returns `ValueError::IndexOutOfBounds` if `index >= len()`
    pub fn set(&self, index: usize, value: impl Into<ValueItem>) -> ValueResult<Self> {
        if index >= self.len() {
            return Err(ValueError::index_out_of_bounds(index, self.len()));
        }

        let mut new_vec = self.inner.clone();
        new_vec.set(index, value.into());
        Ok(Self { inner: new_vec })
    }

    /// Insert element at index (returns new Array)
    ///
    /// # Errors
    ///
    /// Returns `ValueError::IndexOutOfBounds` if `index > len()`
    pub fn insert(&self, index: usize, value: impl Into<ValueItem>) -> ValueResult<Self> {
        if index > self.len() {
            return Err(ValueError::index_out_of_bounds(index, self.len()));
        }

        let mut new_vec = self.inner.clone();
        new_vec.insert(index, value.into());
        Ok(Self { inner: new_vec })
    }

    /// Remove element at index (returns new Array and removed value)
    ///
    /// # Errors
    ///
    /// Returns `ValueError::IndexOutOfBounds` if `index >= len()`
    pub fn remove(&self, index: usize) -> ValueResult<(Self, ValueItem)> {
        if index >= self.len() {
            return Err(ValueError::index_out_of_bounds(index, self.len()));
        }

        let mut new_vec = self.inner.clone();
        let removed = new_vec.remove(index);
        Ok((Self { inner: new_vec }, removed))
    }

    /// Get first element
    #[must_use]
    pub fn first(&self) -> Option<&ValueItem> {
        self.inner.front()
    }

    /// Get last element
    #[must_use]
    pub fn last(&self) -> Option<&ValueItem> {
        self.inner.back()
    }

    /// Concatenate with another array
    #[must_use = "immutable methods return a new instance"]
    pub fn concat(&self, other: &Array) -> Self {
        let mut new_vec = self.inner.clone();
        new_vec.append(other.inner.clone());
        Self { inner: new_vec }
    }

    /// Get a slice of the array
    pub fn slice(&self, start: usize, end: usize) -> ValueResult<Self> {
        if start > end || end > self.len() {
            return Err(ValueError::out_of_range(
                format!("{}..{}", start, end),
                "0",
                self.len().to_string(),
            ));
        }

        let slice: Vector<ValueItem> = self
            .inner
            .iter()
            .skip(start)
            .take(end - start)
            .cloned()
            .collect();

        Ok(Self { inner: slice })
    }

    /// Check if array contains a value
    pub fn contains(&self, value: &ValueItem) -> bool {
        self.inner.iter().any(|v| v == value)
    }

    /// Reverse the array
    pub fn reverse(&self) -> Self {
        let reversed: Vector<ValueItem> = self.inner.iter().rev().cloned().collect();
        Self { inner: reversed }
    }

    /// Create iterator
    pub fn iter(&self) -> impl Iterator<Item = &ValueItem> {
        self.inner.iter()
    }

    /// Convert to Vec (allocates)
    pub fn to_vec(&self) -> Vec<ValueItem> {
        self.inner.iter().cloned().collect()
    }
}

impl Default for Array {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for Array {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for Array {}

impl Hash for Array {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for item in self.inner.iter() {
            format!("{:?}", item).hash(state);
        }
    }
}

impl Index<usize> for Array {
    type Output = ValueItem;

    fn index(&self, index: usize) -> &Self::Output {
        &self.inner[index]
    }
}

impl fmt::Display for Array {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}]", self.len())
    }
}

impl From<Vec<ValueItem>> for Array {
    fn from(vec: Vec<ValueItem>) -> Self {
        Self::from_vec(vec)
    }
}

impl FromIterator<ValueItem> for Array {
    fn from_iter<I: IntoIterator<Item = ValueItem>>(iter: I) -> Self {
        Self {
            inner: iter.into_iter().collect(),
        }
    }
}

// ==================== IntoIterator ====================

impl IntoIterator for Array {
    type Item = Value;
    type IntoIter = im::vector::ConsumingIter<Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<'a> IntoIterator for &'a Array {
    type Item = &'a Value;
    type IntoIter = im::vector::Iter<'a, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_array_creation() {
        let arr = Array::new();
        assert_eq!(arr.len(), 0);
        assert!(arr.is_empty());
    }

    #[test]
    fn test_array_from_vec() {
        let arr = Array::from_vec(vec![
            Value::integer(1),
            Value::integer(2),
            Value::integer(3),
        ]);
        assert_eq!(arr.len(), 3);
        assert_eq!(arr.get(0), Some(&Value::integer(1)));
    }

    #[test]
    fn test_array_push() {
        let arr = Array::new();
        let arr = arr.push(Value::integer(1));
        let arr = arr.push(Value::integer(2));

        assert_eq!(arr.len(), 2);
        assert_eq!(arr.get(0), Some(&Value::integer(1)));
    }

    #[test]
    fn test_array_structural_sharing() {
        let arr1 = Array::from_vec(vec![Value::integer(1), Value::integer(2)]);
        let arr2 = arr1.push(Value::integer(3));

        assert_eq!(arr1.len(), 2);
        assert_eq!(arr2.len(), 3);
    }
}
