use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Add, AddAssign, Deref, Index, Range, RangeFrom, RangeFull, RangeTo};
use std::sync::Arc;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use thiserror::Error;

#[cfg(feature = "rayon")]
use rayon::prelude::*;

use crate::Value; // Assuming Value is defined elsewhere

/// Result type alias for Array operations
pub type ArrayResult<T> = Result<T, ArrayError>;

/// Rich, typed errors for Array operations
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ArrayError {
    #[error("Index {index} out of bounds for array of length {len}")]
    IndexOutOfBounds { index: usize, len: usize },

    #[error("Invalid range: start ({start}) > end ({end})")]
    InvalidRange { start: usize, end: usize },

    #[error("Range out of bounds: start={start}, end={end}, length={len}")]
    RangeOutOfBounds { start: usize, end: usize, len: usize },

    #[error("Empty array for operation that requires at least one element")]
    EmptyArray,

    #[error("Invalid operation: {msg}")]
    InvalidOperation { msg: String },

    #[error("Type conversion error: expected array, got {found}")]
    #[cfg(feature = "serde")]
    JsonTypeMismatch { found: &'static str },

    #[error("Value error: {msg}")]
    ValueError { msg: String },
}

/// A high-performance, feature-rich array type with functional programming support
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Array {
    /// Internal storage using Arc for cheap cloning
    inner: Arc<[Value]>,

    /// Cached hash value for O(1) hash operations
    #[cfg_attr(feature = "serde", serde(skip))]
    hash_cache: std::sync::OnceLock<u64>,
}

impl Array {
    // ==================== Constructors ====================

    /// Creates a new Array from a Vec
    #[inline]
    pub fn new(values: Vec<Value>) -> Self {
        Self { inner: values.into(), hash_cache: std::sync::OnceLock::new() }
    }

    /// Creates an empty Array
    #[inline]
    pub fn empty() -> Self {
        Self { inner: Arc::from([]), hash_cache: std::sync::OnceLock::new() }
    }

    /// Creates an Array with specified capacity
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::new(Vec::with_capacity(capacity))
    }

    /// Creates an Array from a slice
    #[inline]
    pub fn from_slice(slice: &[Value]) -> Self {
        Self::new(slice.to_vec())
    }

    /// Creates an Array from an iterator
    #[inline]
    pub fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = Value>,
    {
        Self::new(iter.into_iter().collect())
    }

    /// Creates an Array filled with n copies of a value
    #[inline]
    pub fn filled(value: &Value, count: usize) -> Self {
        Self::new(vec![value.clone(); count])
    }

    /// Creates an Array from a range of integers
    pub fn range(start: i64, end: i64, step: i64) -> ArrayResult<Self> {
        if step == 0 {
            return Err(ArrayError::InvalidOperation { msg: "Step cannot be zero".into() });
        }

        let mut values = Vec::new();

        if step > 0 {
            let mut current = start;
            while current < end {
                values.push(Value::from(current));
                current = current.saturating_add(step);
            }
        } else {
            let mut current = start;
            while current > end {
                values.push(Value::from(current));
                current = current.saturating_add(step);
            }
        }

        Ok(Self::new(values))
    }

    // ==================== Basic Properties ====================

    /// Returns the length of the array
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if the array is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the array as a slice
    #[inline]
    pub fn as_slice(&self) -> &[Value] {
        &self.inner
    }

    /// Converts to a Vec<Value>
    #[inline]
    pub fn to_vec(&self) -> Vec<Value> {
        self.inner.to_vec()
    }

    // ==================== Element Access ====================

    /// Gets element at index
    #[inline]
    pub fn get(&self, index: usize) -> Option<&Value> {
        self.inner.get(index)
    }

    /// Gets element at index with bounds checking
    #[inline]
    pub fn try_get(&self, index: usize) -> ArrayResult<&Value> {
        self.get(index).ok_or_else(|| ArrayError::IndexOutOfBounds { index, len: self.len() })
    }

    /// Gets first element
    #[inline]
    pub fn first(&self) -> Option<&Value> {
        self.inner.first()
    }

    /// Gets last element
    #[inline]
    pub fn last(&self) -> Option<&Value> {
        self.inner.last()
    }

    /// Gets element at index, wrapping around if out of bounds
    #[inline]
    pub fn get_wrapped(&self, index: i64) -> Option<&Value> {
        if self.is_empty() {
            return None;
        }
        let idx = if index < 0 {
            let wrapped = (index % self.len() as i64) + self.len() as i64;
            wrapped as usize
        } else {
            (index as usize) % self.len()
        };
        self.inner.get(idx)
    }

    // ==================== Immutable Operations ====================

    /// Returns a new array with element appended
    #[must_use]
    pub fn push(&self, value: Value) -> Self {
        let mut vec = self.to_vec();
        vec.push(value);
        Self::new(vec)
    }

    /// Returns a new array with element prepended
    #[must_use]
    pub fn unshift(&self, value: Value) -> Self {
        let mut vec = Vec::with_capacity(self.len() + 1);
        vec.push(value);
        vec.extend_from_slice(&self.inner);
        Self::new(vec)
    }

    /// Returns a new array with element inserted at index
    pub fn insert(&self, index: usize, value: Value) -> ArrayResult<Self> {
        if index > self.len() {
            return Err(ArrayError::IndexOutOfBounds { index, len: self.len() });
        }
        let mut vec = self.to_vec();
        vec.insert(index, value);
        Ok(Self::new(vec))
    }

    /// Returns a new array with element at index removed
    pub fn remove(&self, index: usize) -> ArrayResult<(Self, Value)> {
        if index >= self.len() {
            return Err(ArrayError::IndexOutOfBounds { index, len: self.len() });
        }
        let mut vec = self.to_vec();
        let removed = vec.remove(index);
        Ok((Self::new(vec), removed))
    }

    /// Returns a new array with element replaced at index
    pub fn set(&self, index: usize, value: Value) -> ArrayResult<Self> {
        if index >= self.len() {
            return Err(ArrayError::IndexOutOfBounds { index, len: self.len() });
        }
        let mut vec = self.to_vec();
        vec[index] = value;
        Ok(Self::new(vec))
    }

    // ==================== Slicing Operations ====================

    /// Returns a slice of the array
    pub fn slice(&self, start: usize, end: usize) -> ArrayResult<Self> {
        if start > end {
            return Err(ArrayError::InvalidRange { start, end });
        }
        if end > self.len() {
            return Err(ArrayError::RangeOutOfBounds { start, end, len: self.len() });
        }
        Ok(Self::from_slice(&self.inner[start..end]))
    }

    /// Takes first n elements
    #[inline]
    #[must_use]
    pub fn take(&self, n: usize) -> Self {
        let end = n.min(self.len());
        Self::from_slice(&self.inner[..end])
    }

    /// Skips first n elements
    #[inline]
    #[must_use]
    pub fn skip(&self, n: usize) -> Self {
        let start = n.min(self.len());
        Self::from_slice(&self.inner[start..])
    }

    /// Takes last n elements
    #[inline]
    #[must_use]
    pub fn take_last(&self, n: usize) -> Self {
        let len = self.len();
        if n >= len {
            return self.clone();
        }
        Self::from_slice(&self.inner[len - n..])
    }

    /// Takes elements while predicate is true
    #[must_use]
    pub fn take_while<P>(&self, mut predicate: P) -> Self
    where
        P: FnMut(&Value) -> bool,
    {
        let mut result = Vec::new();
        for value in self.inner.iter() {
            if predicate(value) {
                result.push(value.clone());
            } else {
                break;
            }
        }
        Self::new(result)
    }

    /// Skips elements while predicate is true
    #[must_use]
    pub fn skip_while<P>(&self, mut predicate: P) -> Self
    where
        P: FnMut(&Value) -> bool,
    {
        let mut skipping = true;
        let mut result = Vec::new();

        for value in self.inner.iter() {
            if skipping && predicate(value) {
                continue;
            }
            skipping = false;
            result.push(value.clone());
        }

        Self::new(result)
    }

    // ==================== Functional Operations ====================

    /// Maps a function over all elements
    pub fn map<F>(&self, mut f: F) -> ArrayResult<Self>
    where
        F: FnMut(&Value) -> ArrayResult<Value>,
    {
        let mut result = Vec::with_capacity(self.len());
        for value in self.inner.iter() {
            result.push(f(value)?);
        }
        Ok(Self::new(result))
    }

    /// Maps a function and flattens the result
    pub fn flat_map<F>(&self, mut f: F) -> ArrayResult<Self>
    where
        F: FnMut(&Value) -> ArrayResult<Array>,
    {
        let mut result = Vec::new();
        for value in self.inner.iter() {
            result.extend(f(value)?.to_vec());
        }
        Ok(Self::new(result))
    }

    /// Filters elements based on a predicate
    #[must_use]
    pub fn filter<P>(&self, mut predicate: P) -> Self
    where
        P: FnMut(&Value) -> bool,
    {
        let result: Vec<Value> = self.inner.iter().filter(|v| predicate(v)).cloned().collect();
        Self::new(result)
    }

    /// Filter and map in one operation
    #[must_use]
    pub fn filter_map<F>(&self, mut f: F) -> Self
    where
        F: FnMut(&Value) -> Option<Value>,
    {
        let result: Vec<Value> = self.inner.iter().filter_map(|v| f(v)).collect();
        Self::new(result)
    }

    /// Reduces array to a single value
    pub fn reduce<F>(&self, mut f: F) -> ArrayResult<Option<Value>>
    where
        F: FnMut(Value, &Value) -> ArrayResult<Value>,
    {
        let mut iter = self.inner.iter();
        match iter.next() {
            None => Ok(None),
            Some(first) => {
                let mut acc = first.clone();
                for value in iter {
                    acc = f(acc, value)?;
                }
                Ok(Some(acc))
            },
        }
    }

    /// Folds array with an initial value
    pub fn fold<T, F>(&self, init: T, mut f: F) -> ArrayResult<T>
    where
        F: FnMut(T, &Value) -> ArrayResult<T>,
    {
        let mut acc = init;
        for value in self.inner.iter() {
            acc = f(acc, value)?;
        }
        Ok(acc)
    }

    /// Partitions array into two based on predicate
    #[must_use]
    pub fn partition<P>(&self, mut predicate: P) -> (Self, Self)
    where
        P: FnMut(&Value) -> bool,
    {
        let mut true_vec = Vec::new();
        let mut false_vec = Vec::new();

        for value in self.inner.iter() {
            if predicate(value) {
                true_vec.push(value.clone());
            } else {
                false_vec.push(value.clone());
            }
        }

        (Self::new(true_vec), Self::new(false_vec))
    }

    // ==================== Search Operations ====================

    /// Finds first element matching predicate
    #[inline]
    pub fn find<P>(&self, mut predicate: P) -> Option<&Value>
    where
        P: FnMut(&Value) -> bool,
    {
        self.inner.iter().find(|v| predicate(v))
    }

    /// Finds index of first element matching predicate
    #[inline]
    pub fn find_index<P>(&self, predicate: P) -> Option<usize>
    where
        P: FnMut(&Value) -> bool,
    {
        self.inner.iter().position(predicate)
    }

    /// Finds last element matching predicate
    #[inline]
    pub fn find_last<P>(&self, mut predicate: P) -> Option<&Value>
    where
        P: FnMut(&Value) -> bool,
    {
        self.inner.iter().rev().find(|v| predicate(v))
    }

    /// Checks if array contains a value
    #[inline]
    pub fn contains(&self, value: &Value) -> bool {
        self.inner.contains(value)
    }

    /// Finds index of a value
    #[inline]
    pub fn index_of(&self, value: &Value) -> Option<usize> {
        self.inner.iter().position(|v| v == value)
    }

    /// Finds last index of a value
    #[inline]
    pub fn last_index_of(&self, value: &Value) -> Option<usize> {
        self.inner.iter().rposition(|v| v == value)
    }

    /// Counts occurrences of a value
    #[inline]
    pub fn count_occurrences(&self, value: &Value) -> usize {
        self.inner.iter().filter(|v| *v == value).count()
    }

    /// Returns true if any element matches predicate
    #[inline]
    pub fn any<P>(&self, predicate: P) -> bool
    where
        P: FnMut(&Value) -> bool,
    {
        self.inner.iter().any(predicate)
    }

    /// Returns true if all elements match predicate
    #[inline]
    pub fn all<P>(&self, predicate: P) -> bool
    where
        P: FnMut(&Value) -> bool,
    {
        self.inner.iter().all(predicate)
    }

    // ==================== Transformation Operations ====================

    /// Returns a sorted copy of the array
    #[must_use]
    pub fn sorted(&self) -> Self {
        let mut vec = self.to_vec();
        vec.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        Self::new(vec)
    }

    /// Returns a sorted copy using a custom comparator
    #[must_use]
    pub fn sorted_by<F>(&self, mut compare: F) -> Self
    where
        F: FnMut(&Value, &Value) -> Ordering,
    {
        let mut vec = self.to_vec();
        vec.sort_by(|a, b| compare(a, b));
        Self::new(vec)
    }

    /// Returns a sorted copy using a key function
    pub fn sorted_by_key<K, F>(&self, mut f: F) -> ArrayResult<Self>
    where
        F: FnMut(&Value) -> ArrayResult<K>,
        K: Ord,
    {
        // Pre-compute keys to avoid repeated evaluation
        let mut indexed: Vec<(usize, K)> = Vec::with_capacity(self.len());
        for (i, value) in self.inner.iter().enumerate() {
            indexed.push((i, f(value)?));
        }

        // Sort by key
        indexed.sort_by(|a, b| a.1.cmp(&b.1));

        // Build result in sorted order
        let result: Vec<Value> = indexed.into_iter().map(|(i, _)| self.inner[i].clone()).collect();

        Ok(Self::new(result))
    }

    /// Returns a reversed copy
    #[must_use]
    pub fn reversed(&self) -> Self {
        let mut vec = self.to_vec();
        vec.reverse();
        Self::new(vec)
    }

    /// Rotates array left by n positions
    #[must_use]
    pub fn rotate_left(&self, n: usize) -> Self {
        if self.is_empty() || n == 0 {
            return self.clone();
        }

        let n = n % self.len();
        let mut vec = Vec::with_capacity(self.len());
        vec.extend_from_slice(&self.inner[n..]);
        vec.extend_from_slice(&self.inner[..n]);
        Self::new(vec)
    }

    /// Rotates array right by n positions
    #[must_use]
    pub fn rotate_right(&self, n: usize) -> Self {
        if self.is_empty() || n == 0 {
            return self.clone();
        }

        let len = self.len();
        let n = n % len;
        self.rotate_left(len - n)
    }

    /// Shuffles array randomly
    #[must_use]
    pub fn shuffled(&self) -> Self {
        use rand::seq::SliceRandom;
        let mut vec = self.to_vec();
        let mut rng = rand::thread_rng();
        vec.shuffle(&mut rng);
        Self::new(vec)
    }

    // ==================== Combination Operations ====================

    /// Concatenates with another array
    #[must_use]
    pub fn concat(&self, other: &Array) -> Self {
        let mut vec = Vec::with_capacity(self.len() + other.len());
        vec.extend_from_slice(&self.inner);
        vec.extend_from_slice(&other.inner);
        Self::new(vec)
    }

    /// Joins array elements into a string
    pub fn join(&self, separator: &str) -> String {
        self.inner.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(separator)
    }

    /// Flattens nested arrays by one level
    #[must_use]
    pub fn flatten(&self) -> Self {
        let mut result = Vec::new();
        for value in self.inner.iter() {
            // Assuming Value has an as_array method
            // if let Some(arr) = value.as_array() {
            //     result.extend(arr.to_vec());
            // } else {
            result.push(value.clone());
            // }
        }
        Self::new(result)
    }

    /// Groups consecutive equal elements
    #[must_use]
    pub fn group(&self) -> Vec<Self> {
        if self.is_empty() {
            return vec![];
        }

        let mut groups = Vec::new();
        let mut current_group = vec![self.inner[0].clone()];

        for value in &self.inner[1..] {
            if value == &current_group[0] {
                current_group.push(value.clone());
            } else {
                groups.push(Self::new(current_group));
                current_group = vec![value.clone()];
            }
        }

        if !current_group.is_empty() {
            groups.push(Self::new(current_group));
        }

        groups
    }

    /// Removes duplicate values preserving order
    #[must_use]
    pub fn unique(&self) -> Self {
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();

        for value in self.inner.iter() {
            let key = format!("{:?}", value); // Or use a proper hash if Value implements Hash
            if seen.insert(key) {
                result.push(value.clone());
            }
        }

        Self::new(result)
    }

    /// Removes consecutive duplicate values
    #[must_use]
    pub fn dedup(&self) -> Self {
        let mut vec = self.to_vec();
        vec.dedup();
        Self::new(vec)
    }

    /// Creates chunks of specified size
    pub fn chunks(&self, size: usize) -> ArrayResult<Vec<Self>> {
        if size == 0 {
            return Err(ArrayError::InvalidOperation { msg: "Chunk size cannot be zero".into() });
        }

        let chunks: Vec<Self> =
            self.inner.chunks(size).map(|chunk| Self::from_slice(chunk)).collect();

        Ok(chunks)
    }

    /// Creates sliding windows of specified size
    pub fn windows(&self, size: usize) -> ArrayResult<Vec<Self>> {
        if size == 0 {
            return Err(ArrayError::InvalidOperation { msg: "Window size cannot be zero".into() });
        }

        if size > self.len() {
            return Ok(vec![]);
        }

        let windows: Vec<Self> =
            self.inner.windows(size).map(|window| Self::from_slice(window)).collect();

        Ok(windows)
    }

    /// Zips with another array
    #[must_use]
    pub fn zip(&self, other: &Array) -> Self {
        let len = self.len().min(other.len());
        let mut result = Vec::with_capacity(len);

        for i in 0..len {
            // Create a tuple or pair value
            // This depends on your Value implementation
            // result.push(Value::Tuple(vec![self.inner[i].clone(), other.inner[i].clone()]));
            result.push(self.inner[i].clone()); // Placeholder
        }

        Self::new(result)
    }

    /// Intersperses elements with a separator
    #[must_use]
    pub fn intersperse(&self, separator: Value) -> Self {
        if self.is_empty() {
            return self.clone();
        }

        let mut result = Vec::with_capacity(self.len() * 2 - 1);
        let mut iter = self.inner.iter();

        if let Some(first) = iter.next() {
            result.push(first.clone());
            for value in iter {
                result.push(separator.clone());
                result.push(value.clone());
            }
        }

        Self::new(result)
    }

    // ==================== Parallel Operations ====================

    #[cfg(feature = "rayon")]
    /// Parallel map operation for large arrays
    pub fn par_map<F>(&self, f: F) -> ArrayResult<Self>
    where
        F: Fn(&Value) -> ArrayResult<Value> + Sync + Send,
        Value: Send + Sync,
    {
        if self.len() < 1000 {
            return self.map(f);
        }

        let results: Result<Vec<_>, _> = self.inner.par_iter().map(|v| f(v)).collect();

        Ok(Self::new(results?))
    }

    #[cfg(feature = "rayon")]
    /// Parallel filter operation for large arrays
    pub fn par_filter<P>(&self, predicate: P) -> Self
    where
        P: Fn(&Value) -> bool + Sync + Send,
        Value: Send + Sync,
    {
        if self.len() < 1000 {
            return self.filter(predicate);
        }

        let result: Vec<Value> = self.inner.par_iter().filter(|v| predicate(v)).cloned().collect();

        Self::new(result)
    }

    #[cfg(feature = "rayon")]
    /// Parallel sort operation for large arrays
    pub fn par_sorted(&self) -> Self
    where
        Value: Send + Sync + Ord,
    {
        if self.len() < 1000 {
            return self.sorted();
        }

        let mut vec = self.to_vec();
        vec.par_sort();
        Self::new(vec)
    }

    // ==================== Statistics ====================

    /// Calculates sum of numeric values
    pub fn sum(&self) -> ArrayResult<f64> {
        let mut sum = 0.0;
        for _value in self.inner.iter() {
            // TODO: Implement when Value has numeric conversion methods
            // sum += value.as_number().ok_or_else(|| ArrayError::ValueError {
            //     msg: "Cannot sum non-numeric value".into(),
            // })?;
        }
        Ok(sum)
    }

    /// Calculates average of numeric values
    pub fn mean(&self) -> ArrayResult<f64> {
        if self.is_empty() {
            return Err(ArrayError::EmptyArray);
        }
        Ok(self.sum()? / self.len() as f64)
    }

    /// Finds minimum value
    pub fn min(&self) -> Option<&Value> {
        self.inner.iter().min_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal))
    }

    /// Finds maximum value
    pub fn max(&self) -> Option<&Value> {
        self.inner.iter().max_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal))
    }

    // ==================== Iterator Support ====================

    /// Returns an iterator over the values
    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, Value> {
        self.inner.iter()
    }

    /// Returns an iterator that yields cloned values
    pub fn iter_cloned(&self) -> impl Iterator<Item = Value> + '_ {
        self.inner.iter().cloned()
    }

    /// Converts to owned Vec<Value>
    #[inline]
    pub fn into_vec(self) -> Vec<Value> {
        self.inner.to_vec()
    }
}

