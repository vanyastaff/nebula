//! Diff and Patch operations for Value structures
//!
//! This module provides functionality to compare two Values and generate
//! a list of changes (diff), as well as apply those changes to produce
//! a new Value (patch).
//!
//! # Use Cases
//!
//! - **Audit trails**: Track what changed between workflow executions
//! - **Synchronization**: Send only changes instead of full data
//! - **Undo/Redo**: Store diffs for reversible operations
//! - **Debugging**: Compare expected vs actual values
//!
//! # Examples
//!
//! ## Computing a diff
//!
//! ```
//! use nebula_value::Value;
//! use nebula_value::collections::Object;
//! use nebula_value::diff::ValueDiff;
//!
//! let old = Object::from_iter(vec![
//!     ("name".to_string(), Value::text("Alice")),
//!     ("age".to_string(), Value::integer(30)),
//! ]);
//!
//! let new = Object::from_iter(vec![
//!     ("name".to_string(), Value::text("Alice")),
//!     ("age".to_string(), Value::integer(31)),  // Changed
//!     ("email".to_string(), Value::text("alice@example.com")),  // Added
//! ]);
//!
//! let diffs = Value::Object(old).diff(&Value::Object(new));
//! assert_eq!(diffs.len(), 2); // age changed, email added
//! ```
//!
//! ## Applying a patch
//!
//! ```
//! use nebula_value::Value;
//! use nebula_value::collections::Object;
//! use nebula_value::diff::ValueDiff;
//!
//! let original = Object::from_iter(vec![
//!     ("count".to_string(), Value::integer(1)),
//! ]);
//! let root = Value::Object(original);
//!
//! let diffs = vec![
//!     ValueDiff::changed("count", Value::integer(1), Value::integer(2)),
//! ];
//!
//! let patched = root.apply_diff(&diffs).unwrap();
//! assert_eq!(patched.get_path("count").unwrap(), Value::integer(2));
//! ```

use std::collections::HashSet;
use std::fmt;

use crate::collections::{Array, Object};
use crate::core::ValueResult;
use crate::core::path::Path;
use crate::core::value::Value;

// ============================================================================
// DIFF TYPE
// ============================================================================

/// Represents a single change between two Values
#[derive(Debug, Clone, PartialEq)]
pub enum ValueDiff {
    /// A value was added at this path
    Added {
        /// Path where value was added
        path: Path,
        /// The new value
        value: Value,
    },

    /// A value was removed from this path
    Removed {
        /// Path where value was removed
        path: Path,
        /// The old value that was removed
        old_value: Value,
    },

    /// A value was changed at this path
    Changed {
        /// Path where value changed
        path: Path,
        /// The old value
        old_value: Value,
        /// The new value
        new_value: Value,
    },
}

impl ValueDiff {
    /// Create an "added" diff
    pub fn added(path: impl Into<String>, value: Value) -> Self {
        Self::Added {
            path: Path::parse(&path.into()).unwrap_or_default(),
            value,
        }
    }

    /// Create an "added" diff with Path
    pub fn added_at(path: Path, value: Value) -> Self {
        Self::Added { path, value }
    }

    /// Create a "removed" diff
    pub fn removed(path: impl Into<String>, old_value: Value) -> Self {
        Self::Removed {
            path: Path::parse(&path.into()).unwrap_or_default(),
            old_value,
        }
    }

    /// Create a "removed" diff with Path
    pub fn removed_at(path: Path, old_value: Value) -> Self {
        Self::Removed { path, old_value }
    }

    /// Create a "changed" diff
    pub fn changed(path: impl Into<String>, old_value: Value, new_value: Value) -> Self {
        Self::Changed {
            path: Path::parse(&path.into()).unwrap_or_default(),
            old_value,
            new_value,
        }
    }

    /// Create a "changed" diff with Path
    pub fn changed_at(path: Path, old_value: Value, new_value: Value) -> Self {
        Self::Changed {
            path,
            old_value,
            new_value,
        }
    }

    /// Get the path of this diff
    pub fn path(&self) -> &Path {
        match self {
            Self::Added { path, .. } => path,
            Self::Removed { path, .. } => path,
            Self::Changed { path, .. } => path,
        }
    }

