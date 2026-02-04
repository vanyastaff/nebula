//! Iterators for traversing and transforming Value structures
//!
//! This module provides powerful iteration capabilities for working with
//! nested Value structures:
//!
//! - [`ValueWalker`] - Depth-first traversal of all nested values
//! - [`Value::walk`] - Iterate over all values with their paths
//! - [`Value::find`] - Find first value matching a predicate
//! - [`Value::find_all`] - Find all values matching a predicate
//! - [`Value::map_values`] - Transform all leaf values
//!
//! # Examples
//!
//! ## Walking all values
//!
//! ```
//! use nebula_value::Value;
//! use nebula_value::collections::Object;
//!
//! let obj = Object::from_iter(vec![
//!     ("name".to_string(), Value::text("Alice")),
//!     ("age".to_string(), Value::integer(30)),
//! ]);
//! let root = Value::Object(obj);
//!
//! for (path, value) in root.walk() {
//!     println!("{}: {:?}", path, value.kind());
//! }
//! ```
//!
//! ## Finding values
//!
//! ```
//! use nebula_value::Value;
//! use nebula_value::collections::{Array, Object};
//!
//! let data = Object::from_iter(vec![
//!     ("users".to_string(), Value::Array(Array::from_vec(vec![
//!         Value::integer(1),
//!         Value::integer(2),
//!         Value::text("admin"),
//!     ]))),
//! ]);
//! let root = Value::Object(data);
//!
//! // Find first text value
//! if let Some((path, value)) = root.find(|v| v.is_text()) {
//!     println!("Found text at {}: {:?}", path, value);
//! }
//! ```

use crate::collections::{Array, Object};
use crate::core::path::Path;
use crate::core::value::Value;

// ============================================================================
// VALUE WALKER
// ============================================================================

/// Iterator that walks through all values in a nested structure
///
/// Performs depth-first traversal, yielding each value along with its path.
/// The root value is yielded first with an empty path.
///
/// # Traversal Order
///
/// For objects, keys are visited in their natural iteration order (not sorted).
/// For arrays, elements are visited in index order.
///
/// # Examples
///
/// ```
/// use nebula_value::Value;
/// use nebula_value::collections::Object;
///
/// let obj = Object::from_iter(vec![
///     ("a".to_string(), Value::integer(1)),
///     ("b".to_string(), Value::integer(2)),
/// ]);
/// let root = Value::Object(obj);
///
/// let paths: Vec<_> = root.walk().map(|(p, _)| p.to_string()).collect();
/// // Contains: "", "a", "b" (root and both keys)
/// ```
pub struct ValueWalker<'a> {
    /// Stack of (path, value) pairs to visit
    stack: Vec<(Path, &'a Value)>,
    /// Whether to include the root value
    include_root: bool,
    /// Whether root has been yielded
    root_yielded: bool,
}

impl<'a> ValueWalker<'a> {
    /// Create a new walker starting from the given value
    pub fn new(value: &'a Value) -> Self {
        Self {
            stack: vec![(Path::new(), value)],
            include_root: true,
            root_yielded: false,
        }
    }

    /// Create a walker that skips the root value
    pub fn without_root(value: &'a Value) -> Self {
        let mut walker = Self::new(value);
        walker.include_root = false;
        walker
    }

    /// Create a walker for only leaf values (non-containers)
    pub fn leaves_only(value: &'a Value) -> LeavesWalker<'a> {
        LeavesWalker {
            inner: Self::new(value),
        }
    }
}

impl<'a> Iterator for ValueWalker<'a> {
    type Item = (Path, &'a Value);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((path, value)) = self.stack.pop() {
            // Check if this is root
            let is_root = path.is_root() && !self.root_yielded;
            if is_root {
                self.root_yielded = true;
            }

            // Push children onto stack (in reverse order for correct traversal)
            match value {
                Value::Object(obj) => {
                    // Collect keys and reverse for stack order
                    let entries: Vec<_> = obj.entries().collect();
                    for (key, child) in entries.into_iter().rev() {
                        self.stack.push((path.key(key), child));
                    }
                }
                Value::Array(arr) => {
                    // Push in reverse order - collect first since iter() doesn't implement DoubleEndedIterator
                    let items: Vec<_> = arr.iter().enumerate().collect();
                    for (i, child) in items.into_iter().rev() {
                        self.stack.push((path.index(i), child));
                    }
                }
                _ => {}
            }

            // Skip root if configured
            if is_root && !self.include_root {
                continue;
            }

            return Some((path, value));
        }