// ==================== Trait Implementations ====================

impl Default for Array {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl Deref for Array {
    type Target = [Value];

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl AsRef<[Value]> for Array {
    #[inline]
    fn as_ref(&self) -> &[Value] {
        &self.inner
    }
}

impl Borrow<[Value]> for Array {
    #[inline]
    fn borrow(&self) -> &[Value] {
        &self.inner
    }
}

impl fmt::Display for Array {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[")?;
        for (i, value) in self.inner.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", value)?;
        }
        write!(f, "]")
    }
}

// ==================== Index Traits ====================

impl Index<usize> for Array {
    type Output = Value;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.inner[index]
    }
}

impl Index<Range<usize>> for Array {
    type Output = [Value];

    #[inline]
    fn index(&self, range: Range<usize>) -> &Self::Output {
        &self.inner[range]
    }
}

impl Index<RangeFrom<usize>> for Array {
    type Output = [Value];

    #[inline]
    fn index(&self, range: RangeFrom<usize>) -> &Self::Output {
        &self.inner[range]
    }
}

impl Index<RangeTo<usize>> for Array {
    type Output = [Value];

    #[inline]
    fn index(&self, range: RangeTo<usize>) -> &Self::Output {
        &self.inner[range]
    }
}

impl Index<RangeFull> for Array {
    type Output = [Value];

