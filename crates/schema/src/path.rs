//! Typed field-path with dot/array-index notation.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize};
use smallvec::{SmallVec, smallvec};

use crate::{error::ValidationError, key::FieldKey};

/// One segment of a field path.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathSegment {
    /// Object key.
    Key(FieldKey),
    /// List index.
    Index(usize),
}

impl From<FieldKey> for PathSegment {
    fn from(k: FieldKey) -> Self {
        Self::Key(k)
    }
}

impl From<usize> for PathSegment {
    fn from(i: usize) -> Self {
        Self::Index(i)
    }
}

impl fmt::Display for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(k) => f.write_str(k.as_str()),
            Self::Index(i) => write!(f, "[{i}]"),
        }
    }
}

/// Typed reference to a location in a `FieldValues` tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FieldPath(SmallVec<[PathSegment; 4]>);

impl FieldPath {
    /// Empty (root) path.
    pub fn root() -> Self {
        Self(SmallVec::new())
    }

    /// Parse a dotted/bracketed string (e.g. `a.b[0].c`).
    #[allow(clippy::result_large_err)]
    pub fn parse(s: &str) -> Result<Self, ValidationError> {
        if s.is_empty() {
            return Err(Self::err(s, "empty path"));
        }
        let mut segments: SmallVec<[PathSegment; 4]> = smallvec![];
        let mut rest = s;
        let mut first = true;

        while !rest.is_empty() {
            if first {
                first = false;
            } else if let Some(after) = rest.strip_prefix('.') {
                if after.is_empty() || after.starts_with('.') || after.starts_with('[') {
                    return Err(Self::err(s, "invalid separator usage"));
                }
                rest = after;
            }

            let end = rest.find(['.', '[']).unwrap_or(rest.len());
            if end == 0 {
                return Err(Self::err(s, "missing key"));
            }
            let key_lit = &rest[..end];
            let key = FieldKey::new(key_lit).map_err(|_| Self::err(s, "invalid key in path"))?;
            segments.push(PathSegment::Key(key));
            rest = &rest[end..];

            while let Some(after_open) = rest.strip_prefix('[') {
                let close = after_open
                    .find(']')
                    .ok_or_else(|| Self::err(s, "unclosed bracket"))?;
                let digits = &after_open[..close];
                if digits.is_empty() {
                    return Err(Self::err(s, "empty index"));
                }
                let idx: usize = digits
                    .parse()
                    .map_err(|_| Self::err(s, "non-numeric index"))?;
                segments.push(PathSegment::Index(idx));
                rest = &after_open[close + 1..];
            }
        }

        Ok(Self(segments))
    }

    /// Returns all path segments.
    pub fn segments(&self) -> &[PathSegment] {
        &self.0
    }

    /// Returns true when this path is root (empty).
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Append a segment, returning the new path.
    pub fn join(mut self, seg: impl Into<PathSegment>) -> Self {
        self.0.push(seg.into());
        self
    }

    /// Returns the parent path, or `None` for root.
    pub fn parent(&self) -> Option<Self> {
        if self.0.is_empty() {
            None
        } else {
            let mut copy = self.clone();
            copy.0.pop();
            Some(copy)
        }
    }

    /// Returns true when this path starts with the given prefix.
    pub fn starts_with(&self, prefix: &FieldPath) -> bool {
        self.0.len() >= prefix.0.len() && self.0[..prefix.0.len()] == prefix.0[..]
    }

    fn err(value: &str, msg: &'static str) -> ValidationError {
        ValidationError::new("invalid_path")
            .at(FieldPath::root())
            .message(msg)
            .param("path", value.to_owned())
            .build()
    }
}

impl Default for FieldPath {
    fn default() -> Self {
        Self::root()
    }
}

impl fmt::Display for FieldPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, seg) in self.0.iter().enumerate() {
            match seg {
                PathSegment::Key(_) if i > 0 => write!(f, ".{seg}")?,
                _ => write!(f, "{seg}")?,
            }
        }
        Ok(())
    }
}

impl FromStr for FieldPath {
    type Err = ValidationError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for FieldPath {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for FieldPath {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_dot_path() {
        let p = FieldPath::parse("user.email").unwrap();
        assert_eq!(p.to_string(), "user.email");
        assert_eq!(p.segments().len(), 2);
    }

    #[test]
    fn parses_array_index() {
        let p = FieldPath::parse("tags[3]").unwrap();
        assert_eq!(p.segments().len(), 2);
        match &p.segments()[1] {
            PathSegment::Index(i) => assert_eq!(*i, 3),
            _ => panic!("expected Index"),
        }
    }

    #[test]
    fn parses_nested() {
        let p = FieldPath::parse("a.b[0].c").unwrap();
        assert_eq!(p.segments().len(), 4);
        assert_eq!(p.to_string(), "a.b[0].c");
    }

    #[test]
    fn rejects_invalid_syntax() {
        for bad in ["", ".", "a.", ".a", "a[", "a[]", "a[x]", "a..b"] {
            assert!(FieldPath::parse(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn starts_with_works() {
        let a = FieldPath::parse("user.email").unwrap();
        let root = FieldPath::parse("user").unwrap();
        assert!(a.starts_with(&root));
        assert!(!root.starts_with(&a));
        assert!(a.starts_with(&a));
    }

    #[test]
    fn join_appends_segment() {
        let a = FieldPath::parse("user").unwrap();
        let b = a
            .clone()
            .join(PathSegment::Key(FieldKey::new("email").unwrap()));
        assert_eq!(b.to_string(), "user.email");
        let c = b.clone().join(PathSegment::Index(0));
        assert_eq!(c.to_string(), "user.email[0]");
    }

    #[test]
    fn parent_drops_last_segment() {
        let p = FieldPath::parse("a.b.c").unwrap();
        assert_eq!(p.parent().unwrap().to_string(), "a.b");
        assert!(FieldPath::root().parent().is_none());
    }
}
