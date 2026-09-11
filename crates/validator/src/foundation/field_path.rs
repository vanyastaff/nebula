//! Typed field path following RFC 6901 JSON Pointer.
//!
//! [`FieldPath`] provides a validated, zero-overhead wrapper around a canonical
//! JSON Pointer string. It guarantees the path is well-formed at construction
//! time and provides typed operations for composition and segment access.

use std::{borrow::Cow, fmt};

use super::error::to_json_pointer;

/// Invalid RFC 6901 JSON Pointer syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FieldPathError {
    /// A non-root pointer must start with a slash.
    #[error("a non-root JSON Pointer must start with '/'")]
    MissingLeadingSlash,
    /// A tilde must be followed by `0` or `1`.
    #[error("invalid JSON Pointer escape at byte {byte_offset}: expected '~0' or '~1'")]
    InvalidEscape {
        /// Byte offset of the invalid tilde escape.
        byte_offset: usize,
    },
}

/// A validated field path following RFC 6901 JSON Pointer.
///
/// Stores a canonical JSON Pointer string and provides typed operations
/// for path construction, composition, and segment access.
///
/// # Memory Layout
///
/// Same as `Cow<'static, str>` (24 bytes on 64-bit) — zero-overhead newtype.
///
/// # Examples
///
/// ```
/// use nebula_validator::foundation::FieldPath;
///
/// // From dot notation
/// let path = FieldPath::parse("user.name").unwrap();
/// assert_eq!(path.as_str(), "/user/name");
///
/// // From segments
/// let path = FieldPath::from_segments(["user", "addresses", "0", "city"]);
/// assert_eq!(path.as_str(), "/user/addresses/0/city");
///
/// // Composition
/// let parent = FieldPath::single("user");
/// let child = parent.push("email");
/// assert_eq!(child.as_str(), "/user/email");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FieldPath(Cow<'static, str>);

impl FieldPath {
    /// The root JSON Pointer, represented by the empty string.
    #[must_use]
    pub const fn root() -> Self {
        Self(Cow::Borrowed(""))
    }

    /// Whether this pointer identifies the whole document.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Constructs a pointer without interpreting dot/bracket notation or URI fragments.
    ///
    /// Preserves all segments, including empty keys and whitespace. The empty
    /// string identifies the root; `/` identifies its empty-string property.
    ///
    /// # Errors
    /// Returns [`FieldPathError`] for a missing leading slash or a tilde escape
    /// other than `~0` or `~1`, as required by [RFC 6901](https://www.rfc-editor.org/rfc/rfc6901#section-3).
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_validator::foundation::FieldPath;
    /// assert!(FieldPath::from_pointer("").unwrap().is_root());
    /// assert_eq!(FieldPath::from_pointer("/").unwrap().depth(), 1);
    /// assert!(FieldPath::from_pointer("user.name").is_err());
    /// ```
    #[tracing::instrument(level = "trace", skip(pointer), fields(pointer_len = pointer.len()))]
    pub fn from_pointer(pointer: &str) -> Result<Self, FieldPathError> {
        if pointer.is_empty() {
            return Ok(Self::root());
        }
        if !pointer.starts_with('/') {
            return Err(FieldPathError::MissingLeadingSlash);
        }
        let mut bytes = pointer.bytes().enumerate();
        while let Some((byte_offset, byte)) = bytes.next() {
            if byte == b'~' && !matches!(bytes.next(), Some((_, b'0' | b'1'))) {
                return Err(FieldPathError::InvalidEscape { byte_offset });
            }
        }
        Ok(Self(Cow::Owned(pointer.to_owned())))
    }

