//! Text (UTF-8 string) type for nebula-value
//!
//! This module provides a Text type that:
//! - Guarantees UTF-8 validity
//! - Efficient cloning via Arc<str>
//! - Length limits for DoS protection
//! - Zero-copy conversions where possible

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::Arc;

use crate::core::error::{ValueErrorExt, ValueResult};
use crate::core::limits::ValueLimits;
use crate::core::NebulaError;

/// UTF-8 text string with efficient cloning
///
/// Uses Arc<str> internally for cheap cloning of large strings.
/// Small strings (d23 bytes) could use SmallString optimization in the future.
#[derive(Debug, Clone)]
pub struct Text {
    inner: Arc<str>,
}

impl Text {
    /// Create a new Text from a String (takes ownership)
    pub fn new(s: String) -> Self {
        Self {
            inner: Arc::from(s.into_boxed_str()),
        }
    }

    /// Create a new Text from &str (allocates)
    pub fn from_str(s: &str) -> Self {
        Self {
            inner: Arc::from(s),
        }
    }

    /// Create a new Text with length validation
    pub fn with_limits(s: String, limits: &ValueLimits) -> ValueResult<Self> {
        limits.check_string_bytes(s.len())?;
        Ok(Self::new(s))
    }

    /// Create from &str with length validation
    pub fn from_str_with_limits(s: &str, limits: &ValueLimits) -> ValueResult<Self> {
        limits.check_string_bytes(s.len())?;
        Ok(Self::from_str(s))
    }

    /// Get the string as &str
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Get the byte length
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get the character count (O(n) operation)
    pub fn char_count(&self) -> usize {
        self.inner.chars().count()
    }

    /// Check if this text contains the given pattern
    pub fn contains(&self, pattern: &str) -> bool {
        self.inner.contains(pattern)
    }

    /// Check if this text starts with the given pattern
    pub fn starts_with(&self, pattern: &str) -> bool {
        self.inner.starts_with(pattern)
    }

    /// Check if this text ends with the given pattern
    pub fn ends_with(&self, pattern: &str) -> bool {
        self.inner.ends_with(pattern)
    }

    /// Convert to lowercase
    pub fn to_lowercase(&self) -> Text {
        Text::new(self.inner.to_lowercase())
    }

    /// Convert to uppercase
    pub fn to_uppercase(&self) -> Text {
        Text::new(self.inner.to_uppercase())
    }

    /// Trim whitespace from both ends
    pub fn trim(&self) -> Text {
        Text::from_str(self.inner.trim())
    }

    /// Split by delimiter
    pub fn split(&self, delimiter: &str) -> Vec<Text> {
        self.inner
            .split(delimiter)
            .map(|s| Text::from_str(s))
            .collect()
    }

    /// Replace all occurrences of a pattern
    pub fn replace(&self, from: &str, to: &str) -> Text {
        Text::new(self.inner.replace(from, to))
    }

    /// Get a substring by byte range
    pub fn substring(&self, start: usize, end: usize) -> ValueResult<Text> {
        if start > end || end > self.len() {
            return Err(NebulaError::value_out_of_range(
                format!("{}..{}", start, end),
                "0",
                self.len().to_string(),
            ));
        }

        // Ensure we're on character boundaries
        if !self.inner.is_char_boundary(start) || !self.inner.is_char_boundary(end) {
            return Err(NebulaError::validation(
                "substring indices must be on character boundaries",
            ));
        }

        Ok(Text::from_str(&self.inner[start..end]))
    }

    /// Concatenate with another text
    pub fn concat(&self, other: &Text) -> Text {
        let mut result = String::with_capacity(self.len() + other.len());
        result.push_str(&self.inner);
        result.push_str(&other.inner);
        Text::new(result)
    }

    /// Get underlying Arc for zero-copy cloning
    pub fn into_arc(self) -> Arc<str> {
        self.inner
    }
}

