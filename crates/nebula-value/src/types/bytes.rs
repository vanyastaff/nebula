use core::fmt;
use core::ops::{Deref, Index, Range, RangeFrom, RangeFull, RangeTo};
use core::str::FromStr;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use bytes::{BufMut, BytesMut};
use thiserror::Error;

// ══════════════════════════════════════════════════════════════════════════════
// Error Types
// ══════════════════════════════════════════════════════════════════════════════

/// Result type alias for Bytes operations
pub type BytesResult<T> = Result<T, BytesError>;

/// Rich, typed errors for Bytes operations
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum BytesError {
    #[error("Invalid UTF-8 sequence at byte {position}")]
    InvalidUtf8 { position: usize },

    #[error("Invalid hex string: {reason}")]
    InvalidHex { reason: String },

    #[error("Invalid base64: {reason}")]
    InvalidBase64 { reason: String },

    #[error("Slice out of bounds: {start}..{end} for length {len}")]
    SliceOutOfBounds {
        start: usize,
        end: usize,
        len: usize,
    },

    #[error("Index {index} out of bounds for length {len}")]
    IndexOutOfBounds { index: usize, len: usize },

    #[error("Pattern not found")]
    PatternNotFound,

    #[error("Invalid byte value: {value}")]
    InvalidByte { value: u16 },

    #[error("Capacity overflow")]
    CapacityOverflow,

    #[error("JSON type mismatch: expected string/array, got {found}")]
    #[cfg(feature = "serde")]
    JsonTypeMismatch { found: &'static str },
}

// ══════════════════════════════════════════════════════════════════════════════
// Bytes Type
// ══════════════════════════════════════════════════════════════════════════════

/// High-performance binary data type with zero-copy operations
///
/// Features:
/// - Zero-copy cloning via bytes::Bytes
/// - Efficient slicing and concatenation
/// - Multiple encoding formats (hex, base64, etc.)
/// - Pattern matching and searching
/// - Integration with async I/O ecosystem
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Bytes {
    // Use bytes crate's serde support; avoid serde_bytes to satisfy trait bounds
    inner: bytes::Bytes,
}

impl Bytes {
    // ════════════════════════════════════════════════════════════════
    // Constants
    // ════════════════════════════════════════════════════════════════

    /// Empty bytes constant
    pub const EMPTY: Self = Self {
        inner: bytes::Bytes::new(),
    };

    // Common patterns
    pub const CRLF: &'static [u8] = b"\r\n";
    pub const LF: &'static [u8] = b"\n";
    pub const NULL: &'static [u8] = b"\0";
    pub const SPACE: &'static [u8] = b" ";

    // ════════════════════════════════════════════════════════════════
    // Constructors
    // ════════════════════════════════════════════════════════════════

    /// Creates new Bytes from a vector
    #[inline]
    #[must_use]
    pub fn new(data: impl Into<bytes::Bytes>) -> Self {
        Self { inner: data.into() }
    }

    /// Creates empty Bytes
    #[inline]
    #[must_use]
    pub const fn empty() -> Self {
        Self::EMPTY
    }

    /// Creates Bytes from a static byte slice (zero allocation)
    #[inline]
    #[must_use]
    pub fn from_static(data: &'static [u8]) -> Self {
        Self {
            inner: bytes::Bytes::from_static(data),
        }
    }

    /// Creates Bytes by copying a slice
    #[inline]
    #[must_use]
    pub fn copy_from_slice(data: &[u8]) -> Self {
        Self {
            inner: bytes::Bytes::copy_from_slice(data),
        }
    }

    /// Creates Bytes filled with zeros
    #[must_use]
    pub fn zeros(len: usize) -> Self {
        Self {
            inner: bytes::Bytes::from(vec![0u8; len]),
        }
    }

    /// Creates Bytes filled with a specific byte
    #[must_use]
    pub fn repeat(byte: u8, len: usize) -> Self {
        Self {
            inner: bytes::Bytes::from(vec![byte; len]),
        }
    }

    /// Creates a builder for efficient construction
    #[inline]
    #[must_use]
    pub fn builder() -> BytesBuilder {
        BytesBuilder::new()
    }

    /// Creates a builder with specified capacity
    #[inline]
    #[must_use]
    pub fn builder_with_capacity(capacity: usize) -> BytesBuilder {
        BytesBuilder::with_capacity(capacity)
    }

    // ════════════════════════════════════════════════════════════════
    // Properties
    // ════════════════════════════════════════════════════════════════

    /// Returns the length in bytes
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if empty
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns a byte slice view
    #[inline]
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.inner
    }

    /// Converts to a vector (copies data)
    #[inline]
    #[must_use]
    pub fn to_vec(&self) -> Vec<u8> {
        self.inner.to_vec()
    }

    /// Returns the inner bytes::Bytes
    #[inline]
    #[must_use]
    pub fn into_inner(self) -> bytes::Bytes {
        self.inner
    }

    /// Creates from bytes::Bytes
    #[inline]
    #[must_use]
    pub fn from_bytes(bytes: bytes::Bytes) -> Self {
        Self { inner: bytes }
    }

    // ════════════════════════════════════════════════════════════════
    // Slicing Operations (Zero-Copy)
    // ════════════════════════════════════════════════════════════════

    /// Returns a slice of bytes (zero-copy)
    #[inline]
    pub fn slice(&self, range: impl core::ops::RangeBounds<usize>) -> Self {
        use core::ops::Bound;

        let start = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n + 1,
            Bound::Unbounded => 0,
        };

        let end = match range.end_bound() {
            Bound::Included(&n) => n + 1,
            Bound::Excluded(&n) => n,
            Bound::Unbounded => self.len(),
        };

        Self {
            inner: self.inner.slice(start..end),
        }
    }

    /// Returns first n bytes
    #[inline]
    #[must_use]
    pub fn take(&self, n: usize) -> Self {
        self.slice(..n.min(self.len()))
    }

    /// Returns bytes after skipping n
    #[inline]
    #[must_use]
    pub fn skip(&self, n: usize) -> Self {
        self.slice(n.min(self.len())..)
    }

    /// Splits at index (zero-copy)
    #[inline]
    #[must_use]
    pub fn split_at(&self, mid: usize) -> (Self, Self) {
        let first = self.slice(..mid);
        let second = self.slice(mid..);
        (first, second)
    }

    /// Splits off at index (consumes self)
    #[inline]
    pub fn split_off(&mut self, at: usize) -> Self {
        Self {
            inner: self.inner.split_off(at),
        }
    }

    /// Splits to chunks of specified size
    pub fn chunks(&self, chunk_size: usize) -> impl Iterator<Item = Self> + '_ {
        (0..self.len()).step_by(chunk_size).map(move |i| {
            let end = (i + chunk_size).min(self.len());
            self.slice(i..end)
        })
    }

    // ════════════════════════════════════════════════════════════════
    // Element Access
    // ════════════════════════════════════════════════════════════════

    /// Gets byte at index
    #[inline]
    #[must_use]
    pub fn get(&self, index: usize) -> Option<u8> {
        self.inner.get(index).copied()
    }

    /// Gets byte at index with bounds check
    pub fn try_get(&self, index: usize) -> BytesResult<u8> {
        self.get(index).ok_or(BytesError::IndexOutOfBounds {
            index,
            len: self.len(),
        })
    }

    /// Returns first byte
    #[inline]
    #[must_use]
    pub fn first(&self) -> Option<u8> {
        self.inner.first().copied()
    }

    /// Returns last byte
    #[inline]
    #[must_use]
    pub fn last(&self) -> Option<u8> {
        self.inner.last().copied()
    }

    // ════════════════════════════════════════════════════════════════
    // Pattern Matching
    // ════════════════════════════════════════════════════════════════

    /// Checks if starts with pattern
    #[inline]
    #[must_use]
    pub fn starts_with(&self, needle: &[u8]) -> bool {
        self.inner.starts_with(needle)
    }

    /// Checks if ends with pattern
    #[inline]
    #[must_use]
    pub fn ends_with(&self, needle: &[u8]) -> bool {
        self.inner.ends_with(needle)
    }

    /// Finds first occurrence of pattern
    #[must_use]
    pub fn find(&self, needle: &[u8]) -> Option<usize> {
        if needle.is_empty() {
            return Some(0);
        }
        self.inner
            .windows(needle.len())
            .position(|window| window == needle)
    }

    /// Finds last occurrence of pattern
    #[must_use]
    pub fn rfind(&self, needle: &[u8]) -> Option<usize> {
        if needle.is_empty() {
            return Some(self.len());
        }
        self.inner
            .windows(needle.len())
            .rposition(|window| window == needle)
    }

    /// Checks if contains pattern
    #[inline]
    #[must_use]
    pub fn contains(&self, needle: &[u8]) -> bool {
        self.find(needle).is_some()
    }

    /// Counts occurrences of byte
    #[inline]
    #[must_use]
    pub fn count_byte(&self, byte: u8) -> usize {
        self.inner.iter().filter(|&&b| b == byte).count()
    }

    /// Strips prefix if present
    #[must_use]
    pub fn strip_prefix(&self, prefix: &[u8]) -> Option<Self> {
        if self.starts_with(prefix) {
            Some(self.slice(prefix.len()..))
        } else {
            None
        }
    }

    /// Strips suffix if present
    #[must_use]
    pub fn strip_suffix(&self, suffix: &[u8]) -> Option<Self> {
        if self.ends_with(suffix) {
            Some(self.slice(..self.len() - suffix.len()))
        } else {
            None
        }
    }

    /// Splits by delimiter
    pub fn split(&self, delimiter: &[u8]) -> Vec<Self> {
        if delimiter.is_empty() {
            return vec![self.clone()];
        }

        let mut result = Vec::new();
        let mut start = 0;

        while start <= self.len() {
            if let Some(pos) = self.slice(start..).find(delimiter) {
                result.push(self.slice(start..start + pos));
                start += pos + delimiter.len();
            } else {
                result.push(self.slice(start..));
                break;
            }
        }

        result
    }

    /// Joins multiple Bytes with separator
    pub fn join<I>(parts: I, separator: &[u8]) -> Self
    where
        I: IntoIterator<Item = Self>,
    {
        let mut builder = BytesBuilder::new();
        let mut first = true;

        for part in parts {
            if !first {
                builder.extend(separator);
            }
            builder.append(part);
            first = false;
        }

        builder.build()
    }

    // ════════════════════════════════════════════════════════════════
    // String Conversions
    // ════════════════════════════════════════════════════════════════

    /// Converts to UTF-8 string
    pub fn to_utf8(&self) -> BytesResult<String> {
        String::from_utf8(self.to_vec()).map_err(|e| BytesError::InvalidUtf8 {
            position: e.utf8_error().valid_up_to(),
        })
    }

    /// Converts to UTF-8 string (lossy)
    #[inline]
    #[must_use]
    pub fn to_utf8_lossy(&self) -> String {
        String::from_utf8_lossy(&self.inner).into_owned()
    }

    /// Creates from UTF-8 string
    #[inline]
    #[must_use]
    pub fn from_utf8(s: impl AsRef<str>) -> Self {
        Self {
            inner: bytes::Bytes::copy_from_slice(s.as_ref().as_bytes()),
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Hex Encoding
    // ════════════════════════════════════════════════════════════════

    /// Converts to lowercase hex string
    #[must_use]
    pub fn to_hex(&self) -> String {
        const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

        let mut result = String::with_capacity(self.len() * 2);
        for &byte in self.inner.iter() {
            result.push(HEX_CHARS[(byte >> 4) as usize] as char);
            result.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
        }
        result
    }

    /// Converts to uppercase hex string
    #[must_use]
    pub fn to_hex_upper(&self) -> String {
        const HEX_CHARS: &[u8; 16] = b"0123456789ABCDEF";

        let mut result = String::with_capacity(self.len() * 2);
        for &byte in self.inner.iter() {
            result.push(HEX_CHARS[(byte >> 4) as usize] as char);
            result.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
        }
        result
    }

    /// Creates from hex string
    pub fn from_hex(hex: impl AsRef<str>) -> BytesResult<Self> {
        let hex = hex.as_ref();
        let hex = hex.trim();

        // Remove common prefixes
        let hex = hex
            .strip_prefix("0x")
            .or_else(|| hex.strip_prefix("0X"))
            .unwrap_or(hex);

        // Remove spaces and colons
        let cleaned: String = hex
            .chars()
            .filter(|c| !c.is_whitespace() && *c != ':' && *c != '-')
            .collect();

        if cleaned.len() % 2 != 0 {
            return Err(BytesError::InvalidHex {
                reason: "Odd number of hex digits".to_string(),
            });
        }

        let mut bytes = Vec::with_capacity(cleaned.len() / 2);
        let chars: Vec<char> = cleaned.chars().collect();

        for chunk in chars.chunks(2) {
            let high = hex_digit_value(chunk[0]).ok_or_else(|| BytesError::InvalidHex {
                reason: format!("Invalid hex digit: '{}'", chunk[0]),
            })?;
            let low = hex_digit_value(chunk[1]).ok_or_else(|| BytesError::InvalidHex {
                reason: format!("Invalid hex digit: '{}'", chunk[1]),
            })?;
            bytes.push((high << 4) | low);
        }

        Ok(Self::new(bytes))
    }

    // ════════════════════════════════════════════════════════════════
    // Base64 Encoding
    // ════════════════════════════════════════════════════════════════

    /// Converts to base64 string (standard)
    #[cfg(feature = "base64")]
    #[must_use]
    pub fn to_base64(&self) -> String {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        STANDARD.encode(&self.inner)
    }

    /// Converts to base64 string (URL-safe)
    #[cfg(feature = "base64")]
    #[must_use]
    pub fn to_base64_url(&self) -> String {
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
        URL_SAFE_NO_PAD.encode(&self.inner)
    }

    /// Creates from base64 string
    #[cfg(feature = "base64")]
    pub fn from_base64(encoded: impl AsRef<str>) -> BytesResult<Self> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};

        STANDARD
            .decode(encoded.as_ref().trim())
            .map(Self::new)
            .map_err(|e| BytesError::InvalidBase64 {
                reason: e.to_string(),
            })
    }

    /// Creates from base64 string (URL-safe)
    #[cfg(feature = "base64")]
    pub fn from_base64_url(encoded: impl AsRef<str>) -> BytesResult<Self> {
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

        URL_SAFE_NO_PAD
            .decode(encoded.as_ref().trim())
            .map(Self::new)
            .map_err(|e| BytesError::InvalidBase64 {
                reason: e.to_string(),
            })
    }

    // ════════════════════════════════════════════════════════════════
    // Transformations
    // ════════════════════════════════════════════════════════════════

    /// Reverses byte order
    #[must_use]
    pub fn reverse(&self) -> Self {
        let mut vec = self.to_vec();
        vec.reverse();
        Self::new(vec)
    }

    /// Repeats n times
    #[must_use]
    pub fn repeat_n(&self, n: usize) -> Self {
        if n == 0 {
            return Self::empty();
        }
        if n == 1 {
            return self.clone();
        }

        let mut builder = BytesBuilder::with_capacity(self.len() * n);
        for _ in 0..n {
            builder.extend(&self.inner);
        }
        builder.build()
    }

    /// XOR with another Bytes
    pub fn xor(&self, other: &Self) -> BytesResult<Self> {
        if self.len() != other.len() {
            return Err(BytesError::InvalidByte {
                value: self.len() as u16,
            });
        }

        let result: Vec<u8> = self
            .inner
            .iter()
            .zip(other.inner.iter())
            .map(|(a, b)| a ^ b)
            .collect();

        Ok(Self::new(result))
    }

    // ════════════════════════════════════════════════════════════════
    // Checksums and Hashing
    // ════════════════════════════════════════════════════════════════

    /// Simple checksum (sum of all bytes)
    #[inline]
    #[must_use]
    pub fn checksum(&self) -> u8 {
        self.inner.iter().fold(0u8, |acc, &b| acc.wrapping_add(b))
    }

    /// XOR checksum
    #[inline]
    #[must_use]
    pub fn checksum_xor(&self) -> u8 {
        self.inner.iter().fold(0u8, |acc, &b| acc ^ b)
    }

    /// Simple hash (for non-crypto use)
    #[must_use]
    pub fn hash_simple(&self) -> u64 {
        let mut hash = 0u64;
        for &byte in self.inner.iter() {
            hash = hash.wrapping_mul(31).wrapping_add(byte as u64);
        }
        hash
    }

    // ════════════════════════════════════════════════════════════════
    // Statistics
    // ════════════════════════════════════════════════════════════════

    /// Calculates entropy (0.0 to 8.0)
    #[must_use]
    pub fn entropy(&self) -> f64 {
        if self.is_empty() {
            return 0.0;
        }

        let mut counts = [0usize; 256];
        for &byte in self.inner.iter() {
            counts[byte as usize] += 1;
        }

        let len = self.len() as f64;
        let mut entropy = 0.0;

        for &count in counts.iter() {
            if count > 0 {
                let p = count as f64 / len;
                entropy -= p * p.log2();
            }
        }

        entropy
    }

    /// Checks if data appears compressed/encrypted (high entropy)
    #[inline]
    #[must_use]
    pub fn appears_compressed(&self) -> bool {
        self.entropy() > 7.5
    }

    /// Counts unique bytes
    #[must_use]
    pub fn unique_bytes(&self) -> usize {
        let mut seen = [false; 256];
        for &byte in self.inner.iter() {
            seen[byte as usize] = true;
        }
        seen.iter().filter(|&&b| b).count()
    }

    // ════════════════════════════════════════════════════════════════
    // File Type Detection
    // ════════════════════════════════════════════════════════════════

    /// Detects file type by magic bytes
    #[must_use]
    pub fn detect_file_type(&self) -> Option<&'static str> {
        if self.len() < 4 {
            return None;
        }

        let bytes = self.as_slice();

        // Check magic bytes
        match bytes {
            // Images
            [0xFF, 0xD8, 0xFF, ..] => Some("jpeg"),
            [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, ..] => Some("png"),
            [b'G', b'I', b'F', b'8', ..] => Some("gif"),
            [b'B', b'M', ..] if self.len() >= 14 => Some("bmp"),
            [b'R', b'I', b'F', b'F', ..] if self.len() >= 12 => match &bytes[8..12] {
                b"WEBP" => Some("webp"),
                b"WAVE" => Some("wav"),
                _ => None,
            },

            // Documents
            [0x25, b'P', b'D', b'F', ..] => Some("pdf"),
            [b'P', b'K', 0x03, 0x04, ..] | [b'P', b'K', 0x05, 0x06, ..] => Some("zip"),

            // Compression
            [0x1F, 0x8B, ..] => Some("gzip"),
            [b'B', b'Z', b'h', ..] => Some("bzip2"),

            // Executables
            [0x7F, b'E', b'L', b'F', ..] => Some("elf"),
            [b'M', b'Z', ..] => Some("exe"),

            _ => None,
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// BytesBuilder
// ══════════════════════════════════════════════════════════════════════════════

/// Efficient builder for Bytes
#[derive(Debug, Clone, Default)]
pub struct BytesBuilder {
    inner: BytesMut,
}

impl BytesBuilder {
    /// Creates new builder
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: BytesMut::new(),
        }
    }

    /// Creates builder with capacity
    #[inline]
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: BytesMut::with_capacity(capacity),
        }
    }

    /// Returns current length
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if empty
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns capacity
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Reserves capacity
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional);
    }

    /// Pushes a byte
    #[inline]
    pub fn push(&mut self, byte: u8) -> &mut Self {
        self.inner.put_u8(byte);
        self
    }

    /// Extends with a slice
    #[inline]
    pub fn extend(&mut self, data: impl AsRef<[u8]>) -> &mut Self {
        self.inner.extend_from_slice(data.as_ref());
        self
    }

    /// Appends another Bytes
    #[inline]
    pub fn append(&mut self, other: Bytes) -> &mut Self {
        self.inner.extend_from_slice(&other.inner);
        self
    }

    /// Pushes u16 (big-endian)
    #[inline]
    pub fn push_u16(&mut self, val: u16) -> &mut Self {
        self.inner.put_u16(val);
        self
    }

    /// Pushes u32 (big-endian)
    #[inline]
    pub fn push_u32(&mut self, val: u32) -> &mut Self {
        self.inner.put_u32(val);
        self
    }

    /// Pushes u64 (big-endian)
    #[inline]
    pub fn push_u64(&mut self, val: u64) -> &mut Self {
        self.inner.put_u64(val);
        self
    }

    /// Pushes u16 (little-endian)
    #[inline]
    pub fn push_u16_le(&mut self, val: u16) -> &mut Self {
        self.inner.put_u16_le(val);
        self
    }

    /// Pushes u32 (little-endian)
    #[inline]
    pub fn push_u32_le(&mut self, val: u32) -> &mut Self {
        self.inner.put_u32_le(val);
        self
    }

    /// Pushes u64 (little-endian)
    #[inline]
    pub fn push_u64_le(&mut self, val: u64) -> &mut Self {
        self.inner.put_u64_le(val);
        self
    }

    /// Clears the builder
    #[inline]
    pub fn clear(&mut self) -> &mut Self {
        self.inner.clear();
        self
    }

    /// Builds the final Bytes
    #[inline]
    #[must_use]
    pub fn build(self) -> Bytes {
        Bytes {
            inner: self.inner.freeze(),
        }
    }

    /// Builds and resets for reuse
    #[inline]
    pub fn build_and_reset(&mut self) -> Bytes {
        let inner = self.inner.split().freeze();
        Bytes { inner }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Helper Functions
// ══════════════════════════════════════════════════════════════════════════════

#[inline]
fn hex_digit_value(c: char) -> Option<u8> {
    match c {
        '0'..='9' => Some(c as u8 - b'0'),
        'a'..='f' => Some(c as u8 - b'a' + 10),
        'A'..='F' => Some(c as u8 - b'A' + 10),
        _ => None,
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Trait Implementations
// ══════════════════════════════════════════════════════════════════════════════

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.len() <= 32 {
            write!(f, "Bytes({})", self.to_hex())
        } else {
            write!(
                f,
                "Bytes({} bytes, {}...)",
                self.len(),
                self.take(16).to_hex()
            )
        }
    }
}

impl fmt::Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl FromStr for Bytes {
    type Err = BytesError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Try hex first (with 0x prefix or all hex chars)
        if s.starts_with("0x") || s.starts_with("0X") {
            return Self::from_hex(s);
        }

        // Check if it looks like hex
        let is_hex = s
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c.is_whitespace() || c == ':' || c == '-');

        if is_hex && !s.is_empty() {
            Self::from_hex(s)
        } else {
            // Try base64
            #[cfg(feature = "base64")]
            {
                Self::from_base64(s)
            }
            #[cfg(not(feature = "base64"))]
            {
                // Fallback to UTF-8
                Ok(Self::from_utf8(s))
            }
        }
    }
}