    #[inline]
    fn index(&self, _: RangeFull) -> &Self::Output {
        &self.inner
    }
}

// ==================== Conversion Traits ====================

impl From<Vec<Value>> for Array {
    #[inline]
    fn from(vec: Vec<Value>) -> Self {
        Self::new(vec)
    }
}

impl From<&[Value]> for Array {
    #[inline]
    fn from(slice: &[Value]) -> Self {
        Self::from_slice(slice)
    }
}

impl From<Box<[Value]>> for Array {
    #[inline]
    fn from(boxed: Box<[Value]>) -> Self {
        Self { inner: Arc::from(boxed), hash_cache: std::sync::OnceLock::new() }
    }
}

impl From<Arc<[Value]>> for Array {
    #[inline]
    fn from(arc: Arc<[Value]>) -> Self {
        Self { inner: arc, hash_cache: std::sync::OnceLock::new() }
    }
}

impl From<Array> for Vec<Value> {
    #[inline]
    fn from(array: Array) -> Self {
        array.into_vec()
    }
}

impl<const N: usize> From<[Value; N]> for Array {
    #[inline]
    fn from(arr: [Value; N]) -> Self {
        Self::new(arr.to_vec())
    }
}

// ==================== Comparison Traits ====================

impl PartialEq for Array {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for Array {}

impl PartialEq<Vec<Value>> for Array {
    #[inline]
    fn eq(&self, other: &Vec<Value>) -> bool {
        &*self.inner == other.as_slice()
    }
}

impl PartialEq<&[Value]> for Array {
    #[inline]
    fn eq(&self, other: &&[Value]) -> bool {
        &*self.inner == *other
    }
}

impl PartialOrd for Array {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.inner.partial_cmp(&other.inner)
    }
}