    /// Parses a field path from any supported format.
    ///
    /// Accepts dot notation (`user.name`), bracket notation (`items[0]`),
    /// JSON Pointer (`/user/name`), or URI fragment (`#/user/name`).
    /// The empty string identifies the root. Returns `None` for invalid paths.
    /// Use [`Self::from_pointer`] for strict wire-format parsing.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_validator::foundation::FieldPath;
    ///
    /// assert_eq!(
    ///     FieldPath::parse("user.name").unwrap().as_str(),
    ///     "/user/name"
    /// );
    /// assert_eq!(FieldPath::parse("items[0]").unwrap().as_str(), "/items/0");
    /// assert_eq!(
    ///     FieldPath::parse("/already/pointer").unwrap().as_str(),
    ///     "/already/pointer"
    /// );
    /// assert!(FieldPath::parse("").unwrap().is_root());
    /// ```
    #[must_use]
    pub fn parse(path: impl AsRef<str>) -> Option<Self> {
        let path = path.as_ref();
        if path.is_empty() || path.starts_with('/') {
            return Self::from_pointer(path).ok();
        }
        to_json_pointer(path).and_then(|pointer| Self::from_pointer(&pointer).ok())
    }

    /// Creates a single-segment field path.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_validator::foundation::FieldPath;
    ///
    /// let path = FieldPath::single("email");
    /// assert_eq!(path.as_str(), "/email");
    /// ```
    #[must_use]
    pub fn single(segment: impl AsRef<str>) -> Self {
        let segment = segment.as_ref();
        let mut pointer = String::with_capacity(1 + segment.len());
        pointer.push('/');
        escape_segment(segment, &mut pointer);
        Self(Cow::Owned(pointer))
    }

    /// Creates a `FieldPath` from an iterator of segments.
    ///
    /// Preserves every segment, including empty keys. No segments means root.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_validator::foundation::FieldPath;
    ///
    /// let path = FieldPath::from_segments(["user", "addresses", "0"]);
    /// assert_eq!(path.as_str(), "/user/addresses/0");
    ///
    /// assert!(FieldPath::from_segments(Vec::<&str>::new()).is_root());
    /// assert_eq!(FieldPath::from_segments([""]).as_str(), "/");
    /// ```
    #[must_use]
    pub fn from_segments<I, S>(segments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut pointer = String::new();
        for segment in segments {
            pointer.push('/');
            escape_segment(segment.as_ref(), &mut pointer);
        }
        Self(Cow::Owned(pointer))
    }

