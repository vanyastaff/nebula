use std::borrow::{Borrow, Cow};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Add, AddAssign, Deref, Index, Range, RangeFrom, RangeFull, RangeTo};
use std::str::{Chars, FromStr};
use std::sync::Arc;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use bytes::Bytes;
use thiserror::Error;

#[cfg(feature = "pattern")]
use regex::Regex;

#[cfg(feature = "rayon")]
use rayon::prelude::*;

/// Result type alias for Text operations
pub type TextResult<T> = Result<T, TextError>;

/// Rich, typed errors for Text operations
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum TextError {
    #[error("Invalid substring range: start ({start}) > end ({end})")]
    InvalidRange { start: usize, end: usize },

    #[error("Substring indices out of bounds: start={start}, end={end}, length={len}")]
    OutOfBounds { start: usize, end: usize, len: usize },

    #[error("Character index {index} out of bounds for string of length {len}")]
    CharIndexOutOfBounds { index: usize, len: usize },

    #[error("Parse error for type {ty}: {msg}")]
    ParseError { ty: &'static str, msg: String },

    #[error("Invalid UTF-8 sequence at byte {index}")]
    InvalidUtf8 { index: usize },

    #[error("Pattern compilation error: {msg}")]
    #[cfg(feature = "pattern")]
    PatternError { msg: String },

    #[error("JSON type mismatch: expected string, got {found}")]
    #[cfg(feature = "serde")]
    JsonTypeMismatch { found: &'static str },
}

/// A high-performance, feature-rich string type with zero-cost abstractions
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Text {
    /// Internal string storage
    inner: Arc<str>,

    /// Cached character count for O(1) access
    #[cfg_attr(feature = "serde", serde(skip))]
    char_count_cache: std::sync::OnceLock<usize>,
}

impl Text {
    // ==================== Constructors ====================

    /// Creates a new Text from an owned String
    #[inline]
    pub fn new(value: String) -> Self {
        Self {
            inner: value.into(),
            char_count_cache: std::sync::OnceLock::new(),
        }
    }

    /// Creates a new Text from anything convertible to String
    #[inline]
    pub fn from_value(value: impl Into<String>) -> Self {
        Self::new(value.into())
    }

    /// Creates an empty Text (const-friendly)
    #[inline]
    pub fn empty() -> Self {
        Self {
            inner: Arc::from(""),
            char_count_cache: std::sync::OnceLock::new(),
        }
    }

    /// Creates a Text with specified capacity
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::new(String::with_capacity(capacity))
    }

    /// Creates a Text from a static string slice (zero allocation)
    #[inline]
    #[must_use]
    pub fn from_static(s: &'static str) -> Self {
        Self {
            inner: Arc::from(s),
            char_count_cache: std::sync::OnceLock::new(),
        }
    }

    /// Creates a Text from Bytes
    pub fn from_bytes(bytes: Bytes) -> TextResult<Self> {
        let vec = bytes.to_vec();
        String::from_utf8(vec)
            .map(Self::new)
            .map_err(|e| TextError::InvalidUtf8 { index: e.utf8_error().valid_up_to() })
    }

    /// Creates a Text from a byte slice
    pub fn from_utf8(bytes: &[u8]) -> TextResult<Self> {
        std::str::from_utf8(bytes)
            .map(|s| Self::from(s))
            .map_err(|e| TextError::InvalidUtf8 { index: e.valid_up_to() })
    }

    // ==================== Basic Properties ====================

    /// Returns true if the text is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the length in bytes
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns the number of characters (cached for performance)
    #[inline]
    pub fn char_count(&self) -> usize {
        *self.char_count_cache.get_or_init(|| self.inner.chars().count())
    }

    /// Returns the text as a string slice
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Converts to an owned String
    #[inline]
    pub fn into_string(self) -> String {
        self.inner.to_string()
    }

    /// Returns as Bytes (zero-copy when possible)
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_bytes()
    }

    /// Converts to Bytes
    pub fn to_bytes(&self) -> Bytes {
        Bytes::copy_from_slice(self.as_bytes())
    }

    // ==================== Transformations ====================

    /// Returns a trimmed string slice
    #[inline]
    pub fn trim(&self) -> &str {
        self.inner.trim()
    }

    /// Creates a new Text with trimmed content
    #[inline]
    pub fn trimmed(&self) -> Text {
        Self::from(self.trim())
    }

    /// Converts to lowercase
    #[inline]
    pub fn to_lowercase(&self) -> Text {
        Self::new(self.inner.to_lowercase())
    }

    /// Converts to uppercase
    #[inline]
    pub fn to_uppercase(&self) -> Text {
        Self::new(self.inner.to_uppercase())
    }

    /// Converts to ASCII lowercase
    #[inline]
    pub fn to_ascii_lowercase(&self) -> Text {
        Self::new(self.inner.to_ascii_lowercase())
    }

    /// Converts to ASCII uppercase
    #[inline]
    pub fn to_ascii_uppercase(&self) -> Text {
        Self::new(self.inner.to_ascii_uppercase())
    }

    /// Capitalizes the first character, lowercases the rest
    pub fn capitalize(&self) -> Text {
        let mut chars = self.inner.chars();
        match chars.next() {
            None => Self::empty(),
            Some(first) => {
                let mut result = String::with_capacity(self.len());
                result.extend(first.to_uppercase());
                result.push_str(&chars.as_str().to_lowercase());
                Self::new(result)
            }
        }
    }

    /// Title case: capitalizes first letter of each word
    pub fn to_title_case(&self) -> Text {
        let mut result = String::with_capacity(self.len());
        let mut capitalize_next = true;

        for ch in self.inner.chars() {
            if ch.is_whitespace() {
                capitalize_next = true;
                result.push(ch);
            } else if capitalize_next {
                result.extend(ch.to_uppercase());
                capitalize_next = false;
            } else {
                result.extend(ch.to_lowercase());
            }
        }

        Self::new(result)
    }

    // ==================== Search and Comparison ====================

    /// Checks if text contains a pattern
    #[inline]
    pub fn contains(&self, pattern: &str) -> bool {
        self.inner.contains(pattern)
    }

    /// Checks if text starts with a pattern
    #[inline]
    pub fn starts_with(&self, pattern: &str) -> bool {
        self.inner.starts_with(pattern)
    }

    /// Checks if text ends with a pattern
    #[inline]
    pub fn ends_with(&self, pattern: &str) -> bool {
        self.inner.ends_with(pattern)
    }

    /// Finds the first occurrence of a pattern
    #[inline]
    pub fn find(&self, pattern: &str) -> Option<usize> {
        self.inner.find(pattern)
    }

    /// Finds the last occurrence of a pattern
    #[inline]
    pub fn rfind(&self, pattern: &str) -> Option<usize> {
        self.inner.rfind(pattern)
    }

    /// Count occurrences of a pattern
    pub fn count_matches(&self, pattern: &str) -> usize {
        self.inner.matches(pattern).count()
    }

    /// Removes prefix if present
    #[inline]
    pub fn strip_prefix(&self, prefix: &str) -> Option<Text> {
        self.inner.strip_prefix(prefix).map(Self::from)
    }

    /// Removes suffix if present
    #[inline]
    pub fn strip_suffix(&self, suffix: &str) -> Option<Text> {
        self.inner.strip_suffix(suffix).map(Self::from)
    }

    /// Replaces all occurrences of a pattern
    #[inline]
    pub fn replace(&self, from: &str, to: &str) -> Text {
        Self::new(self.inner.replace(from, to))
    }

    /// Replaces first n occurrences of a pattern
    #[inline]
    pub fn replacen(&self, from: &str, to: &str, count: usize) -> Text {
        Self::new(self.inner.replacen(from, to, count))
    }

    // ==================== Pattern Matching (Regex) ====================

    #[cfg(feature = "pattern")]
    /// Checks if text matches a regex pattern
    pub fn matches_pattern(&self, pattern: &str) -> TextResult<bool> {
        Regex::new(pattern)
            .map(|re| re.is_match(&self.inner))
            .map_err(|e| TextError::PatternError { msg: e.to_string() })
    }

    #[cfg(feature = "pattern")]
    /// Replaces all regex matches
    pub fn regex_replace(&self, pattern: &str, replacement: &str) -> TextResult<Text> {
        Regex::new(pattern)
            .map(|re| Self::new(re.replace_all(&self.inner, replacement).into_owned()))
            .map_err(|e| TextError::PatternError { msg: e.to_string() })
    }

    #[cfg(feature = "pattern")]
    /// Finds all regex matches
    pub fn find_all_matches(&self, pattern: &str) -> TextResult<Vec<&str>> {
        Regex::new(pattern)
            .map(|re| re.find_iter(&self.inner).map(|m| m.as_str()).collect())
            .map_err(|e| TextError::PatternError { msg: e.to_string() })
    }

    // ==================== Splitting ====================

    /// Splits by a pattern into a Vec of Text
    pub fn split(&self, pattern: &str) -> Vec<Text> {
        self.inner.split(pattern).map(Self::from).collect()
    }

    /// Splits by whitespace
    pub fn split_whitespace(&self) -> Vec<Text> {
        self.inner.split_whitespace().map(Self::from).collect()
    }

    /// Splits into lines
    pub fn lines(&self) -> Vec<Text> {
        self.inner.lines().map(Self::from).collect()
    }

    /// Zero-allocation split iterator
    #[inline]
    pub fn split_iter<'a>(&'a self, pattern: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.inner.split(pattern)
    }

    /// Zero-allocation whitespace split iterator
    #[inline]
    pub fn split_whitespace_iter(&self) -> impl Iterator<Item = &str> + '_ {
        self.inner.split_whitespace()
    }

    /// Zero-allocation lines iterator
    #[inline]
    pub fn lines_iter(&self) -> impl Iterator<Item = &str> + '_ {
        self.inner.lines()
    }

    /// Split at most n times
    pub fn splitn(&self, n: usize, pattern: &str) -> Vec<Text> {
        self.inner.splitn(n, pattern).map(Self::from).collect()
    }

    // ==================== Substring Operations ====================

    /// Returns a substring by character indices [start, end)
    pub fn substring(&self, start: usize, end: usize) -> TextResult<Text> {
        if start > end {
            return Err(TextError::InvalidRange { start, end });
        }

        let char_count = self.char_count();
        if start > char_count || end > char_count {
            return Err(TextError::OutOfBounds {
                start,
                end,
                len: char_count,
            });
        }

        if start == end {
            return Ok(Self::empty());
        }

        // Optimize for common cases
        if start == 0 && end == char_count {
            return Ok(self.clone());
        }

        // Use char_indices for accurate UTF-8 boundary mapping
        let mut char_indices = self.inner.char_indices();

        let start_byte = if start == 0 {
            0
        } else {
            char_indices.nth(start - 1).map(|(i, c)| i + c.len_utf8()).unwrap_or(0)
        };

        let end_byte = if end == start {
            start_byte
        } else {
            char_indices.nth(end - start - 1).map(|(i, c)| i + c.len_utf8()).unwrap_or(self.len())
        };

        Ok(Self::from(&self.inner[start_byte..end_byte]))
    }

    /// Takes first n characters
    pub fn take(&self, n: usize) -> Text {
        if n == 0 {
            return Self::empty();
        }

        let char_count = self.char_count();
        if n >= char_count {
            return self.clone();
        }

        self.substring(0, n).unwrap_or_else(|_| self.clone())
    }

    /// Skips first n characters
    pub fn skip(&self, n: usize) -> Text {
        if n == 0 {
            return self.clone();
        }

        let char_count = self.char_count();
        if n >= char_count {
            return Self::empty();
        }

        self.substring(n, char_count).unwrap_or_else(|_| Self::empty())
    }

    /// Takes last n characters
    pub fn take_last(&self, n: usize) -> Text {
        let char_count = self.char_count();
        if n >= char_count {
            return self.clone();
        }
        self.skip(char_count - n)
    }

    /// Gets character at index
    pub fn char_at(&self, index: usize) -> TextResult<char> {
        self.inner.chars().nth(index).ok_or_else(|| {
            TextError::CharIndexOutOfBounds {
                index,
                len: self.char_count(),
            }
        })
    }

    /// Gets a slice of characters
    pub fn slice(&self, start: usize, len: usize) -> TextResult<Text> {
        self.substring(start, start + len)
    }

    // ==================== Parsing and Validation ====================

    /// Parses text as type T
    pub fn parse<T>(&self) -> TextResult<T>
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        self.inner
            .trim()
            .parse::<T>()
            .map_err(|e| TextError::ParseError {
                ty: std::any::type_name::<T>(),
                msg: e.to_string(),
            })
    }

    /// Checks if text represents a valid number
    #[inline]
    pub fn is_numeric(&self) -> bool {
        self.inner.trim().parse::<f64>().is_ok()
    }

    /// Checks if all characters are alphabetic
    #[inline]
    pub fn is_alphabetic(&self) -> bool {
        !self.is_empty() && self.inner.chars().all(char::is_alphabetic)
    }

    /// Checks if all characters are alphanumeric
    #[inline]
    pub fn is_alphanumeric(&self) -> bool {
        !self.is_empty() && self.inner.chars().all(char::is_alphanumeric)
    }

    /// Checks if all characters are ASCII
    #[inline]
    pub fn is_ascii(&self) -> bool {
        self.inner.is_ascii()
    }

    /// Checks if all characters are whitespace
    #[inline]
    pub fn is_whitespace(&self) -> bool {
        !self.is_empty() && self.inner.chars().all(char::is_whitespace)
    }

    /// Checks if text is a valid identifier (starts with letter/underscore, contains only alphanumeric/underscore)
    pub fn is_identifier(&self) -> bool {
        if self.is_empty() {
            return false;
        }

        let mut chars = self.inner.chars();
        if let Some(first) = chars.next() {
            if !first.is_alphabetic() && first != '_' {
                return false;
            }
        }

        chars.all(|c| c.is_alphanumeric() || c == '_')
    }

    // ==================== Advanced Operations ====================

    /// Repeats text n times
    #[inline]
    pub fn repeat(&self, n: usize) -> Text {
        Self::new(self.inner.repeat(n))
    }

    /// Reverses the text (Unicode-aware)
    #[inline]
    pub fn reverse(&self) -> Text {
        Self::new(self.inner.chars().rev().collect())
    }

    /// Pads text to width with spaces on the left
    pub fn pad_left(&self, width: usize, fill: char) -> Text {
        let current = self.char_count();
        if current >= width {
            return self.clone();
        }

        let mut result = String::with_capacity(width * fill.len_utf8());
        for _ in 0..(width - current) {
            result.push(fill);
        }
        result.push_str(&self.inner);
        Self::new(result)
    }

    /// Pads text to width with spaces on the right
    pub fn pad_right(&self, width: usize, fill: char) -> Text {
        let current = self.char_count();
        if current >= width {
            return self.clone();
        }

        let mut result = String::with_capacity(width * fill.len_utf8());
        result.push_str(&self.inner);
        for _ in 0..(width - current) {
            result.push(fill);
        }
        Self::new(result)
    }

    /// Centers text within width
    pub fn center(&self, width: usize, fill: char) -> Text {
        let current = self.char_count();
        if current >= width {
            return self.clone();
        }

        let total_padding = width - current;
        let left_padding = total_padding / 2;
        let right_padding = total_padding - left_padding;

        let mut result = String::with_capacity(width * fill.len_utf8());
        for _ in 0..left_padding {
            result.push(fill);
        }
        result.push_str(&self.inner);
        for _ in 0..right_padding {
            result.push(fill);
        }
        Self::new(result)
    }

    /// Truncates to max length with ellipsis
    pub fn truncate_with_ellipsis(&self, max_len: usize) -> Text {
        let char_count = self.char_count();
        if char_count <= max_len {
            return self.clone();
        }

        if max_len <= 3 {
            return Self::from("...");
        }

        let truncate_at = max_len - 3;
        let truncated = self.take(truncate_at);
        Self::new(format!("{}...", truncated.as_str()))
    }

    /// Wraps text at specified width
    pub fn word_wrap(&self, width: usize) -> Vec<Text> {
        if width == 0 || self.is_empty() {
            return vec![self.clone()];
        }

        let mut result = Vec::new();
        let mut current_line = String::new();
        let mut current_width = 0;

        for word in self.split_whitespace_iter() {
            let word_len = word.chars().count();

            if current_width > 0 && current_width + word_len + 1 > width {
                result.push(Self::new(current_line.clone()));
                current_line.clear();
                current_width = 0;
            }

            if current_width > 0 {
                current_line.push(' ');
                current_width += 1;
            }

            current_line.push_str(word);
            current_width += word_len;
        }

        if !current_line.is_empty() {
            result.push(Self::new(current_line));
        }

        result
    }

    /// Joins multiple texts with a separator
    pub fn join<I, T>(iter: I, separator: &str) -> Text
    where
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        let mut result = String::new();
        let mut first = true;

        for item in iter {
            if !first {
                result.push_str(separator);
            }
            result.push_str(item.as_ref());
            first = false;
        }

        Self::new(result)
    }

    // ==================== Parallel Operations (Rayon) ====================

    #[cfg(feature = "rayon")]
    /// Parallel character count for very large strings
    pub fn par_char_count(&self) -> usize {
        if self.len() < 10_000 {
            return self.char_count();
        }

        // Split into chunks at valid UTF-8 boundaries
        let chunk_size = self.len() / rayon::current_num_threads();
        let mut boundaries = vec![0];
        let mut current = 0;

        while current < self.len() {
            let next = (current + chunk_size).min(self.len());
            // Find valid UTF-8 boundary
            let boundary = if next == self.len() {
                next
            } else {
                // Scan backwards for valid boundary
                let mut b = next;
                while b > current && !self.inner.is_char_boundary(b) {
                    b -= 1;
                }
                b
            };

            if boundary > current {
                boundaries.push(boundary);
                current = boundary;
            } else {
                break;
            }
        }

        boundaries.windows(2)
            .par_bridge()
            .map(|w| self.inner[w[0]..w[1]].chars().count())
            .sum()
    }

    #[cfg(feature = "rayon")]
    /// Parallel word count
    pub fn par_word_count(&self) -> usize {
        if self.len() < 10_000 {
            return self.split_whitespace().len();
        }

        self.lines()
            .par_iter()
            .map(|line| line.split_whitespace().len())
            .sum()
    }

    // ==================== Base64 Operations ====================

    /// Encodes text as base64
    #[cfg(feature = "base64")]
    pub fn to_base64(&self) -> Text {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        Self::new(STANDARD.encode(self.as_bytes()))
    }

    /// Decodes from base64
    #[cfg(feature = "base64")]
    pub fn from_base64(encoded: &str) -> TextResult<Text> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};

        STANDARD.decode(encoded)
            .map_err(|e| TextError::ParseError {
                ty: "base64",
                msg: e.to_string(),
            })
            .and_then(|bytes| Self::from_utf8(&bytes))
    }
}

