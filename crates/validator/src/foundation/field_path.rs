//! Typed field path following RFC 6901 JSON Pointer.
//!
//! [`FieldPath`] provides a validated, zero-overhead wrapper around a canonical
//! JSON Pointer string. It guarantees the path is well-formed at construction
//! time and provides typed operations for composition and segment access.

use std::{borrow::Cow, fmt};

use super::error::to_json_pointer;

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
/// let path = FieldPath::from_segments(["user", "addresses", "0", "city"]).unwrap();
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
    /// Parses a field path from any supported format.
    ///
    /// Accepts dot notation (`user.name`), bracket notation (`items[0]`),
    /// JSON Pointer (`/user/name`), or URI fragment (`#/user/name`).
    /// Returns `None` if the path is empty or invalid.
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
    /// assert!(FieldPath::parse("").is_none());
    /// ```
    #[must_use]
    pub fn parse(path: impl AsRef<str>) -> Option<Self> {
        to_json_pointer(path.as_ref()).map(|p| Self(Cow::Owned(p)))
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
    /// Returns `None` if no non-empty segments are provided.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_validator::foundation::FieldPath;
    ///
    /// let path = FieldPath::from_segments(["user", "addresses", "0"]).unwrap();
    /// assert_eq!(path.as_str(), "/user/addresses/0");
    ///
    /// assert!(FieldPath::from_segments(Vec::<&str>::new()).is_none());
    /// ```
    #[must_use]
    pub fn from_segments<I, S>(segments: I) -> Option<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut pointer = String::new();
        let mut has_segments = false;
        for segment in segments {
            let seg = segment.as_ref();
            if !seg.is_empty() {
                has_segments = true;
                pointer.push('/');
                escape_segment(seg, &mut pointer);
            }
        }
        if has_segments {
            Some(Self(Cow::Owned(pointer)))
        } else {
            None
        }
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
        self.0[1..] // skip leading '/'
            .split('/')
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
        self.0[1..].split('/').count()
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
    /// Returns `None` for single-segment paths.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_validator::foundation::FieldPath;
    ///
    /// let path = FieldPath::parse("user.addresses[0].city").unwrap();
    /// assert_eq!(path.parent().unwrap().as_str(), "/user/addresses/0");
    /// assert!(FieldPath::single("name").parent().is_none());
    /// ```
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        match self.0.rfind('/') {
            Some(0) => None, // single segment: "/name"
            Some(pos) => Some(Self(Cow::Owned(self.0[..pos].to_owned()))),
            None => None,
        }
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
        pointer.push_str(&other.0); // other.0 starts with '/'
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
        FieldPath::parse(raw.as_ref())
            .ok_or_else(|| serde::de::Error::custom(format!("invalid field path: {raw:?}")))
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
    fn parse_empty_returns_none() {
        assert!(FieldPath::parse("").is_none());
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
        let path = FieldPath::from_segments(["user", "addresses", "0", "city"]).unwrap();
        assert_eq!(path.as_str(), "/user/addresses/0/city");
    }

    #[test]
    fn from_segments_empty_returns_none() {
        assert!(FieldPath::from_segments(Vec::<&str>::new()).is_none());
        assert!(FieldPath::from_segments(["", ""]).is_none());
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
        assert!(FieldPath::single("name").parent().is_none());
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
        let path = FieldPath::from_segments(["a/b", "c~d"]).unwrap();
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
    fn deserialize_rejects_empty() {
        let result: Result<FieldPath, _> = serde_json::from_value(serde_json::json!(""));
        assert!(result.is_err());
    }

    #[test]
    fn roundtrip_stable_across_formats() {
        let p = FieldPath::parse("items[0].city").unwrap();
        let encoded = serde_json::to_value(&p).unwrap();
        let decoded: FieldPath = serde_json::from_value(encoded).unwrap();
        assert_eq!(p, decoded);
    }
}