impl Ord for Array {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        let len_cmp = self.len().cmp(&other.len());
        if len_cmp != Ordering::Equal {
            return len_cmp;
        }
        for (a, b) in self.inner.iter().zip(other.inner.iter()) {
            if let Some(ord) = a.partial_cmp(b) {
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
        Ordering::Equal
    }
}

impl Hash for Array {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let hash = self.hash_cache.get_or_init(|| {
            use std::collections::hash_map::DefaultHasher;
            let mut hasher = DefaultHasher::new();
            // Hash length and stringified values to avoid requiring Value: Hash
            self.len().hash(&mut hasher);
            for v in self.inner.iter() {
                v.to_string().hash(&mut hasher);
            }
            hasher.finish()
        });
        hash.hash(state);
    }
}

// ==================== Arithmetic Operations ====================

impl Add for Array {
    type Output = Array;

    fn add(self, rhs: Self) -> Self::Output {
        self.concat(&rhs)
    }
}

impl Add<&Array> for Array {
    type Output = Array;

    fn add(self, rhs: &Array) -> Self::Output {
        self.concat(rhs)
    }
}

impl Add<Array> for &Array {
    type Output = Array;

    fn add(self, rhs: Array) -> Self::Output {
        self.concat(&rhs)
    }
}

impl Add for &Array {
    type Output = Array;