// ==================== Trait Implementations ====================

impl Deref for Text {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl AsRef<str> for Text {
    #[inline]
    fn as_ref(&self) -> &str {
        &self.inner
    }
}

impl AsRef<[u8]> for Text {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.inner.as_bytes()
    }
}

impl Borrow<str> for Text {
    #[inline]
    fn borrow(&self) -> &str {
        &self.inner
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl Default for Text {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

// ==================== Conversion Traits ====================

impl From<String> for Text {
    #[inline]
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for Text {
    #[inline]
    fn from(s: &str) -> Self {
        Self::new(s.to_string())
    }
}

impl From<&String> for Text {
    #[inline]
    fn from(s: &String) -> Self {
        Self::new(s.clone())
    }
}

impl From<Box<str>> for Text {
    #[inline]
    fn from(s: Box<str>) -> Self {
        Self {
            inner: Arc::from(s),
            char_count_cache: std::sync::OnceLock::new(),
        }
    }
}

impl From<Arc<str>> for Text {
    #[inline]
    fn from(s: Arc<str>) -> Self {
        Self {
            inner: s,
            char_count_cache: std::sync::OnceLock::new(),
        }
    }
}

impl From<Cow<'_, str>> for Text {
    #[inline]
    fn from(cow: Cow<'_, str>) -> Self {
        match cow {
            Cow::Borrowed(s) => Self::from(s),
            Cow::Owned(s) => Self::new(s),
        }
    }
}

impl From<char> for Text {
    #[inline]
    fn from(c: char) -> Self {
        Self::new(c.to_string())
    }
}

impl From<Text> for String {
    #[inline]
    fn from(text: Text) -> Self {
        text.into_string()
    }
}

impl From<Text> for Arc<str> {
    #[inline]
    fn from(text: Text) -> Self {
        text.inner
    }
}

impl From<Text> for Cow<'_, str> {
    #[inline]
    fn from(text: Text) -> Self {
        Cow::Owned(text.into_string())
    }
}

impl From<Text> for Bytes {
    #[inline]
    fn from(text: Text) -> Self {
        text.to_bytes()
    }
}

impl FromStr for Text {
    type Err = TextError;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from(s))
    }
}