impl Deref for Bytes {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl AsRef<[u8]> for Bytes {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.inner
    }
}

// Index implementations
impl Index<usize> for Bytes {
    type Output = u8;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.inner[index]
    }
}

impl Index<Range<usize>> for Bytes {
    type Output = [u8];

    #[inline]
    fn index(&self, range: Range<usize>) -> &Self::Output {
        &self.inner[range]
    }
}

impl Index<RangeFrom<usize>> for Bytes {
    type Output = [u8];

    #[inline]
    fn index(&self, range: RangeFrom<usize>) -> &Self::Output {
        &self.inner[range]
    }
}

impl Index<RangeTo<usize>> for Bytes {
    type Output = [u8];

    #[inline]
    fn index(&self, range: RangeTo<usize>) -> &Self::Output {
        &self.inner[range]
    }
}

impl Index<RangeFull> for Bytes {
    type Output = [u8];

    #[inline]
    fn index(&self, _: RangeFull) -> &Self::Output {
        &self.inner[..]
    }
}

// Conversion traits
impl From<Vec<u8>> for Bytes {
    #[inline]
    fn from(vec: Vec<u8>) -> Self {
        Self::new(vec)
    }
}

impl From<&[u8]> for Bytes {
    #[inline]
    fn from(slice: &[u8]) -> Self {
        Self::copy_from_slice(slice)
    }
}