    /// Check if this is an "added" diff
    pub fn is_added(&self) -> bool {
        matches!(self, Self::Added { .. })
    }

    /// Check if this is a "removed" diff
    pub fn is_removed(&self) -> bool {
        matches!(self, Self::Removed { .. })
    }

    /// Check if this is a "changed" diff
    pub fn is_changed(&self) -> bool {
        matches!(self, Self::Changed { .. })
    }

    /// Get the new value (for Added and Changed)
    pub fn new_value(&self) -> Option<&Value> {
        match self {
            Self::Added { value, .. } => Some(value),
            Self::Changed { new_value, .. } => Some(new_value),
            Self::Removed { .. } => None,
        }
    }

    /// Get the old value (for Removed and Changed)
    pub fn old_value(&self) -> Option<&Value> {
        match self {
            Self::Removed { old_value, .. } => Some(old_value),
            Self::Changed { old_value, .. } => Some(old_value),
            Self::Added { .. } => None,
        }
    }

    /// Invert this diff (for undo operations)
    pub fn invert(&self) -> Self {
        match self {
            Self::Added { path, value } => Self::Removed {
                path: path.clone(),
                old_value: value.clone(),
            },
            Self::Removed { path, old_value } => Self::Added {
                path: path.clone(),
                value: old_value.clone(),
            },
            Self::Changed {
                path,
                old_value,
                new_value,
            } => Self::Changed {
                path: path.clone(),
                old_value: new_value.clone(),
                new_value: old_value.clone(),
            },
        }
    }
}

impl fmt::Display for ValueDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Added { path, value } => {
                write!(f, "+ {}: {:?}", path, value.kind())
            }
            Self::Removed { path, old_value } => {
                write!(f, "- {}: {:?}", path, old_value.kind())
            }
            Self::Changed {
                path,
                old_value,
                new_value,
            } => {
                write!(
                    f,
                    "~ {}: {:?} -> {:?}",
                    path,
                    old_value.kind(),
                    new_value.kind()
                )
            }
        }
    }
}

// ============================================================================
// DIFF OPTIONS
// ============================================================================

/// Options for controlling diff behavior
#[derive(Debug, Clone)]
pub struct DiffOptions {
    /// Whether to include structural changes (object/array additions/removals)
    pub include_structural: bool,
    /// Maximum depth to traverse (None = unlimited)
    pub max_depth: Option<usize>,
    /// Paths to ignore during comparison
    pub ignore_paths: HashSet<String>,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            include_structural: true,
            max_depth: None,
            ignore_paths: HashSet::new(),
        }
    }
}

impl DiffOptions {
    /// Create new options with defaults
    pub fn new() -> Self {
        Self::default()
    }

    /// Set max depth
    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.max_depth = Some(depth);
        self
    }

    /// Add a path to ignore
    pub fn ignore(mut self, path: impl Into<String>) -> Self {
        self.ignore_paths.insert(path.into());
        self
    }

    /// Exclude structural changes
    pub fn values_only(mut self) -> Self {
        self.include_structural = false;
        self
    }
}

// ============================================================================
// VALUE DIFF IMPLEMENTATION
// ============================================================================

impl Value {
    /// Compute the diff between this value and another
    ///
    /// Returns a list of changes needed to transform `self` into `other`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::Object;
    ///
    /// let old = Object::from_iter(vec![
    ///     ("name".to_string(), Value::text("Alice")),
    /// ]);
    /// let new = Object::from_iter(vec![
    ///     ("name".to_string(), Value::text("Bob")),
    /// ]);
    ///
    /// let diffs = Value::Object(old).diff(&Value::Object(new));
    /// assert_eq!(diffs.len(), 1);
    /// assert!(diffs[0].is_changed());
    /// ```
    pub fn diff(&self, other: &Value) -> Vec<ValueDiff> {
        self.diff_with_options(other, &DiffOptions::default())
    }

    /// Compute diff with custom options
    pub fn diff_with_options(&self, other: &Value, options: &DiffOptions) -> Vec<ValueDiff> {
        let mut diffs = Vec::new();
        self.diff_recursive(other, &Path::new(), options, 0, &mut diffs);
        diffs
    }