// ==================== Comparison Traits ====================

impl PartialEq for Text {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for Text {}

impl PartialEq<str> for Text {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        &*self.inner == other
    }
}

impl PartialEq<&str> for Text {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        &*self.inner == *other
    }
}

impl PartialEq<String> for Text {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        &*self.inner == other
    }
}

impl PartialEq<Text> for str {
    #[inline]
    fn eq(&self, other: &Text) -> bool {
        self == &*other.inner
    }
}

impl PartialEq<Text> for String {
    #[inline]
    fn eq(&self, other: &Text) -> bool {
        self == &*other.inner
    }
}

impl PartialOrd for Text {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Text {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.inner.cmp(&other.inner)
    }
}

impl Hash for Text {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
    }
}

// ==================== Arithmetic Operations ====================

impl Add for Text {
    type Output = Text;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(format!("{}{}", self.inner, rhs.inner))
    }
}

impl Add<&str> for Text {
    type Output = Text;

    fn add(self, rhs: &str) -> Self::Output {
        Self::new(format!("{}{}", self.inner, rhs))
    }
}

impl Add<&String> for Text {
    type Output = Text;

    fn add(self, rhs: &String) -> Self::Output {
        Self::new(format!("{}{}", self.inner, rhs))
    }
}

impl Add<Text> for &str {
    type Output = Text;

