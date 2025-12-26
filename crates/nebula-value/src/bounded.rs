//! Bounded types with compile-time limits using const generics.
//!
//! This module provides wrapper types that enforce size limits at compile time
//! where possible, and at runtime where necessary.
//!
//! # Examples
//!
//! ```
//! use nebula_value::bounded::BoundedText;
//!
//! // Text limited to 100 bytes
//! type Username = BoundedText<100>;
//!
//! let username = Username::new("alice".to_string())?;
//! assert_eq!(username.as_str(), "alice");
//!
//! // This would fail at runtime
//! let long_name = "a".repeat(200);
//! assert!(Username::new(long_name).is_err());
//! # Ok::<(), nebula_value::ValueError>(())
//! ```

use crate::collections::{Array, Object};
use crate::core::Value;
use crate::core::{ValueError, ValueResult};
use crate::scalar::Text;

/// Text with a compile-time maximum byte length.
///
/// This provides stronger type safety than using `Text` directly,
/// as the maximum length is encoded in the type.
///
/// # Type Parameters
///
/// * `MAX` - Maximum number of bytes allowed
///
/// # Examples
///
/// ```
/// use nebula_value::bounded::BoundedText;
///
/// // Email limited to 255 bytes
/// type Email = BoundedText<255>;
///
/// let email = Email::new("user@example.com".to_string())?;
/// assert_eq!(email.len(), 16);
/// # Ok::<(), nebula_value::ValueError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BoundedText<const MAX: usize> {
    inner: Text,
}

impl<const MAX: usize> BoundedText<MAX> {
    /// Create a new bounded text with validation.
    ///
    /// # Errors
    ///
    /// Returns [`ValueError::LimitExceeded`] if the text exceeds `MAX` bytes.
    pub fn new(s: String) -> ValueResult<Self> {
        if s.len() > MAX {
            return Err(ValueError::limit_exceeded(
                format!("BoundedText<{}>", MAX),
                MAX,
                s.len(),
            ));
        }
        Ok(Self {
            inner: Text::new(s),
        })
    }

    /// Create from &str with validation.
    pub fn parse(s: &str) -> ValueResult<Self> {
        Self::new(s.to_string())
    }

    /// Get the inner Text value.
    #[inline]
    pub fn into_inner(self) -> Text {
        self.inner
    }

    /// Get as &str.
    #[inline]
    pub fn as_str(&self) -> &str {
        self.inner.as_str()
    }

    /// Get the byte length.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get the maximum allowed length.
    #[inline]
    pub const fn max_len() -> usize {
        MAX
    }
}

impl<const MAX: usize> std::fmt::Display for BoundedText<MAX> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl<const MAX: usize> AsRef<str> for BoundedText<MAX> {
    fn as_ref(&self) -> &str {
        self.inner.as_ref()
    }
}

impl<const MAX: usize> TryFrom<String> for BoundedText<MAX> {
    type Error = ValueError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl<const MAX: usize> TryFrom<&str> for BoundedText<MAX> {
    type Error = ValueError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::parse(s)
    }
}

/// Array with a compile-time maximum length.
///
/// # Type Parameters
///
/// * `MAX` - Maximum number of elements allowed
///
/// # Examples
///
/// ```
/// use nebula_value::bounded::BoundedArray;
/// use nebula_value::Value;
///
/// // Array limited to 10 elements
/// type SmallArray = BoundedArray<10>;
///
/// let mut arr = SmallArray::new();
/// arr = arr.push(Value::integer(1))?;
/// arr = arr.push(Value::integer(2))?;
/// assert_eq!(arr.len(), 2);
/// # Ok::<(), nebula_value::ValueError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedArray<const MAX: usize> {
    inner: Array,
}

impl<const MAX: usize> BoundedArray<MAX> {
    /// Create a new empty bounded array.
    pub fn new() -> Self {
        Self {
            inner: Array::new(),
        }
    }

    /// Create from Vec with validation.
    ///
    /// # Errors
    ///
    /// Returns [`ValueError::LimitExceeded`] if the vec exceeds `MAX` elements.
    pub fn from_vec(vec: Vec<Value>) -> ValueResult<Self> {
        if vec.len() > MAX {
            return Err(ValueError::limit_exceeded(
                format!("BoundedArray<{}>", MAX),
                MAX,
                vec.len(),
            ));
        }
        Ok(Self {
            inner: Array::from_vec(vec),
        })
    }

    /// Push an element with limit check.
    ///
    /// # Errors
    ///
    /// Returns [`ValueError::LimitExceeded`] if array is already at max capacity.
    pub fn push(&self, value: Value) -> ValueResult<Self> {
        if self.len() >= MAX {
            return Err(ValueError::limit_exceeded(
                format!("BoundedArray<{}>", MAX),
                MAX,
                self.len() + 1,
            ));
        }
        Ok(Self {
            inner: self.inner.push(value),
        })
    }

