use std::cmp::PartialEq;
use std::fmt;
use std::str::FromStr;

use derive_more::{AsRef, Deref};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// A normalized string identifier that follows specific formatting rules.
/// Keys are normalized to lowercase with underscores replacing whitespace.
/// They can only contain ASCII lowercase letters and underscores, with a
/// maximum length of 64 characters.
#[derive(Clone, Hash, Deref, AsRef)]
#[deref(forward)]
pub struct Key(String);

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum KeyParseError {
    #[error("Key cannot be empty or whitespace")]
    Empty,

    #[error("Key contains invalid characters")]
    InvalidCharacters,

    #[error("Key is too long (max 64 characters)")]
    TooLong,
}

impl Key {
    // This is a public constructor function for the Key struct.
    // It takes any type S that can be referenced as a string slice (&str).
    // It returns a Result, containing either a valid Key or a KeyParseError.
    pub fn new<S: AsRef<str>>(s: S) -> Result<Self, KeyParseError> {
        // 1. Get a string slice from the input and remove leading/trailing whitespace.
        let s = s.as_ref().trim();

        // 2. Check if the string is empty after trimming.
        if s.is_empty() {
            // If empty, return the specific Empty parse error.
            return Err(KeyParseError::Empty);
        }

        // 3. Process the string:
        //    - Split by whitespace (e.g., "my key" -> ["my", "key"])
        //    - Collect into a vector (Vec<&str>)
        //    - Join the parts with underscores (e.g., ["my", "key"] -> "my_key")
        //    - Convert the entire result to lowercase ASCII (e.g., "My_Key" ->
        //      "my_key")
        let joined = s
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("_")
            .to_ascii_lowercase();

        // 4. Check if the resulting string contains only allowed characters. Allowed
        //    characters are lowercase ASCII letters (a-z) and underscores (_).
        if !joined.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            // If invalid characters are found, return the specific InvalidCharacters parse
            // error.
            return Err(KeyParseError::InvalidCharacters);
        }

        // 5. Check the length of the resulting string.
        if joined.len() > 64 {
            // If too long, return the specific TooLong parse error.
            return Err(KeyParseError::TooLong);
        }

        // 6. If all checks passed, create a new Key instance with the processed string.
        //    Wrap the Key instance in Ok to indicate success.
        Ok(Key(joined))
    }

    /// Returns the inner string value of the Key
    pub fn into_inner(self) -> String {
        self.0
    }

    /// Returns a string slice of the inner value
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Validates whether a string would create a valid Key without actually
    /// creating it
    pub fn is_valid<S: AsRef<str>>(s: S) -> bool {
        Key::new(s).is_ok()
    }
}

impl FromStr for Key {
    type Err = KeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Key::new(s)
    }
}

impl TryFrom<&str> for Key {
    type Error = KeyParseError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Key::new(s)
    }
}

impl TryFrom<String> for Key {
    type Error = KeyParseError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Key::new(&s)
    }
}

impl TryFrom<&String> for Key {
    type Error = KeyParseError;
    fn try_from(s: &String) -> Result<Self, Self::Error> {
        Key::new(s)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key({})", self.0)
    }
}

// Generic implementation for comparing Key with any type that can be referenced
// as a string
impl<T> PartialEq<T> for Key
where
    T: AsRef<str>,
{
    fn eq(&self, other: &T) -> bool {
        self.as_ref() == other.as_ref()
    }
}

// Enable right-side comparisons by implementing PartialEq for common string
// types Note: These implementations need to be separate from Key's core
// PartialEq trait implementation
impl PartialEq<Key> for str {
    fn eq(&self, other: &Key) -> bool {
        self == other.as_ref()
    }
}

impl PartialEq<Key> for String {
    fn eq(&self, other: &Key) -> bool {
        self.as_str() == other.as_ref()
    }
}

impl PartialEq<Key> for &str {
    fn eq(&self, other: &Key) -> bool {
        *self == other.as_ref()
    }
}