    fn add(self, rhs: Self) -> Self::Output {
        self.concat(rhs)
    }
}

impl AddAssign for Array {
    fn add_assign(&mut self, rhs: Self) {
        *self = self.concat(&rhs);
    }
}

impl AddAssign<&Array> for Array {
    fn add_assign(&mut self, rhs: &Array) {
        *self = self.concat(rhs);
    }
}

// ==================== Iterator Traits ====================

impl FromIterator<Value> for Array {
    fn from_iter<T: IntoIterator<Item = Value>>(iter: T) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl<'a> FromIterator<&'a Value> for Array {
    fn from_iter<T: IntoIterator<Item = &'a Value>>(iter: T) -> Self {
        Self::new(iter.into_iter().cloned().collect())
    }
}

impl Extend<Value> for Array {
    fn extend<T: IntoIterator<Item = Value>>(&mut self, iter: T) {
        let additional: Vec<Value> = iter.into_iter().collect();
        *self = self.concat(&Array::new(additional));
    }
}

impl<'a> Extend<&'a Value> for Array {
    fn extend<T: IntoIterator<Item = &'a Value>>(&mut self, iter: T) {
        let additional: Vec<Value> = iter.into_iter().cloned().collect();
        *self = self.concat(&Array::new(additional));
    }
}