    /// Internal recursive diff implementation
    fn diff_recursive(
        &self,
        other: &Value,
        current_path: &Path,
        options: &DiffOptions,
        depth: usize,
        diffs: &mut Vec<ValueDiff>,
    ) {
        // Check depth limit
        if let Some(max_depth) = options.max_depth {
            if depth > max_depth {
                return;
            }
        }

        // Check if path should be ignored
        if options.ignore_paths.contains(&current_path.to_string()) {
            return;
        }

        // Handle different type combinations
        match (self, other) {
            // Both objects - compare recursively
            (Value::Object(old_obj), Value::Object(new_obj)) => {
                self.diff_objects(old_obj, new_obj, current_path, options, depth, diffs);
            }

            // Both arrays - compare recursively
            (Value::Array(old_arr), Value::Array(new_arr)) => {
                self.diff_arrays(old_arr, new_arr, current_path, options, depth, diffs);
            }

            // Same type, compare values
            _ if self.kind() == other.kind() => {
                if self != other {
                    diffs.push(ValueDiff::changed_at(
                        current_path.clone(),
                        self.clone(),
                        other.clone(),
                    ));
                }
            }

            // Different types - always a change
            _ => {
                diffs.push(ValueDiff::changed_at(
                    current_path.clone(),
                    self.clone(),
                    other.clone(),
                ));
            }
        }
    }

    /// Diff two objects
    fn diff_objects(
        &self,
        old_obj: &Object,
        new_obj: &Object,
        current_path: &Path,
        options: &DiffOptions,
        depth: usize,
        diffs: &mut Vec<ValueDiff>,
    ) {
        // Find removed and changed keys
        for (key, old_value) in old_obj.entries() {
            let child_path = current_path.key(key.as_str());

            match new_obj.get(key) {
                Some(new_value) => {
                    // Key exists in both - recurse
                    old_value.diff_recursive(new_value, &child_path, options, depth + 1, diffs);
                }
                None => {
                    // Key removed
                    if options.include_structural {
                        diffs.push(ValueDiff::removed_at(child_path, old_value.clone()));
                    }
                }
            }
        }

        // Find added keys
        for (key, new_value) in new_obj.entries() {
            if !old_obj.contains_key(key) {
                let child_path = current_path.key(key.as_str());
                if options.include_structural {
                    diffs.push(ValueDiff::added_at(child_path, new_value.clone()));
                }
            }
        }
    }

    /// Diff two arrays
    fn diff_arrays(
        &self,
        old_arr: &Array,
        new_arr: &Array,
        current_path: &Path,
        options: &DiffOptions,
        depth: usize,
        diffs: &mut Vec<ValueDiff>,
    ) {
        let old_len = old_arr.len();
        let new_len = new_arr.len();
        let min_len = old_len.min(new_len);

        // Compare elements that exist in both
        for i in 0..min_len {
            let child_path = current_path.index(i);
            if let (Some(old_val), Some(new_val)) = (old_arr.get(i), new_arr.get(i)) {
                old_val.diff_recursive(new_val, &child_path, options, depth + 1, diffs);
            }
        }

        // Handle removed elements (old is longer)
        if old_len > new_len && options.include_structural {
            for i in min_len..old_len {
                let child_path = current_path.index(i);
                if let Some(old_val) = old_arr.get(i) {
                    diffs.push(ValueDiff::removed_at(child_path, old_val.clone()));
                }
            }
        }

        // Handle added elements (new is longer)
        if new_len > old_len && options.include_structural {
            for i in min_len..new_len {
                let child_path = current_path.index(i);
                if let Some(new_val) = new_arr.get(i) {
                    diffs.push(ValueDiff::added_at(child_path, new_val.clone()));
                }
            }
        }
    }

    /// Apply a list of diffs to this value
    ///
    /// Returns a new Value with all diffs applied.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::Object;
    /// use nebula_value::diff::ValueDiff;
    ///
    /// let original = Object::from_iter(vec![
    ///     ("count".to_string(), Value::integer(1)),
    /// ]);
    /// let root = Value::Object(original);
    ///
    /// let diffs = vec![
    ///     ValueDiff::changed("count", Value::integer(1), Value::integer(2)),
    /// ];
    ///
    /// let patched = root.apply_diff(&diffs).unwrap();
    /// assert_eq!(patched.get_path("count").unwrap(), Value::integer(2));
    /// ```
    ///
    /// # Errors
    ///
    /// Returns error if a diff cannot be applied (e.g., path doesn't exist for removal).
    pub fn apply_diff(&self, diffs: &[ValueDiff]) -> ValueResult<Value> {
        let mut result = self.clone();

        for diff in diffs {
            result = result.apply_single_diff(diff)?;
        }

        Ok(result)
    }