// Implement PartialEq for Key to compare with another Key
impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

// Add specific implementation for &&str to fix the comparison issue
impl PartialEq<Key> for &&str {
    fn eq(&self, other: &Key) -> bool {
        **self == other.as_ref()
    }
}

// Implement Eq for Key now that PartialEq is properly implemented
impl Eq for Key {}

impl Serialize for Key {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

// This struct implements the Visitor pattern for Key deserialization
struct KeyVisitor;

impl<'de> Visitor<'de> for KeyVisitor {
    // Specifies what type this visitor will return
    type Value = Key;

    // Provides a description for error messages
    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a valid parameter key string")
    }

    // This method is called when deserializing a string value
    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        // Use the Key::new method to convert the string to a Key
        // and convert any errors to Serde's error type
        Key::new(value).map_err(|e| E::custom(e.to_string()))
    }
}

// The Deserialize implementation then uses this visitor
impl<'de> Deserialize<'de> for Key {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Tell Serde to deserialize the input as a string and use our visitor
        deserializer.deserialize_str(KeyVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_creation() {
        let k = Key::new("My KEY").unwrap(); // "my_key"

        // Key compared with various string types
        assert_eq!(k, "my_key");
        assert_eq!(k, String::from("my_key"));
        assert_eq!(k, &"my_key");

        // String types compared with Key (bidirectional equality)
        assert_eq!("my_key", k);
        assert_eq!(String::from("my_key"), k);
        assert_eq!(&"my_key", k);

        // Key compared with Key
        let k2 = Key::new("my_key").unwrap();
        assert_eq!(k, k2);
    }

    #[test]
    fn test_key_normalization() {
        assert_eq!(
            Key::new("Hello World").unwrap(),
            Key::new("hello world").unwrap()
        );
        assert_eq!(
            Key::new("  Multi  Space  ").unwrap(),
            Key::new("multi space").unwrap()
        );
        assert_eq!(
            Key::new("with_underscore").unwrap(),
            Key::new("with_underscore").unwrap()
        );
    }

    #[test]
    fn test_key_validation() {
        // Empty keys
        assert!(Key::new("").is_err());
        assert!(Key::new("   ").is_err());

        // Invalid characters
        assert!(Key::new("Invalid-Dash").is_err());
        assert!(Key::new("Numbers123").is_err());
        assert!(Key::new("special@char").is_err());

        // Too long
        let long_key = "a".repeat(65);
        assert!(Key::new(long_key).is_err());

        // Valid keys
        assert!(Key::is_valid("valid_key"));
        assert!(Key::is_valid("a_b_c"));
        assert!(!Key::is_valid(""));
        assert!(!Key::is_valid("INVALID!"));
    }

    #[test]
    fn test_try_from_implementations() {
        // Test TryFrom<&str>
        let k1: Result<Key, _> = TryFrom::try_from("test_key");
        assert!(k1.is_ok());
        assert_eq!(k1.unwrap(), "test_key");

        // Test TryFrom<String>
        let k2: Result<Key, _> = TryFrom::try_from(String::from("test_key"));
        assert!(k2.is_ok());
        assert_eq!(k2.unwrap(), "test_key");

        // Test TryFrom<&String>
        let s = String::from("test_key");
        let k3: Result<Key, _> = TryFrom::try_from(&s);
        assert!(k3.is_ok());
        assert_eq!(k3.unwrap(), "test_key");

        // Test from_str
        let k4: Result<Key, _> = "test_key".parse();
        assert!(k4.is_ok());
        assert_eq!(k4.unwrap(), "test_key");
    }

    #[test]
    fn test_utility_methods() {
        let key = Key::new("test_key").unwrap();

        // Test as_str
        assert_eq!(key.as_str(), "test_key");

        // Test into_inner
        let inner = key.into_inner();
        assert_eq!(inner, "test_key");
    }
}