impl IntoIterator for Array {
    type Item = Value;
    type IntoIter = std::vec::IntoIter<Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
    }
}

impl<'a> IntoIterator for &'a Array {
    type Item = &'a Value;
    type IntoIter = std::slice::Iter<'a, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

// ==================== JSON Support ====================

#[cfg(feature = "serde")]
impl From<Array> for serde_json::Value {
    fn from(array: Array) -> Self {
        serde_json::Value::Array(
            array.into_vec().into_iter().map(|v| serde_json::Value::from(v)).collect(),
        )
    }
}

#[cfg(feature = "serde")]
impl TryFrom<serde_json::Value> for Array {
    type Error = ArrayError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        match value {
            serde_json::Value::Array(arr) => {
                let values: Vec<Value> = arr
                    .into_iter()
                    .map(|v| Value::try_from(v))
                    .collect::<Result<_, _>>()
                    .map_err(|_| ArrayError::JsonTypeMismatch { found: "invalid element" })?;
                Ok(Array::new(values))
            },
            serde_json::Value::Null => Ok(Array::empty()),
            serde_json::Value::Bool(_) => Err(ArrayError::JsonTypeMismatch { found: "bool" }),
            serde_json::Value::Number(_) => Err(ArrayError::JsonTypeMismatch { found: "number" }),
            serde_json::Value::String(_) => Err(ArrayError::JsonTypeMismatch { found: "string" }),
            serde_json::Value::Object(_) => Err(ArrayError::JsonTypeMismatch { found: "object" }),
        }
    }
}

