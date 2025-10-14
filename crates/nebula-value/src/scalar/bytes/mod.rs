//! Binary data (Bytes) type for nebula-value
//!
//! This module provides a Bytes type that:
//! - Efficient storage using bytes crate
//! - Base64 encoding/decoding
//! - Length limits for DoS protection
//! - Zero-copy cloning

use std::fmt;
use std::hash::{Hash, Hasher};

use base64::Engine;
use bytes::Bytes as BytesBuf;

use crate::core::limits::ValueLimits;
use crate::core::{ValueError, ValueResult};

/// Binary data with efficient cloning
///
/// Uses `bytes::Bytes` internally which provides:
/// - Reference-counted storage
/// - Zero-copy cloning
/// - Shared immutable data
#[derive(Debug, Clone)]
pub struct Bytes {
    inner: BytesBuf,
}

impl Bytes {
    /// Create new Bytes from a Vec<u8>
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            inner: BytesBuf::from(data),
        }
    }

    /// Create from a byte slice (allocates)
    pub fn from_slice(data: &[u8]) -> Self {
        Self {
            inner: BytesBuf::copy_from_slice(data),
        }
    }

    /// Create with length validation
    pub fn with_limits(data: Vec<u8>, limits: &ValueLimits) -> ValueResult<Self> {
        limits.check_bytes_length(data.len())?;
        Ok(Self::new(data))
    }

    /// Create from slice with length validation
    pub fn from_slice_with_limits(data: &[u8], limits: &ValueLimits) -> ValueResult<Self> {
        limits.check_bytes_length(data.len())?;
        Ok(Self::from_slice(data))
    }

    /// Create from base64 encoded string
    pub fn from_base64(encoded: &str) -> ValueResult<Self> {
        let engine = base64::engine::general_purpose::STANDARD;
        let decoded = engine
            .decode(encoded)
            .map_err(|e| ValueError::parse_error("base64", e.to_string()))?;

        Ok(Self::new(decoded))
    }

    /// Create from base64 with length validation
    pub fn from_base64_with_limits(encoded: &str, limits: &ValueLimits) -> ValueResult<Self> {
        let bytes = Self::from_base64(encoded)?;
        limits.check_bytes_length(bytes.len())?;
        Ok(bytes)
    }

    /// Encode to base64 string
    pub fn to_base64(&self) -> String {
        let engine = base64::engine::general_purpose::STANDARD;
        engine.encode(&self.inner)
    }

    /// Get the byte slice
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.inner
    }

    /// Get the length in bytes
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get a sub-slice by range
    pub fn slice(&self, start: usize, end: usize) -> ValueResult<Bytes> {
        if start > end || end > self.len() {
            return Err(ValueError::out_of_range(
                format!("{}..{}", start, end),
                "0",
                self.len().to_string(),
            ));
        }

        Ok(Self {
            inner: self.inner.slice(start..end),
        })
    }

    /// Concatenate with another Bytes
    pub fn concat(&self, other: &Bytes) -> Bytes {
        let mut result = Vec::with_capacity(self.len() + other.len());
        result.extend_from_slice(&self.inner);
        result.extend_from_slice(&other.inner);
        Self::new(result)
    }

    /// Convert to Vec<u8> (allocates if not uniquely owned)
    pub fn to_vec(&self) -> Vec<u8> {
        self.inner.to_vec()
    }

    /// Get underlying BytesBuf for zero-copy operations
    pub fn into_inner(self) -> BytesBuf {
        self.inner
    }
}

impl PartialEq for Bytes {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for Bytes {}

impl PartialOrd for Bytes {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Bytes {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.inner.cmp(&other.inner)
    }
}

impl Hash for Bytes {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
    }
}

impl fmt::Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{} bytes>", self.len())
    }
}

// Conversions
impl From<Vec<u8>> for Bytes {
    fn from(data: Vec<u8>) -> Self {
        Self::new(data)
    }
}

impl From<&[u8]> for Bytes {
    fn from(data: &[u8]) -> Self {
        Self::from_slice(data)
    }
}

impl From<BytesBuf> for Bytes {
    fn from(buf: BytesBuf) -> Self {
        Self { inner: buf }
    }
}

impl From<Bytes> for Vec<u8> {
    fn from(bytes: Bytes) -> Self {
        bytes.to_vec()
    }
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bytes_creation() {
        let bytes = Bytes::new(vec![1, 2, 3, 4, 5]);
        assert_eq!(bytes.len(), 5);
        assert_eq!(bytes.as_slice(), &[1, 2, 3, 4, 5]);
        assert!(!bytes.is_empty());
    }