    /// Returns the canonical JSON Pointer string.
    #[must_use]
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns an iterator over the unescaped path segments.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_validator::foundation::FieldPath;
    ///
    /// let path = FieldPath::parse("user.addresses[0].city").unwrap();
    /// let segments: Vec<_> = path.segments().collect();
    /// assert_eq!(segments, ["user", "addresses", "0", "city"]);
    /// ```
    pub fn segments(&self) -> impl Iterator<Item = Cow<'_, str>> {
        self.0
            .strip_prefix('/')
            .into_iter()
            .flat_map(|path| path.split('/'))
            .map(|segment| {
                if segment.contains('~') {
                    Cow::Owned(segment.replace("~1", "/").replace("~0", "~"))
                } else {
                    Cow::Borrowed(segment)
                }
            })
    }

    /// Returns the number of segments in the path.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_validator::foundation::FieldPath;
    ///
    /// assert_eq!(FieldPath::single("name").depth(), 1);
    /// assert_eq!(FieldPath::parse("user.name").unwrap().depth(), 2);
    /// ```
    #[must_use]
    pub fn depth(&self) -> usize {
        self.0.bytes().filter(|byte| *byte == b'/').count()
    }

    /// Returns the last segment of the path.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_validator::foundation::FieldPath;
    ///
    /// let path = FieldPath::parse("user.email").unwrap();
    /// assert_eq!(path.last_segment().unwrap(), "email");
    /// ```
    #[must_use]
    pub fn last_segment(&self) -> Option<Cow<'_, str>> {
        self.segments().last()
    }

    /// Returns the parent path (all segments except the last).
    ///
    /// Returns `None` only for root. A single-segment path has root as its parent.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_validator::foundation::FieldPath;
    ///
    /// let path = FieldPath::parse("user.addresses[0].city").unwrap();
    /// assert_eq!(path.parent().unwrap().as_str(), "/user/addresses/0");
    /// assert_eq!(FieldPath::single("name").parent(), Some(FieldPath::root()));
    /// ```
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        match self.0.rfind('/') {
            Some(0) => Some(Self::root()),
            Some(pos) => Some(Self(Cow::Owned(self.0[..pos].to_owned()))),
            None => None,
        }
    }

    /// Whether this path is the prefix itself or one of its descendants.
    ///
    /// Compares whole segments: `/user` is not a prefix of `/username`.
    /// Root is a prefix of every path.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_validator::foundation::FieldPath;
    /// let user = FieldPath::single("user");
    /// assert!(user.push("name").starts_with(&user));
    /// assert!(!FieldPath::single("username").starts_with(&user));
    /// assert!(user.starts_with(&FieldPath::root()));
    /// ```
    #[must_use]
    pub fn starts_with(&self, prefix: &Self) -> bool {
        self.0
            .strip_prefix(prefix.as_str())
            .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('/'))
    }

    /// Creates a new path by appending a segment.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_validator::foundation::FieldPath;
    ///
    /// let path = FieldPath::single("user").push("email");
    /// assert_eq!(path.as_str(), "/user/email");
    /// ```
    #[must_use]
    pub fn push(&self, segment: impl AsRef<str>) -> Self {
        let segment = segment.as_ref();
        let mut pointer = String::with_capacity(self.0.len() + 1 + segment.len());
        pointer.push_str(&self.0);
        pointer.push('/');
        escape_segment(segment, &mut pointer);
        Self(Cow::Owned(pointer))
    }

    /// Creates a new path by appending all segments from another path.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_validator::foundation::FieldPath;
    ///
    /// let base = FieldPath::single("user");
    /// let nested = FieldPath::parse("addresses[0].city").unwrap();
    /// let full = base.append(&nested);
    /// assert_eq!(full.as_str(), "/user/addresses/0/city");
    /// ```
    #[must_use]
    pub fn append(&self, other: &FieldPath) -> Self {
        let mut pointer = String::with_capacity(self.0.len() + other.0.len());
        pointer.push_str(&self.0);
        pointer.push_str(&other.0); // A non-root pointer starts with '/'.
        Self(Cow::Owned(pointer))
    }

    /// Converts into the inner `Cow<'static, str>`.
    #[must_use]
    #[inline]
    pub fn into_inner(self) -> Cow<'static, str> {
        self.0
    }
}

impl fmt::Display for FieldPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl serde::Serialize for FieldPath {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for FieldPath {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = <Cow<'de, str> as serde::Deserialize>::deserialize(d)?;
        FieldPath::from_pointer(raw.as_ref()).map_err(serde::de::Error::custom)
    }
}

impl From<FieldPath> for Cow<'static, str> {
    fn from(path: FieldPath) -> Self {
        path.0
    }
}

impl From<FieldPath> for String {
    fn from(path: FieldPath) -> Self {
        path.0.into_owned()
    }
}