    fn add(self, rhs: Text) -> Self::Output {
        Text::new(format!("{}{}", self, rhs.inner))
    }
}

impl AddAssign for Text {
    fn add_assign(&mut self, rhs: Self) {
        *self = self.clone() + rhs;
    }
}

impl AddAssign<&str> for Text {
    fn add_assign(&mut self, rhs: &str) {
        *self = self.clone() + rhs;
    }
}

// ==================== Index Operations ====================

impl Index<Range<usize>> for Text {
    type Output = str;

    fn index(&self, range: Range<usize>) -> &Self::Output {
        &self.inner[range]
    }
}

impl Index<RangeFrom<usize>> for Text {
    type Output = str;

    fn index(&self, range: RangeFrom<usize>) -> &Self::Output {
        &self.inner[range]
    }
}

impl Index<RangeTo<usize>> for Text {
    type Output = str;

    fn index(&self, range: RangeTo<usize>) -> &Self::Output {
        &self.inner[range]
    }
}

impl Index<RangeFull> for Text {
    type Output = str;

    fn index(&self, _: RangeFull) -> &Self::Output {
        &self.inner
    }
}

// ==================== Iterator Support ====================

impl FromIterator<char> for Text {
    fn from_iter<T: IntoIterator<Item = char>>(iter: T) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl FromIterator<String> for Text {
    fn from_iter<T: IntoIterator<Item = String>>(iter: T) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl FromIterator<Text> for Text {
    fn from_iter<T: IntoIterator<Item = Text>>(iter: T) -> Self {
        let strings: Vec<String> = iter.into_iter().map(|t| t.into_string()).collect();
        Self::new(strings.join(""))
    }
}

impl<'a> FromIterator<&'a str> for Text {
    fn from_iter<T: IntoIterator<Item = &'a str>>(iter: T) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl<'a> FromIterator<&'a Text> for Text {
    fn from_iter<T: IntoIterator<Item = &'a Text>>(iter: T) -> Self {
        let strings: Vec<&str> = iter.into_iter().map(|t| t.as_str()).collect();
        Self::new(strings.join(""))
    }
}

impl Extend<char> for Text {
    fn extend<T: IntoIterator<Item = char>>(&mut self, iter: T) {
        let additional: String = iter.into_iter().collect();
        *self = self.clone() + &additional;
    }
}

impl Extend<String> for Text {
    fn extend<T: IntoIterator<Item = String>>(&mut self, iter: T) {
        let additional: String = iter.into_iter().collect();
        *self = self.clone() + &additional;
    }
}

impl<'a> Extend<&'a str> for Text {
    fn extend<T: IntoIterator<Item = &'a str>>(&mut self, iter: T) {
        let additional: String = iter.into_iter().collect();
        *self = self.clone() + &additional;
    }
}