    #[test]
    fn test_bytes_from_slice() {
        let data = [1, 2, 3, 4, 5];
        let bytes = Bytes::from_slice(&data);
        assert_eq!(bytes.as_slice(), &data);
    }

    #[test]
    fn test_bytes_with_limits() {
        let limits = ValueLimits::strict();

        // Should succeed
        let bytes = Bytes::with_limits(vec![1, 2, 3], &limits);
        assert!(bytes.is_ok());

        // Should fail - too large
        let large_data = vec![0u8; 20_000_000];
        let bytes = Bytes::with_limits(large_data, &limits);
        assert!(bytes.is_err());
    }

    #[test]
    fn test_bytes_base64_encode() {
        let bytes = Bytes::new(vec![72, 101, 108, 108, 111]); // "Hello"
        let encoded = bytes.to_base64();
        assert_eq!(encoded, "SGVsbG8=");
    }

    #[test]
    fn test_bytes_base64_decode() {
        let encoded = "SGVsbG8=";
        let bytes = Bytes::from_base64(encoded).unwrap();
        assert_eq!(bytes.as_slice(), b"Hello");
    }

    #[test]
    fn test_bytes_base64_roundtrip() {
        let original = Bytes::new(vec![1, 2, 3, 4, 5, 255, 0, 128]);
        let encoded = original.to_base64();
        let decoded = Bytes::from_base64(&encoded).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_bytes_base64_invalid() {
        let result = Bytes::from_base64("invalid!!!base64");
        assert!(result.is_err());
    }

    #[test]
    fn test_bytes_slice() {
        let bytes = Bytes::new(vec![1, 2, 3, 4, 5]);

        let sub = bytes.slice(1, 4).unwrap();
        assert_eq!(sub.as_slice(), &[2, 3, 4]);

        // Out of bounds
        assert!(bytes.slice(0, 10).is_err());
        assert!(bytes.slice(5, 3).is_err());
    }

    #[test]
    fn test_bytes_concat() {
        let bytes1 = Bytes::new(vec![1, 2, 3]);
        let bytes2 = Bytes::new(vec![4, 5, 6]);
        let result = bytes1.concat(&bytes2);

        assert_eq!(result.as_slice(), &[1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn test_bytes_equality() {
        let bytes1 = Bytes::new(vec![1, 2, 3]);
        let bytes2 = Bytes::new(vec![1, 2, 3]);
        let bytes3 = Bytes::new(vec![1, 2, 4]);

        assert_eq!(bytes1, bytes2);
        assert_ne!(bytes1, bytes3);
    }

    #[test]
    fn test_bytes_ordering() {
        let bytes1 = Bytes::new(vec![1, 2, 3]);
        let bytes2 = Bytes::new(vec![1, 2, 4]);
        let bytes3 = Bytes::new(vec![1, 3, 0]);

        assert!(bytes1 < bytes2);
        assert!(bytes2 < bytes3);
        assert!(bytes1 < bytes3);
    }

    #[test]
    fn test_bytes_hash() {
        use std::collections::HashMap;

        let mut map = HashMap::new();
        map.insert(Bytes::new(vec![1, 2, 3]), "value1");
        map.insert(Bytes::new(vec![4, 5, 6]), "value2");

        assert_eq!(map.get(&Bytes::new(vec![1, 2, 3])), Some(&"value1"));
        assert_eq!(map.get(&Bytes::new(vec![4, 5, 6])), Some(&"value2"));
        assert_eq!(map.get(&Bytes::new(vec![7, 8, 9])), None);
    }

    #[test]
    fn test_bytes_display() {
        let bytes = Bytes::new(vec![1, 2, 3, 4, 5]);
        assert_eq!(bytes.to_string(), "<5 bytes>");
    }

    #[test]
    fn test_bytes_clone_efficiency() {
        let bytes1 = Bytes::new(vec![1, 2, 3, 4, 5]);
        let bytes2 = bytes1.clone();

        // Both should share the same underlying data (zero-copy)
        assert_eq!(bytes1, bytes2);
        assert_eq!(bytes1.as_slice().as_ptr(), bytes2.as_slice().as_ptr());
    }

    #[test]
    fn test_bytes_empty() {
        let bytes = Bytes::new(vec![]);
        assert!(bytes.is_empty());
        assert_eq!(bytes.len(), 0);
        assert_eq!(bytes.to_base64(), "");
    }
}