// Deref to &str for convenience
impl Deref for Text {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl PartialEq for Text {
    fn eq(&self, other: &Self) -> bool {
        self.inner.as_ref() == other.inner.as_ref()
    }
}

impl Eq for Text {}

impl PartialOrd for Text {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Text {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.inner.as_ref().cmp(other.inner.as_ref())
    }
}

impl Hash for Text {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.as_ref().hash(state);
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

// Conversions
impl From<String> for Text {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Self::from_str(s)
    }
}

impl From<Arc<str>> for Text {
    fn from(arc: Arc<str>) -> Self {
        Self { inner: arc }
    }
}

impl From<Text> for String {
    fn from(text: Text) -> Self {
        text.inner.to_string()
    }
}

impl AsRef<str> for Text {
    fn as_ref(&self) -> &str {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_creation() {
        let text = Text::new("hello".to_string());
        assert_eq!(text.as_str(), "hello");
        assert_eq!(text.len(), 5);
        assert!(!text.is_empty());
    }

    #[test]
    fn test_text_from_str() {
        let text = Text::from_str("world");
        assert_eq!(text.as_str(), "world");
    }

    #[test]
    fn test_text_with_limits() {
        let limits = ValueLimits::strict();

        // Should succeed
        let text = Text::with_limits("hello".to_string(), &limits);
        assert!(text.is_ok());

        // Should fail - too long for strict limits
        let long_string = "a".repeat(2_000_000);
        let text = Text::with_limits(long_string, &limits);
        assert!(text.is_err());
    }

    #[test]
    fn test_text_operations() {
        let text = Text::from_str("  Hello World  ");

        assert_eq!(text.to_lowercase().as_str(), "  hello world  ");
        assert_eq!(text.to_uppercase().as_str(), "  HELLO WORLD  ");
        assert_eq!(text.trim().as_str(), "Hello World");

        assert!(text.contains("World"));
        assert!(text.starts_with("  "));
        assert!(text.ends_with("  "));
    }

    #[test]
    fn test_text_split() {
        let text = Text::from_str("a,b,c");
        let parts = text.split(",");

        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].as_str(), "a");
        assert_eq!(parts[1].as_str(), "b");
        assert_eq!(parts[2].as_str(), "c");
    }

    #[test]
    fn test_text_replace() {
        let text = Text::from_str("hello world");
        let replaced = text.replace("world", "Rust");
        assert_eq!(replaced.as_str(), "hello Rust");
    }

    #[test]
    fn test_text_substring() {
        let text = Text::from_str("hello");

        let sub = text.substring(0, 5).unwrap();
        assert_eq!(sub.as_str(), "hello");

        let sub = text.substring(1, 4).unwrap();
        assert_eq!(sub.as_str(), "ell");

        // Out of bounds
        assert!(text.substring(0, 10).is_err());
        assert!(text.substring(5, 3).is_err());
    }

    #[test]
    fn test_text_concat() {
        let text1 = Text::from_str("hello ");
        let text2 = Text::from_str("world");
        let result = text1.concat(&text2);

        assert_eq!(result.as_str(), "hello world");
    }

    #[test]
    fn test_text_equality() {
        let text1 = Text::from_str("hello");
        let text2 = Text::from_str("hello");
        let text3 = Text::from_str("world");

        assert_eq!(text1, text2);
        assert_ne!(text1, text3);
    }

    #[test]
    fn test_text_ordering() {
        let text1 = Text::from_str("apple");
        let text2 = Text::from_str("banana");
        let text3 = Text::from_str("cherry");

        assert!(text1 < text2);
        assert!(text2 < text3);
        assert!(text1 < text3);
    }

    #[test]
    fn test_text_hash() {
        use std::collections::HashMap;

        let mut map = HashMap::new();
        map.insert(Text::from_str("key1"), 42);
        map.insert(Text::from_str("key2"), 100);

        assert_eq!(map.get(&Text::from_str("key1")), Some(&42));
        assert_eq!(map.get(&Text::from_str("key2")), Some(&100));
        assert_eq!(map.get(&Text::from_str("key3")), None);
    }


    #[test]
    fn test_text_clone_efficiency() {
        let text1 = Text::from_str("hello");
        let text2 = text1.clone();

        // Both should point to the same Arc
        assert_eq!(Arc::strong_count(&text1.inner), Arc::strong_count(&text2.inner));
        assert_eq!(text1, text2);
    }
}