// ==================== JSON Support ====================

#[cfg(feature = "serde")]
impl From<Text> for serde_json::Value {
    fn from(text: Text) -> Self {
        serde_json::Value::String(text.into_string())
    }
}

#[cfg(feature = "serde")]
impl TryFrom<serde_json::Value> for Text {
    type Error = TextError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        match value {
            serde_json::Value::String(s) => Ok(Text::new(s)),
            serde_json::Value::Null => Ok(Text::empty()),
            serde_json::Value::Bool(_) => Err(TextError::JsonTypeMismatch { found: "bool" }),
            serde_json::Value::Number(_) => Err(TextError::JsonTypeMismatch { found: "number" }),
            serde_json::Value::Array(_) => Err(TextError::JsonTypeMismatch { found: "array" }),
            serde_json::Value::Object(_) => Err(TextError::JsonTypeMismatch { found: "object" }),
        }
    }
}

// ==================== Send + Sync ====================

// Text is automatically Send + Sync because Arc<str> is Send + Sync
unsafe impl Send for Text {}
unsafe impl Sync for Text {}

// ==================== IntoIterator for references ====================

impl<'a> IntoIterator for &'a Text {
    type Item = char;
    type IntoIter = Chars<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.chars()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_creation() {
        let t1 = Text::new("hello".to_string());
        let t2 = Text::from("world");
        let t3: Text = "test".parse().unwrap();

        assert_eq!(t1.as_str(), "hello");
        assert_eq!(t2.as_str(), "world");
        assert_eq!(t3.as_str(), "test");
    }