        None
    }
}

// ============================================================================
// LEAVES WALKER
// ============================================================================

/// Iterator that yields only leaf values (non-container types)
///
/// Skips Object and Array values, only yielding scalar and temporal values.
pub struct LeavesWalker<'a> {
    inner: ValueWalker<'a>,
}

impl<'a> Iterator for LeavesWalker<'a> {
    type Item = (Path, &'a Value);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.inner.next() {
                Some((path, value)) if !value.is_collection() => {
                    return Some((path, value));
                }
                Some(_) => continue, // Skip collections
                None => return None,
            }
        }
    }
}

// ============================================================================
// VALUE METHODS
// ============================================================================

impl Value {
    /// Walk through all values in a depth-first manner
    ///
    /// Returns an iterator that yields `(Path, &Value)` pairs for every
    /// value in the structure, including nested values.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::Object;
    ///
    /// let obj = Object::from_iter(vec![
    ///     ("name".to_string(), Value::text("Alice")),
    /// ]);
    /// let root = Value::Object(obj);
    ///
    /// let count = root.walk().count();
    /// assert_eq!(count, 2); // root object + "name" value
    /// ```
    pub fn walk(&self) -> ValueWalker<'_> {
        ValueWalker::new(self)
    }

    /// Walk through all values, excluding the root
    ///
    /// Useful when you only want to process children.
    pub fn walk_children(&self) -> ValueWalker<'_> {
        ValueWalker::without_root(self)
    }

    /// Walk through only leaf values (non-containers)
    ///
    /// Skips Object and Array values, yielding only scalars and temporals.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::{Array, Object};
    ///
    /// let obj = Object::from_iter(vec![
    ///     ("nums".to_string(), Value::Array(Array::from_vec(vec![
    ///         Value::integer(1),
    ///         Value::integer(2),
    ///     ]))),
    /// ]);
    /// let root = Value::Object(obj);
    ///
    /// let leaves: Vec<_> = root.walk_leaves().collect();
    /// assert_eq!(leaves.len(), 2); // Only the two integers
    /// ```
    pub fn walk_leaves(&self) -> LeavesWalker<'_> {
        ValueWalker::leaves_only(self)
    }

    /// Find the first value matching a predicate
    ///
    /// Performs depth-first search and returns the first match.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::{Array, Object};
    ///
    /// let obj = Object::from_iter(vec![
    ///     ("items".to_string(), Value::Array(Array::from_vec(vec![
    ///         Value::integer(1),
    ///         Value::text("hello"),
    ///         Value::integer(3),
    ///     ]))),
    /// ]);
    /// let root = Value::Object(obj);
    ///
    /// let result = root.find(|v| v.is_text());
    /// assert!(result.is_some());
    /// assert_eq!(result.unwrap().0.to_string(), "items[1]");
    /// ```
    pub fn find<F>(&self, predicate: F) -> Option<(Path, &Value)>
    where
        F: Fn(&Value) -> bool,
    {
        self.walk().find(|(_, v)| predicate(v))
    }

    /// Find all values matching a predicate
    ///
    /// Returns a vector of all matches with their paths.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::{Array, Object};
    ///
    /// let obj = Object::from_iter(vec![
    ///     ("nums".to_string(), Value::Array(Array::from_vec(vec![
    ///         Value::integer(1),
    ///         Value::integer(2),
    ///         Value::integer(3),
    ///     ]))),
    /// ]);
    /// let root = Value::Object(obj);
    ///
    /// let integers = root.find_all(|v| v.is_integer());
    /// assert_eq!(integers.len(), 3);
    /// ```
    pub fn find_all<F>(&self, predicate: F) -> Vec<(Path, &Value)>
    where
        F: Fn(&Value) -> bool,
    {
        self.walk().filter(|(_, v)| predicate(v)).collect()
    }

    /// Find values by type
    ///
    /// Convenience method for finding all values of a specific kind.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::{Value, ValueKind};
    /// use nebula_value::collections::Object;
    ///
    /// let obj = Object::from_iter(vec![
    ///     ("name".to_string(), Value::text("Alice")),
    ///     ("age".to_string(), Value::integer(30)),
    /// ]);
    /// let root = Value::Object(obj);
    ///
    /// let texts = root.find_by_kind(ValueKind::String);
    /// assert_eq!(texts.len(), 1);
    /// ```
    pub fn find_by_kind(&self, kind: crate::ValueKind) -> Vec<(Path, &Value)> {
        self.find_all(|v| v.kind() == kind)
    }

    /// Transform all values using a mapping function
    ///
    /// Creates a new Value with all leaf values transformed.
    /// The structure (objects and arrays) is preserved.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::Object;
    ///
    /// let obj = Object::from_iter(vec![
    ///     ("count".to_string(), Value::integer(5)),
    /// ]);
    /// let root = Value::Object(obj);
    ///
    /// // Double all integers
    /// let doubled = root.map_values(|v| {
    ///     if let Some(i) = v.as_integer() {
    ///         Value::integer(i.value() * 2)
    ///     } else {
    ///         v.clone()
    ///     }
    /// });
    ///
    /// assert_eq!(doubled.get_path("count").unwrap(), Value::integer(10));
    /// ```
    pub fn map_values<F>(&self, transform: F) -> Value
    where
        F: Fn(&Value) -> Value + Copy,
    {
        self.map_values_recursive(transform)
    }

    /// Internal recursive implementation of map_values
    fn map_values_recursive<F>(&self, transform: F) -> Value
    where
        F: Fn(&Value) -> Value + Copy,
    {
        match self {
            Value::Object(obj) => {
                let new_obj = obj
                    .entries()
                    .map(|(k, v)| (k.clone(), v.map_values_recursive(transform)))
                    .collect::<Object>();
                Value::Object(new_obj)
            }
            Value::Array(arr) => {
                let new_arr = arr
                    .iter()
                    .map(|v| v.map_values_recursive(transform))
                    .collect::<Array>();
                Value::Array(new_arr)
            }
            // For non-container types, apply transform
            _ => transform(self),
        }
    }

    /// Transform values at specific paths
    ///
    /// Only transforms values whose paths match the predicate.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::Object;
    ///
    /// let obj = Object::from_iter(vec![
    ///     ("public".to_string(), Value::text("visible")),
    ///     ("secret".to_string(), Value::text("hidden")),
    /// ]);
    /// let root = Value::Object(obj);
    ///
    /// // Redact values at paths containing "secret"
    /// let redacted = root.map_at(
    ///     |path| path.to_string().contains("secret"),
    ///     |_| Value::text("[REDACTED]"),
    /// );
    ///
    /// assert_eq!(redacted.get_path("secret").unwrap(), Value::text("[REDACTED]"));
    /// assert_eq!(redacted.get_path("public").unwrap(), Value::text("visible"));
    /// ```
    pub fn map_at<P, F>(&self, path_predicate: P, transform: F) -> Value
    where
        P: Fn(&Path) -> bool + Copy,
        F: Fn(&Value) -> Value + Copy,
    {
        self.map_at_recursive(&Path::new(), path_predicate, transform)
    }

    /// Internal recursive implementation of map_at
    fn map_at_recursive<P, F>(&self, current_path: &Path, path_predicate: P, transform: F) -> Value
    where
        P: Fn(&Path) -> bool + Copy,
        F: Fn(&Value) -> Value + Copy,
    {
        // Check if this path should be transformed
        if path_predicate(current_path) {
            return transform(self);
        }

        match self {
            Value::Object(obj) => {
                let new_obj = obj
                    .entries()
                    .map(|(k, v)| {
                        let child_path = current_path.key(k);
                        (
                            k.clone(),
                            v.map_at_recursive(&child_path, path_predicate, transform),
                        )
                    })
                    .collect::<Object>();
                Value::Object(new_obj)
            }
            Value::Array(arr) => {
                let new_arr = arr
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let child_path = current_path.index(i);
                        v.map_at_recursive(&child_path, path_predicate, transform)
                    })
                    .collect::<Array>();
                Value::Array(new_arr)
            }
            _ => self.clone(),
        }
    }

    /// Filter object keys or array elements
    ///
    /// For objects, keeps only keys where the predicate returns true.
    /// For arrays, keeps only elements where the predicate returns true.
    /// Other types are returned unchanged.
    ///
    /// Note: This only filters the top level. Use `filter_deep` for recursive filtering.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::Object;
    ///
    /// let obj = Object::from_iter(vec![
    ///     ("name".to_string(), Value::text("Alice")),
    ///     ("_internal".to_string(), Value::text("hidden")),
    ///     ("age".to_string(), Value::integer(30)),
    /// ]);
    /// let root = Value::Object(obj);
    ///
    /// // Remove keys starting with underscore
    /// let filtered = root.filter(|path, _| {
    ///     !path.to_string().starts_with("_")
    /// });
    ///
    /// assert!(filtered.has_path("name"));
    /// assert!(!filtered.has_path("_internal"));
    /// ```
    pub fn filter<F>(&self, predicate: F) -> Value
    where
        F: Fn(&Path, &Value) -> bool,
    {
        match self {
            Value::Object(obj) => {
                let filtered = obj
                    .entries()
                    .filter(|(k, v)| {
                        let path = Path::new().key(k.as_str());
                        predicate(&path, v)
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<Object>();
                Value::Object(filtered)
            }
            Value::Array(arr) => {
                let filtered = arr
                    .iter()
                    .enumerate()
                    .filter(|(i, v)| {
                        let path = Path::new().index(*i);
                        predicate(&path, v)
                    })
                    .map(|(_, v)| v.clone())
                    .collect::<Array>();
                Value::Array(filtered)
            }
            _ => self.clone(),
        }
    }

    /// Recursively filter all nested values
    ///
    /// Removes any value (at any nesting level) where the predicate returns false.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::{Array, Object};
    ///
    /// let inner = Object::from_iter(vec![
    ///     ("keep".to_string(), Value::integer(1)),
    ///     ("_remove".to_string(), Value::integer(2)),
    /// ]);
    /// let outer = Object::from_iter(vec![
    ///     ("nested".to_string(), Value::Object(inner)),
    /// ]);
    /// let root = Value::Object(outer);
    ///
    /// // Remove all keys starting with underscore at any level
    /// let filtered = root.filter_deep(|path, _| {
    ///     path.last()
    ///         .and_then(|s| s.as_key())
    ///         .map(|k| !k.starts_with("_"))
    ///         .unwrap_or(true)
    /// });
    ///
    /// assert!(filtered.has_path("nested.keep"));
    /// assert!(!filtered.has_path("nested._remove"));
    /// ```
    pub fn filter_deep<F>(&self, predicate: F) -> Value
    where
        F: Fn(&Path, &Value) -> bool + Copy,
    {
        self.filter_deep_recursive(&Path::new(), predicate)
    }

    /// Internal recursive implementation of filter_deep
    fn filter_deep_recursive<F>(&self, current_path: &Path, predicate: F) -> Value
    where
        F: Fn(&Path, &Value) -> bool + Copy,
    {
        match self {
            Value::Object(obj) => {
                let filtered = obj
                    .entries()
                    .filter(|(k, v)| {
                        let child_path = current_path.key(k.as_str());
                        predicate(&child_path, v)
                    })
                    .map(|(k, v)| {
                        let child_path = current_path.key(k.as_str());
                        (k.clone(), v.filter_deep_recursive(&child_path, predicate))
                    })
                    .collect::<Object>();
                Value::Object(filtered)
            }
            Value::Array(arr) => {
                let filtered = arr
                    .iter()
                    .enumerate()
                    .filter(|(i, v)| {
                        let child_path = current_path.index(*i);
                        predicate(&child_path, v)
                    })
                    .map(|(i, v)| {
                        let child_path = current_path.index(i);
                        v.filter_deep_recursive(&child_path, predicate)
                    })
                    .collect::<Array>();
                Value::Array(filtered)
            }
            _ => self.clone(),
        }
    }

    /// Count all values (including nested)
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::{Array, Object};
    ///
    /// let arr = Array::from_vec(vec![Value::integer(1), Value::integer(2)]);
    /// let obj = Object::from_iter(vec![
    ///     ("items".to_string(), Value::Array(arr)),
    /// ]);
    /// let root = Value::Object(obj);
    ///
    /// assert_eq!(root.count_values(), 4); // root + items array + 2 integers
    /// ```
    pub fn count_values(&self) -> usize {
        self.walk().count()
    }

    /// Count only leaf values (non-containers)
    pub fn count_leaves(&self) -> usize {
        self.walk_leaves().count()
    }

    /// Get the maximum nesting depth
    ///
    /// Returns 0 for scalars, 1 for flat collections, etc.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::{Array, Object};
    ///
    /// assert_eq!(Value::integer(42).depth(), 0);
    ///
    /// let flat = Object::from_iter(vec![
    ///     ("a".to_string(), Value::integer(1)),
    /// ]);
    /// assert_eq!(Value::Object(flat).depth(), 1);
    ///
    /// let nested = Object::from_iter(vec![
    ///     ("inner".to_string(), Value::Object(Object::from_iter(vec![
    ///         ("deep".to_string(), Value::integer(1)),
    ///     ]))),
    /// ]);
    /// assert_eq!(Value::Object(nested).depth(), 2);
    /// ```
    pub fn depth(&self) -> usize {
        self.walk().map(|(path, _)| path.len()).max().unwrap_or(0)
    }

    /// Flatten nested structure into a single-level object
    ///
    /// Converts nested paths into dot-notation keys.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::Object;
    ///
    /// let inner = Object::from_iter(vec![
    ///     ("city".to_string(), Value::text("NYC")),
    /// ]);
    /// let outer = Object::from_iter(vec![
    ///     ("address".to_string(), Value::Object(inner)),
    ///     ("name".to_string(), Value::text("Alice")),
    /// ]);
    /// let root = Value::Object(outer);
    ///
    /// let flat = root.flatten();
    /// // flatten creates literal keys like "address.city"
    /// assert!(flat.get_key("name").is_some());
    /// assert!(flat.get_key("address.city").is_some());
    /// ```
    pub fn flatten(&self) -> Value {
        let mut result = Object::new();

        for (path, value) in self.walk_leaves() {
            if !path.is_root() {
                result = result.insert(path.to_string(), value.clone());
            }
        }

        Value::Object(result)
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_nested_value() -> Value {
        let inner = Object::from_iter(vec![
            ("city".to_string(), Value::text("NYC")),
            ("zip".to_string(), Value::text("10001")),
        ]);
        let users = Array::from_vec(vec![
            Value::Object(Object::from_iter(vec![
                ("name".to_string(), Value::text("Alice")),
                ("age".to_string(), Value::integer(30)),
            ])),
            Value::Object(Object::from_iter(vec![
                ("name".to_string(), Value::text("Bob")),
                ("age".to_string(), Value::integer(25)),
            ])),
        ]);
        let root = Object::from_iter(vec![
            ("address".to_string(), Value::Object(inner)),
            ("users".to_string(), Value::Array(users)),
            ("active".to_string(), Value::boolean(true)),
        ]);
        Value::Object(root)
    }

    // ==================== Walk Tests ====================

    #[test]
    fn test_walk_scalar() {
        let val = Value::integer(42);
        let items: Vec<_> = val.walk().collect();
        assert_eq!(items.len(), 1);
        assert!(items[0].0.is_root());
    }

    #[test]
    fn test_walk_flat_object() {
        let obj = Object::from_iter(vec![
            ("a".to_string(), Value::integer(1)),
            ("b".to_string(), Value::integer(2)),
        ]);
        let root = Value::Object(obj);

        let items: Vec<_> = root.walk().collect();
        assert_eq!(items.len(), 3); // root + 2 values
    }

    #[test]
    fn test_walk_nested() {
        let root = make_nested_value();
        let count = root.walk().count();
        // root(1) + address(1) + city(1) + zip(1) + users(1) + user0(1) + name(1) + age(1) + user1(1) + name(1) + age(1) + active(1) = 12
        assert!(count >= 10);
    }

    #[test]
    fn test_walk_children() {
        let obj = Object::from_iter(vec![
            ("a".to_string(), Value::integer(1)),
            ("b".to_string(), Value::integer(2)),
        ]);
        let root = Value::Object(obj);

        let items: Vec<_> = root.walk_children().collect();
        assert_eq!(items.len(), 2); // Only children, not root
    }

    #[test]
    fn test_walk_leaves() {
        let root = make_nested_value();
        let leaves: Vec<_> = root.walk_leaves().collect();

        // All leaves should be non-containers
        for (_, value) in &leaves {
            assert!(!value.is_collection());
        }
    }

    // ==================== Find Tests ====================

    #[test]
    fn test_find_first() {
        let root = make_nested_value();

        let result = root.find(|v| v.is_boolean());
        assert!(result.is_some());
        let (path, value) = result.unwrap();
        assert_eq!(path.to_string(), "active");
        assert_eq!(*value, Value::boolean(true));
    }

    #[test]
    fn test_find_not_found() {
        let root = Value::integer(42);
        let result = root.find(|v| v.is_text());
        assert!(result.is_none());
    }

    #[test]
    fn test_find_all() {
        let root = make_nested_value();

        let integers = root.find_all(|v| v.is_integer());
        assert_eq!(integers.len(), 2); // Two ages
    }

    #[test]
    fn test_find_by_kind() {
        let root = make_nested_value();

        let texts = root.find_by_kind(crate::ValueKind::String);
        assert!(texts.len() >= 4); // city, zip, 2 names
    }

    // ==================== Map Tests ====================

    #[test]
    fn test_map_values() {
        let obj = Object::from_iter(vec![
            ("a".to_string(), Value::integer(1)),
            ("b".to_string(), Value::integer(2)),
        ]);
        let root = Value::Object(obj);

        let doubled = root.map_values(|v| {
            if let Some(i) = v.as_integer() {
                Value::integer(i.value() * 2)
            } else {
                v.clone()
            }
        });

        assert_eq!(doubled.get_path("a").unwrap(), Value::integer(2));
        assert_eq!(doubled.get_path("b").unwrap(), Value::integer(4));
    }

    #[test]
    fn test_map_at() {
        let obj = Object::from_iter(vec![
            ("public".to_string(), Value::text("visible")),
            ("secret".to_string(), Value::text("hidden")),
        ]);
        let root = Value::Object(obj);

        let redacted = root.map_at(
            |path| path.to_string() == "secret",
            |_| Value::text("[REDACTED]"),
        );

        assert_eq!(redacted.get_path("public").unwrap(), Value::text("visible"));
        assert_eq!(
            redacted.get_path("secret").unwrap(),
            Value::text("[REDACTED]")
        );
    }

    // ==================== Filter Tests ====================

    #[test]
    fn test_filter() {
        let obj = Object::from_iter(vec![
            ("keep".to_string(), Value::integer(1)),
            ("_remove".to_string(), Value::integer(2)),
        ]);
        let root = Value::Object(obj);

        let filtered = root.filter(|path, _| !path.to_string().starts_with("_"));

        assert!(filtered.has_path("keep"));
        assert!(!filtered.has_path("_remove"));
    }

    #[test]
    fn test_filter_deep() {
        let inner = Object::from_iter(vec![
            ("keep".to_string(), Value::integer(1)),
            ("_remove".to_string(), Value::integer(2)),
        ]);
        let outer = Object::from_iter(vec![("nested".to_string(), Value::Object(inner))]);
        let root = Value::Object(outer);

        let filtered = root.filter_deep(|path, _| {
            path.last()
                .and_then(|s| s.as_key())
                .map(|k| !k.starts_with("_"))
                .unwrap_or(true)
        });

        assert!(filtered.has_path("nested.keep"));
        assert!(!filtered.has_path("nested._remove"));
    }

    // ==================== Utility Tests ====================

    #[test]
    fn test_count_values() {
        let arr = Array::from_vec(vec![Value::integer(1), Value::integer(2)]);
        let obj = Object::from_iter(vec![("items".to_string(), Value::Array(arr))]);
        let root = Value::Object(obj);

        assert_eq!(root.count_values(), 4); // root + array + 2 integers
    }

    #[test]
    fn test_count_leaves() {
        let arr = Array::from_vec(vec![Value::integer(1), Value::integer(2)]);
        let obj = Object::from_iter(vec![("items".to_string(), Value::Array(arr))]);
        let root = Value::Object(obj);

        assert_eq!(root.count_leaves(), 2); // Only the 2 integers
    }

    #[test]
    fn test_depth() {
        assert_eq!(Value::integer(42).depth(), 0);

        let flat = Object::from_iter(vec![("a".to_string(), Value::integer(1))]);
        assert_eq!(Value::Object(flat).depth(), 1);

        let nested = Object::from_iter(vec![(
            "inner".to_string(),
            Value::Object(Object::from_iter(vec![(
                "deep".to_string(),
                Value::integer(1),
            )])),
        )]);
        assert_eq!(Value::Object(nested).depth(), 2);
    }

    #[test]
    fn test_flatten() {
        let inner = Object::from_iter(vec![("city".to_string(), Value::text("NYC"))]);
        let outer = Object::from_iter(vec![
            ("address".to_string(), Value::Object(inner)),
            ("name".to_string(), Value::text("Alice")),
        ]);
        let root = Value::Object(outer);

        let flat = root.flatten();
        // flatten creates keys like "address.city" as literal key names (not paths)
        // so we use get_key to check for the literal key
        assert!(flat.get_key("name").is_some());
        assert!(flat.get_key("address.city").is_some());
    }
}