impl From<&str> for Bytes {
    #[inline]
    fn from(s: &str) -> Self {
        Self::from_utf8(s)
    }
}

impl From<String> for Bytes {
    #[inline]
    fn from(s: String) -> Self {
        Self::new(s.into_bytes())
    }
}

impl From<bytes::Bytes> for Bytes {
    #[inline]
    fn from(bytes: bytes::Bytes) -> Self {
        Self { inner: bytes }
    }
}

impl From<Bytes> for bytes::Bytes {
    #[inline]
    fn from(bytes: Bytes) -> Self {
        bytes.inner
    }
}

impl From<Bytes> for Vec<u8> {
    #[inline]
    fn from(bytes: Bytes) -> Self {
        bytes.to_vec()
    }
}

impl<const N: usize> From<[u8; N]> for Bytes {
    #[inline]
    fn from(arr: [u8; N]) -> Self {
        Self::copy_from_slice(&arr)
    }
}

impl<const N: usize> From<&[u8; N]> for Bytes {
    #[inline]
    fn from(arr: &[u8; N]) -> Self {
        Self::copy_from_slice(arr)
    }
}

// Iterator support
impl FromIterator<u8> for Bytes {
    fn from_iter<T: IntoIterator<Item = u8>>(iter: T) -> Self {
        let vec: Vec<u8> = iter.into_iter().collect();
        Self::new(vec)
    }
}