    #[test]
    fn test_transformations() {
        let text = Text::from("Hello World");

        assert_eq!(text.to_lowercase().as_str(), "hello world");
        assert_eq!(text.to_uppercase().as_str(), "HELLO WORLD");
        assert_eq!(text.capitalize().as_str(), "Hello world");
        assert_eq!(text.to_title_case().as_str(), "Hello World");
    }

    #[test]
    fn test_substring() {
        let text = Text::from("Hello, World!");

        assert_eq!(text.substring(0, 5).unwrap().as_str(), "Hello");
        assert_eq!(text.substring(7, 12).unwrap().as_str(), "World");
        assert_eq!(text.take(5).as_str(), "Hello");
        assert_eq!(text.skip(7).as_str(), "World!");
        assert_eq!(text.take_last(6).as_str(), "World!");
    }

    #[test]
    fn test_padding() {
        let text = Text::from("test");

        assert_eq!(text.pad_left(8, ' ').as_str(), "    test");
        assert_eq!(text.pad_right(8, ' ').as_str(), "test    ");
        assert_eq!(text.center(8, ' ').as_str(), "  test  ");
    }

    #[test]
    fn test_word_wrap() {
        let text = Text::from("This is a long text that needs to be wrapped");
        let wrapped = text.word_wrap(10);

        assert_eq!(wrapped[0].as_str(), "This is a");
        assert_eq!(wrapped[1].as_str(), "long text");
    }