/// Escapes a segment according to RFC 6901.
fn escape_segment(segment: &str, out: &mut String) {
    for ch in segment.chars() {
        match ch {
            '~' => out.push_str("~0"),
            '/' => out.push_str("~1"),
            _ => out.push(ch),
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dot_notation() {
        let path = FieldPath::parse("user.name").unwrap();
        assert_eq!(path.as_str(), "/user/name");
    }

    #[test]
    fn parse_bracket_notation() {
        let path = FieldPath::parse("items[0].name").unwrap();
        assert_eq!(path.as_str(), "/items/0/name");
    }

    #[test]
    fn parse_json_pointer() {
        let path = FieldPath::parse("/already/pointer").unwrap();
        assert_eq!(path.as_str(), "/already/pointer");
    }

    #[test]
    fn parse_uri_fragment() {
        let path = FieldPath::parse("#/user/email").unwrap();
        assert_eq!(path.as_str(), "/user/email");
    }

    #[test]
    fn parse_empty_returns_root() {
        assert_eq!(FieldPath::parse(""), Some(FieldPath::root()));
        assert!(FieldPath::parse("  ").is_none());
    }

    #[test]
    fn single_segment() {
        let path = FieldPath::single("email");
        assert_eq!(path.as_str(), "/email");
        assert_eq!(path.depth(), 1);
    }

    #[test]
    fn from_segments_basic() {
        let path = FieldPath::from_segments(["user", "addresses", "0", "city"]);
        assert_eq!(path.as_str(), "/user/addresses/0/city");
    }

    #[test]
    fn from_segments_preserves_empty_keys_and_root() {
        assert!(FieldPath::from_segments(Vec::<&str>::new()).is_root());
        assert_eq!(FieldPath::from_segments(["", ""]).as_str(), "//");
    }

    #[test]
    fn segments_roundtrip() {
        let path = FieldPath::parse("user.addresses[0].city").unwrap();
        let segments: Vec<_> = path.segments().collect();
        assert_eq!(segments, ["user", "addresses", "0", "city"]);
    }

    #[test]
    fn depth() {
        assert_eq!(FieldPath::single("x").depth(), 1);
        assert_eq!(FieldPath::parse("a.b.c").unwrap().depth(), 3);
    }

    #[test]
    fn last_segment() {
        let path = FieldPath::parse("user.email").unwrap();
        assert_eq!(path.last_segment().unwrap(), "email");
    }

    #[test]
    fn parent() {
        let path = FieldPath::parse("user.addresses[0].city").unwrap();
        let parent = path.parent().unwrap();
        assert_eq!(parent.as_str(), "/user/addresses/0");
        assert_eq!(FieldPath::single("name").parent(), Some(FieldPath::root()));
    }

    #[test]
    fn push_segment() {
        let path = FieldPath::single("user").push("email");
        assert_eq!(path.as_str(), "/user/email");
    }

    #[test]
    fn append_paths() {
        let base = FieldPath::single("user");
        let nested = FieldPath::parse("addresses[0].city").unwrap();
        let full = base.append(&nested);
        assert_eq!(full.as_str(), "/user/addresses/0/city");
    }

    #[test]
    fn display_trait() {
        let path = FieldPath::parse("user.name").unwrap();
        assert_eq!(format!("{path}"), "/user/name");
    }

    #[test]
    fn into_cow() {
        let path = FieldPath::single("email");
        let cow: Cow<'static, str> = path.into();
        assert_eq!(cow.as_ref(), "/email");
    }

    #[test]
    fn escape_special_chars() {
        let path = FieldPath::from_segments(["a/b", "c~d"]);
        assert_eq!(path.as_str(), "/a~1b/c~0d");
        let segments: Vec<_> = path.segments().collect();
        assert_eq!(segments, ["a/b", "c~d"]);
    }

    #[test]
    fn serialize_is_plain_string() {
        let p = FieldPath::parse("user.email").unwrap();
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json, serde_json::json!("/user/email"));
    }

    #[test]
    fn deserialize_from_string() {
        let p: FieldPath = serde_json::from_value(serde_json::json!("/user/email")).unwrap();
        assert_eq!(p.as_str(), "/user/email");
    }

    #[test]
    fn deserialize_accepts_root() {
        let result: Result<FieldPath, _> = serde_json::from_value(serde_json::json!(""));
        assert_eq!(result.unwrap(), FieldPath::root());
    }

    #[test]
    fn roundtrip_stable_across_formats() {
        let p = FieldPath::parse("items[0].city").unwrap();
        let encoded = serde_json::to_value(&p).unwrap();
        let decoded: FieldPath = serde_json::from_value(encoded).unwrap();
        assert_eq!(p, decoded);
    }
}