    /// Apply a single diff
    fn apply_single_diff(&self, diff: &ValueDiff) -> ValueResult<Value> {
        match diff {
            ValueDiff::Added { path, value } => self.set_by_path(path, value.clone()),
            ValueDiff::Removed { path, .. } => {
                if path.is_root() {
                    Ok(Value::Null)
                } else {
                    self.remove_by_path(path).map(|(v, _)| v)
                }
            }
            ValueDiff::Changed {
                path, new_value, ..
            } => self.set_by_path(path, new_value.clone()),
        }
    }

    /// Check if two values are equal (deep comparison)
    ///
    /// This is equivalent to checking if `diff()` returns an empty list,
    /// but more efficient.
    pub fn deep_eq(&self, other: &Value) -> bool {
        self == other
    }

    /// Get a summary of differences
    ///
    /// Returns counts of added, removed, and changed values.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::Object;
    ///
    /// let old = Object::from_iter(vec![
    ///     ("a".to_string(), Value::integer(1)),
    ///     ("b".to_string(), Value::integer(2)),
    /// ]);
    /// let new = Object::from_iter(vec![
    ///     ("a".to_string(), Value::integer(10)),
    ///     ("c".to_string(), Value::integer(3)),
    /// ]);
    ///
    /// let (added, removed, changed) = Value::Object(old).diff_summary(&Value::Object(new));
    /// assert_eq!(added, 1);    // "c" added
    /// assert_eq!(removed, 1);  // "b" removed
    /// assert_eq!(changed, 1);  // "a" changed
    /// ```
    pub fn diff_summary(&self, other: &Value) -> (usize, usize, usize) {
        let diffs = self.diff(other);
        let added = diffs.iter().filter(|d| d.is_added()).count();
        let removed = diffs.iter().filter(|d| d.is_removed()).count();
        let changed = diffs.iter().filter(|d| d.is_changed()).count();
        (added, removed, changed)
    }

    /// Create a patch that can transform `old` into `new`
    ///
    /// Alias for `old.diff(new)`.
    pub fn create_patch(old: &Value, new: &Value) -> Vec<ValueDiff> {
        old.diff(new)
    }