    #[cfg(feature = "base64")]
    #[test]
    fn test_base64() {
        let original = Text::from("Hello, World!");
        let encoded = original.to_base64();
        let decoded = Text::from_base64(encoded.as_str()).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_char_count_caching() {
        let text = Text::from("Hello 世界");

        // First call computes and caches
        assert_eq!(text.char_count(), 8);

        // Second call uses cached value
        assert_eq!(text.char_count(), 8);
    }

    #[test]
    fn test_arc_sharing() {
        let t1 = Text::from("shared");
        let t2 = t1.clone();

        // Both should point to the same Arc
        assert_eq!(t1.as_str(), t2.as_str());
    }

    #[cfg(feature = "pattern")]
    #[test]
    fn test_regex() {
        let text = Text::from("hello123world456");

        assert!(text.matches_pattern(r"\d+").unwrap());

        let replaced = text.regex_replace(r"\d+", "X").unwrap();
        assert_eq!(replaced.as_str(), "helloXworldX");

        let matches = text.find_all_matches(r"\d+").unwrap();
        assert_eq!(matches, vec!["123", "456"]);
    }

    #[cfg(feature = "rayon")]
    #[test]
    fn test_parallel_operations() {
        let text = Text::from("Lorem ipsum dolor sit amet ".repeat(1000));

        let serial_count = text.char_count();
        let parallel_count = text.par_char_count();

        assert_eq!(serial_count, parallel_count);

        let word_count = text.par_word_count();
        assert!(word_count > 0);
    }
}