    /// Get the inner Array.
    #[inline]
    pub fn into_inner(self) -> Array {
        self.inner
    }

    /// Get the length.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get the maximum allowed length.
    #[inline]
    pub const fn max_len() -> usize {
        MAX
    }

    /// Get element at index.
    #[inline]
    pub fn get(&self, index: usize) -> Option<&Value> {
        self.inner.get(index)
    }
}

impl<const MAX: usize> Default for BoundedArray<MAX> {
    fn default() -> Self {
        Self::new()
    }
}

/// Object with a compile-time maximum number of keys.
///
/// # Type Parameters
///
/// * `MAX` - Maximum number of keys allowed
///
/// # Examples
///
/// ```
/// use nebula_value::bounded::BoundedObject;
/// use nebula_value::Value;
///
/// // Object limited to 5 keys
/// type SmallConfig = BoundedObject<5>;
///
/// let mut obj = SmallConfig::new();
/// obj = obj.insert("host", Value::text("localhost"))?;
/// obj = obj.insert("port", Value::integer(8080))?;
/// assert_eq!(obj.len(), 2);
/// # Ok::<(), nebula_value::ValueError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedObject<const MAX: usize> {
    inner: Object,
}

impl<const MAX: usize> BoundedObject<MAX> {
    /// Create a new empty bounded object.
    pub fn new() -> Self {
        Self {
            inner: Object::new(),
        }
    }

    /// Insert a key-value pair with limit check.
    ///
    /// # Errors
    ///
    /// Returns [`ValueError::LimitExceeded`] if object is already at max capacity
    /// and the key doesn't exist yet.
    pub fn insert(&self, key: impl Into<String>, value: Value) -> ValueResult<Self> {
        let key = key.into();
        let is_new_key = !self.inner.contains_key(&key);

        if is_new_key && self.len() >= MAX {
            return Err(ValueError::limit_exceeded(
                format!("BoundedObject<{}>", MAX),
                MAX,
                self.len() + 1,
            ));
        }

        Ok(Self {
            inner: self.inner.insert(key, value),
        })
    }

    /// Get the inner Object.
    #[inline]
    pub fn into_inner(self) -> Object {
        self.inner
    }

    /// Get the number of keys.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get the maximum allowed number of keys.
    #[inline]
    pub const fn max_len() -> usize {
        MAX
    }

    /// Get value by key.
    #[inline]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.inner.get(key)
    }
}

impl<const MAX: usize> Default for BoundedObject<MAX> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bounded_text() {
        type Username = BoundedText<20>;

        // Valid username
        let user = Username::new("alice".to_string()).unwrap();
        assert_eq!(user.as_str(), "alice");
        assert_eq!(user.len(), 5);
        assert_eq!(Username::max_len(), 20);

        // Too long
        let long = "a".repeat(30);
        assert!(Username::new(long).is_err());

        // Empty is valid
        let empty = Username::new(String::new()).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_bounded_array() {
        type SmallArray = BoundedArray<3>;

        let mut arr = SmallArray::new();
        assert_eq!(arr.len(), 0);
        assert_eq!(SmallArray::max_len(), 3);

        // Push within limit
        arr = arr.push(Value::integer(1)).unwrap();
        arr = arr.push(Value::integer(2)).unwrap();
        arr = arr.push(Value::integer(3)).unwrap();
        assert_eq!(arr.len(), 3);

        // Exceeds limit
        assert!(arr.push(Value::integer(4)).is_err());
    }

    #[test]
    fn test_bounded_object() {
        type SmallConfig = BoundedObject<2>;

        let mut obj = SmallConfig::new();
        assert_eq!(obj.len(), 0);
        assert_eq!(SmallConfig::max_len(), 2);

        // Insert within limit
        obj = obj.insert("a", Value::integer(1)).unwrap();
        obj = obj.insert("b", Value::integer(2)).unwrap();
        assert_eq!(obj.len(), 2);

        // Exceeds limit
        assert!(obj.insert("c", Value::integer(3)).is_err());

        // Updating existing key is OK
        obj = obj.insert("a", Value::integer(10)).unwrap();
        assert_eq!(obj.get("a"), Some(&Value::integer(10)));
    }

    #[test]
    fn test_try_from() {
        type Email = BoundedText<100>;

        let email: Email = "test@example.com".try_into().unwrap();
        assert_eq!(email.as_str(), "test@example.com");

        let long = "a".repeat(150);
        assert!(Email::try_from(long.as_str()).is_err());
    }
}