    /// Invert a list of diffs (for undo)
    ///
    /// Returns diffs that would reverse the original changes.
    pub fn invert_diffs(diffs: &[ValueDiff]) -> Vec<ValueDiff> {
        diffs.iter().rev().map(|d| d.invert()).collect()
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== Basic Diff Tests ====================

    #[test]
    fn test_diff_equal_values() {
        let val = Value::integer(42);
        let diffs = val.diff(&Value::integer(42));
        assert!(diffs.is_empty());
    }

    #[test]
    fn test_diff_different_scalars() {
        let old = Value::integer(1);
        let new = Value::integer(2);
        let diffs = old.diff(&new);

        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].is_changed());
    }

    #[test]
    fn test_diff_different_types() {
        let old = Value::integer(42);
        let new = Value::text("42");
        let diffs = old.diff(&new);

        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].is_changed());
    }

    #[test]
    fn test_diff_object_added_key() {
        let old = Object::from_iter(vec![("a".to_string(), Value::integer(1))]);
        let new = Object::from_iter(vec![
            ("a".to_string(), Value::integer(1)),
            ("b".to_string(), Value::integer(2)),
        ]);

        let diffs = Value::Object(old).diff(&Value::Object(new));

        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].is_added());
        assert_eq!(diffs[0].path().to_string(), "b");
    }

    #[test]
    fn test_diff_object_removed_key() {
        let old = Object::from_iter(vec![
            ("a".to_string(), Value::integer(1)),
            ("b".to_string(), Value::integer(2)),
        ]);
        let new = Object::from_iter(vec![("a".to_string(), Value::integer(1))]);

        let diffs = Value::Object(old).diff(&Value::Object(new));

        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].is_removed());
        assert_eq!(diffs[0].path().to_string(), "b");
    }

    #[test]
    fn test_diff_object_changed_value() {
        let old = Object::from_iter(vec![("count".to_string(), Value::integer(1))]);
        let new = Object::from_iter(vec![("count".to_string(), Value::integer(2))]);

        let diffs = Value::Object(old).diff(&Value::Object(new));

        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].is_changed());
        assert_eq!(diffs[0].path().to_string(), "count");
    }

    #[test]
    fn test_diff_nested_object() {
        let old_inner = Object::from_iter(vec![("x".to_string(), Value::integer(1))]);
        let new_inner = Object::from_iter(vec![("x".to_string(), Value::integer(2))]);

        let old = Object::from_iter(vec![("nested".to_string(), Value::Object(old_inner))]);
        let new = Object::from_iter(vec![("nested".to_string(), Value::Object(new_inner))]);

        let diffs = Value::Object(old).diff(&Value::Object(new));

        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path().to_string(), "nested.x");
    }

    #[test]
    fn test_diff_array_changed_element() {
        let old = Array::from_vec(vec![Value::integer(1), Value::integer(2)]);
        let new = Array::from_vec(vec![Value::integer(1), Value::integer(99)]);

        let diffs = Value::Array(old).diff(&Value::Array(new));

        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].is_changed());
        assert_eq!(diffs[0].path().to_string(), "[1]");
    }

    #[test]
    fn test_diff_array_added_element() {
        let old = Array::from_vec(vec![Value::integer(1)]);
        let new = Array::from_vec(vec![Value::integer(1), Value::integer(2)]);

        let diffs = Value::Array(old).diff(&Value::Array(new));

        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].is_added());
        assert_eq!(diffs[0].path().to_string(), "[1]");
    }

    #[test]
    fn test_diff_array_removed_element() {
        let old = Array::from_vec(vec![Value::integer(1), Value::integer(2)]);
        let new = Array::from_vec(vec![Value::integer(1)]);

        let diffs = Value::Array(old).diff(&Value::Array(new));

        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].is_removed());
        assert_eq!(diffs[0].path().to_string(), "[1]");
    }

    // ==================== Apply Diff Tests ====================

    #[test]
    fn test_apply_diff_add() {
        let root = Value::Object(Object::new());
        let diffs = vec![ValueDiff::added("name", Value::text("Alice"))];

        let patched = root.apply_diff(&diffs).unwrap();
        assert_eq!(patched.get_path("name").unwrap(), Value::text("Alice"));
    }

    #[test]
    fn test_apply_diff_remove() {
        let obj = Object::from_iter(vec![
            ("name".to_string(), Value::text("Alice")),
            ("age".to_string(), Value::integer(30)),
        ]);
        let root = Value::Object(obj);

        let diffs = vec![ValueDiff::removed("age", Value::integer(30))];

        let patched = root.apply_diff(&diffs).unwrap();
        assert!(patched.has_path("name"));
        assert!(!patched.has_path("age"));
    }

    #[test]
    fn test_apply_diff_change() {
        let obj = Object::from_iter(vec![("count".to_string(), Value::integer(1))]);
        let root = Value::Object(obj);

        let diffs = vec![ValueDiff::changed(
            "count",
            Value::integer(1),
            Value::integer(2),
        )];

        let patched = root.apply_diff(&diffs).unwrap();
        assert_eq!(patched.get_path("count").unwrap(), Value::integer(2));
    }

    #[test]
    fn test_apply_multiple_diffs() {
        let obj = Object::from_iter(vec![
            ("a".to_string(), Value::integer(1)),
            ("b".to_string(), Value::integer(2)),
        ]);
        let root = Value::Object(obj);

        let diffs = vec![
            ValueDiff::changed("a", Value::integer(1), Value::integer(10)),
            ValueDiff::removed("b", Value::integer(2)),
            ValueDiff::added("c", Value::integer(3)),
        ];

        let patched = root.apply_diff(&diffs).unwrap();
        assert_eq!(patched.get_path("a").unwrap(), Value::integer(10));
        assert!(!patched.has_path("b"));
        assert_eq!(patched.get_path("c").unwrap(), Value::integer(3));
    }

    // ==================== Roundtrip Tests ====================

    #[test]
    fn test_diff_apply_roundtrip() {
        let old = Object::from_iter(vec![
            ("name".to_string(), Value::text("Alice")),
            ("age".to_string(), Value::integer(30)),
        ]);
        let new = Object::from_iter(vec![
            ("name".to_string(), Value::text("Bob")),
            ("email".to_string(), Value::text("bob@example.com")),
        ]);

        let old_val = Value::Object(old);
        let new_val = Value::Object(new.clone());

        let diffs = old_val.diff(&new_val);
        let patched = old_val.apply_diff(&diffs).unwrap();

        assert_eq!(patched, Value::Object(new));
    }

    // ==================== Invert Tests ====================

    #[test]
    fn test_invert_added() {
        let diff = ValueDiff::added("path", Value::integer(42));
        let inverted = diff.invert();

        assert!(inverted.is_removed());
        assert_eq!(inverted.path().to_string(), "path");
    }

    #[test]
    fn test_invert_removed() {
        let diff = ValueDiff::removed("path", Value::integer(42));
        let inverted = diff.invert();

        assert!(inverted.is_added());
    }

    #[test]
    fn test_invert_changed() {
        let diff = ValueDiff::changed("path", Value::integer(1), Value::integer(2));
        let inverted = diff.invert();

        assert!(inverted.is_changed());
        assert_eq!(inverted.old_value(), Some(&Value::integer(2)));
        assert_eq!(inverted.new_value(), Some(&Value::integer(1)));
    }

    #[test]
    fn test_invert_diffs_undo() {
        let obj = Object::from_iter(vec![("count".to_string(), Value::integer(1))]);
        let original = Value::Object(obj);

        // Make changes
        let diffs = vec![ValueDiff::changed(
            "count",
            Value::integer(1),
            Value::integer(2),
        )];
        let modified = original.apply_diff(&diffs).unwrap();

        // Undo changes
        let undo_diffs = Value::invert_diffs(&diffs);
        let restored = modified.apply_diff(&undo_diffs).unwrap();

        assert_eq!(original, restored);
    }

    // ==================== Options Tests ====================

    #[test]
    fn test_diff_with_max_depth() {
        let inner = Object::from_iter(vec![("deep".to_string(), Value::integer(1))]);
        let old_outer =
            Object::from_iter(vec![("nested".to_string(), Value::Object(inner.clone()))]);

        let new_inner = Object::from_iter(vec![("deep".to_string(), Value::integer(2))]);
        let new_outer = Object::from_iter(vec![("nested".to_string(), Value::Object(new_inner))]);

        let old = Value::Object(old_outer);
        let new = Value::Object(new_outer);

        // With depth 1, we shouldn't see the deep change
        let options = DiffOptions::new().with_max_depth(1);
        let diffs = old.diff_with_options(&new, &options);

        // The nested object itself is compared as a whole
        assert!(diffs.is_empty() || diffs.iter().all(|d| d.path().len() <= 1));
    }

    #[test]
    fn test_diff_ignore_paths() {
        let old = Object::from_iter(vec![
            ("public".to_string(), Value::integer(1)),
            ("secret".to_string(), Value::integer(100)),
        ]);
        let new = Object::from_iter(vec![
            ("public".to_string(), Value::integer(2)),
            ("secret".to_string(), Value::integer(200)),
        ]);

        let options = DiffOptions::new().ignore("secret");
        let diffs = Value::Object(old).diff_with_options(&Value::Object(new), &options);

        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path().to_string(), "public");
    }

    // ==================== Summary Tests ====================

    #[test]
    fn test_diff_summary() {
        let old = Object::from_iter(vec![
            ("a".to_string(), Value::integer(1)),
            ("b".to_string(), Value::integer(2)),
        ]);
        let new = Object::from_iter(vec![
            ("a".to_string(), Value::integer(10)),
            ("c".to_string(), Value::integer(3)),
        ]);

        let (added, removed, changed) = Value::Object(old).diff_summary(&Value::Object(new));

        assert_eq!(added, 1); // "c" added
        assert_eq!(removed, 1); // "b" removed
        assert_eq!(changed, 1); // "a" changed
    }
}