impl IntoIterator for Bytes {
    type Item = u8;
    type IntoIter = bytes::buf::IntoIter<bytes::Bytes>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<'a> IntoIterator for &'a Bytes {
    type Item = &'a u8;
    type IntoIter = core::slice::Iter<'a, u8>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
    }
}

impl Extend<u8> for Bytes {
    fn extend<T: IntoIterator<Item = u8>>(&mut self, iter: T) {
        let vec: Vec<u8> = iter.into_iter().collect();
        if !vec.is_empty() {
            let mut builder = BytesBuilder::new();
            builder.append(self.clone());
            builder.extend(&vec);
            *self = builder.build();
        }
    }
}

// Comparison with byte slices
impl PartialEq<[u8]> for Bytes {
    #[inline]
    fn eq(&self, other: &[u8]) -> bool {
        &**self == other
    }
}

impl PartialEq<&[u8]> for Bytes {
    #[inline]
    fn eq(&self, other: &&[u8]) -> bool {
        &**self == *other
    }
}

impl PartialEq<Vec<u8>> for Bytes {
    #[inline]
    fn eq(&self, other: &Vec<u8>) -> bool {
        **self == other[..]
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// JSON Support
// ══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "serde")]
impl From<Bytes> for serde_json::Value {
    fn from(bytes: Bytes) -> Self {
        // Encode as base64 for JSON
        #[cfg(feature = "base64")]
        {
            serde_json::Value::String(bytes.to_base64())
        }
        #[cfg(not(feature = "base64"))]
        {
            // Fallback to hex
            serde_json::Value::String(bytes.to_hex())
        }
    }
}

#[cfg(feature = "serde")]
impl TryFrom<serde_json::Value> for Bytes {
    type Error = BytesError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        match value {
            serde_json::Value::String(s) => {
                // Try base64 first, then hex
                #[cfg(feature = "base64")]
                {
                    Self::from_base64(&s).or_else(|_| Self::from_hex(&s))
                }
                #[cfg(not(feature = "base64"))]
                {
                    Self::from_hex(&s)
                }
            }
            serde_json::Value::Array(arr) => {
                // Array of bytes
                let mut bytes = Vec::with_capacity(arr.len());
                for val in arr {
                    match val {
                        serde_json::Value::Number(n) => {
                            let byte = n.as_u64().and_then(|n| u8::try_from(n).ok()).ok_or(
                                BytesError::InvalidByte {
                                    value: n.as_u64().unwrap_or(256) as u16,
                                },
                            )?;
                            bytes.push(byte);
                        }
                        _ => {
                            return Err(BytesError::JsonTypeMismatch {
                                found: "non-number in array",
                            });
                        }
                    }
                }
                Ok(Self::new(bytes))
            }
            serde_json::Value::Null => Ok(Self::empty()),
            _ => Err(BytesError::JsonTypeMismatch {
                found: match value {
                    serde_json::Value::Bool(_) => "bool",
                    serde_json::Value::Number(_) => "number",
                    serde_json::Value::Object(_) => "object",
                    _ => "unknown",
                },
            }),
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Send + Sync
// ══════════════════════════════════════════════════════════════════════════════

// Bytes is automatically Send + Sync because bytes::Bytes is Send + Sync

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_operations() {
        let bytes = Bytes::from_utf8("hello");
        assert_eq!(bytes.len(), 5);
        assert!(!bytes.is_empty());
        assert_eq!(bytes.as_slice(), b"hello");
    }

    #[test]
    fn test_slicing() {
        let bytes = Bytes::from_utf8("hello world");
        let slice = bytes.slice(0..5);
        assert_eq!(slice.as_slice(), b"hello");

        let (first, second) = bytes.split_at(6);
        assert_eq!(first.as_slice(), b"hello ");
        assert_eq!(second.as_slice(), b"world");
    }

    #[test]
    fn test_builder() {
        let mut builder = Bytes::builder();
        builder
            .extend(b"hello")
            .push(b' ')
            .extend(b"world")
            .push_u16(0x1234);

        let bytes = builder.build();
        assert_eq!(&bytes[0..11], b"hello world");
        assert_eq!(bytes[11], 0x12);
        assert_eq!(bytes[12], 0x34);
    }

    #[test]
    fn test_hex_encoding() {
        let bytes = Bytes::from([0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(bytes.to_hex(), "deadbeef");
        assert_eq!(bytes.to_hex_upper(), "DEADBEEF");

        let decoded = Bytes::from_hex("dead beef").unwrap();
        assert_eq!(decoded, bytes);

        let decoded2 = Bytes::from_hex("0xDEADBEEF").unwrap();
        assert_eq!(decoded2, bytes);
    }

    #[cfg(feature = "base64")]
    #[test]
    fn test_base64_encoding() {
        let bytes = Bytes::from_utf8("hello world");
        let encoded = bytes.to_base64();
        assert_eq!(encoded, "aGVsbG8gd29ybGQ=");

        let decoded = Bytes::from_base64(&encoded).unwrap();
        assert_eq!(decoded, bytes);
    }

    #[test]
    fn test_pattern_matching() {
        let bytes = Bytes::from_utf8("hello world hello");

        assert!(bytes.starts_with(b"hello"));
        assert!(bytes.ends_with(b"hello"));
        assert!(bytes.contains(b"world"));

        assert_eq!(bytes.find(b"hello"), Some(0));
        assert_eq!(bytes.rfind(b"hello"), Some(12));
        assert_eq!(bytes.count_byte(b'l'), 5);

        let stripped = bytes.strip_prefix(b"hello ").unwrap();
        assert_eq!(stripped.as_slice(), b"world hello");
    }

    #[test]
    fn test_split_join() {
        let bytes = Bytes::from_utf8("a,b,c");
        let parts = bytes.split(b",");
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].as_slice(), b"a");
        assert_eq!(parts[1].as_slice(), b"b");
        assert_eq!(parts[2].as_slice(), b"c");

        let joined = Bytes::join(parts, b", ");
        assert_eq!(joined.as_slice(), b"a, b, c");
    }

    #[test]
    fn test_entropy() {
        let low_entropy = Bytes::zeros(100);
        assert!(low_entropy.entropy() < 0.1);
        assert!(!low_entropy.appears_compressed());

        let high_entropy = Bytes::from((0..=255u8).collect::<Vec<_>>());
        assert!(high_entropy.entropy() > 7.9);
        assert!(high_entropy.appears_compressed());
    }

    #[test]
    fn test_file_detection() {
        let jpeg = Bytes::from([0xFF, 0xD8, 0xFF, 0xE0]);
        assert_eq!(jpeg.detect_file_type(), Some("jpeg"));

        let png = Bytes::from([0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(png.detect_file_type(), Some("png"));

        let pdf = Bytes::from(b"%PDF-1.4");
        assert_eq!(pdf.detect_file_type(), Some("pdf"));
    }

    #[test]
    fn test_chunks() {
        let bytes = Bytes::from_utf8("abcdefgh");
        let chunks: Vec<_> = bytes.chunks(3).collect();
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].as_slice(), b"abc");
        assert_eq!(chunks[1].as_slice(), b"def");
        assert_eq!(chunks[2].as_slice(), b"gh");
    }

    #[test]
    fn test_xor() {
        let a = Bytes::from([0xFF, 0x00, 0xAA]);
        let b = Bytes::from([0x00, 0xFF, 0x55]);
        let result = a.xor(&b).unwrap();
        assert_eq!(result.as_slice(), &[0xFF, 0xFF, 0xFF]);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_json_conversion() {
        use serde_json::json;

        let bytes = Bytes::from_utf8("test");
        let json = serde_json::Value::from(bytes.clone());

        // Should be base64 encoded
        #[cfg(feature = "base64")]
        assert_eq!(json, json!("dGVzdA=="));

        // Round trip
        let decoded = Bytes::try_from(json).unwrap();
        assert_eq!(decoded, bytes);

        // From array
        let arr = json!([116, 101, 115, 116]);
        let from_arr = Bytes::try_from(arr).unwrap();
        assert_eq!(from_arr, bytes);
    }
}