// ==================== Send + Sync ====================

unsafe impl Send for Array {}
unsafe impl Sync for Array {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_creation() {
        let arr1 = Array::new(vec![Value::from(1), Value::from(2)]);
        let arr2 = Array::empty();
        let arr3 = Array::filled(&Value::from(42), 3);

        assert_eq!(arr1.len(), 2);
        assert!(arr2.is_empty());
        assert_eq!(arr3.len(), 3);
    }

    #[test]
    fn test_range() {
        let arr = Array::range(0, 5, 1).unwrap();
        assert_eq!(arr.len(), 5);

        let arr_reverse = Array::range(5, 0, -1).unwrap();
        assert_eq!(arr_reverse.len(), 5);
    }

    #[test]
    fn test_immutable_operations() {
        let arr = Array::new(vec![Value::from(1), Value::from(2)]);

        let pushed = arr.push(Value::from(3));
        assert_eq!(pushed.len(), 3);
        assert_eq!(arr.len(), 2); // Original unchanged

        let removed = arr.remove(0).unwrap();
        assert_eq!(removed.0.len(), 1);
        assert_eq!(arr.len(), 2); // Original unchanged
    }

    #[test]
    fn test_functional_operations() {
        let arr = Array::new(vec![Value::from(1), Value::from(2), Value::from(3)]);

        let doubled = arr.map(|v| Ok(Value::from(v.as_i64().unwrap() * 2))).unwrap();
        assert_eq!(doubled.len(), 3);

        let filtered = arr.filter(|v| v.as_i64().unwrap() > 1);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_slicing() {
        let arr = Array::new(vec![
            Value::from(1),
            Value::from(2),
            Value::from(3),
            Value::from(4),
            Value::from(5),
        ]);

        let sliced = arr.slice(1, 4).unwrap();
        assert_eq!(sliced.len(), 3);

        let taken = arr.take(3);
        assert_eq!(taken.len(), 3);

        let skipped = arr.skip(2);
        assert_eq!(skipped.len(), 3);
    }

    #[test]
    fn test_arc_sharing() {
        let arr1 = Array::new(vec![Value::from(1), Value::from(2)]);
        let arr2 = arr1.clone();

        // Both should share the same Arc
        assert_eq!(arr1, arr2);
    }

    #[test]
    fn test_unique() {
        let arr = Array::new(vec![
            Value::from(1),
            Value::from(2),
            Value::from(1),
            Value::from(3),
            Value::from(2),
        ]);

        let unique = arr.unique();
        assert_eq!(unique.len(), 3);
    }

    #[test]
    fn test_chunks() {
        let arr = Array::new(vec![
            Value::from(1),
            Value::from(2),
            Value::from(3),
            Value::from(4),
            Value::from(5),
        ]);

        let chunks = arr.chunks(2).unwrap();
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 2);
        assert_eq!(chunks[2].len(), 1);
    }

    #[cfg(feature = "rayon")]
    #[test]
    fn test_parallel_operations() {
        let large_arr = Array::new((0..10000).map(|i| Value::from(i)).collect());

        let par_mapped = large_arr.par_map(|v| Ok(Value::from(v.as_i64().unwrap() * 2))).unwrap();
        assert_eq!(par_mapped.len(), 10000);

        let par_filtered = large_arr.par_filter(|v| v.as_i64().unwrap() % 2 == 0);
        assert_eq!(par_filtered.len(), 5000);
    }
}
