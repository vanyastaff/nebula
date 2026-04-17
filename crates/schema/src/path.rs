use std::fmt;

use serde::{Deserialize, Serialize};

/// Typed reference to a field key path.
///
/// Minimal stub — Task 7 does the full rewrite with segment-level typing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldPath(String);

impl FieldPath {
    /// Build a local path resolved from the current object context.
    pub fn local(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    /// Build a root-anchored empty path (schema root).
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Parse a dotted path string.
    ///
    /// Currently accepts any non-empty string; Task 7 will add stricter validation.
    pub fn parse(path: &str) -> Result<Self, crate::error::ValidationError> {
        Ok(Self(path.to_owned()))
    }

    /// Returns true when this path starts with the given prefix path.
    pub fn starts_with(&self, prefix: &Self) -> bool {
        if prefix.0.is_empty() {
            return true;
        }
        if self.0 == prefix.0 {
            return true;
        }
        self.0.starts_with(&format!("{}.", prefix.0))
    }

    /// Returns true when this path is root-anchored (empty string = root).
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Borrow path as string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FieldPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::FieldPath;

    #[test]
    fn root_is_empty() {
        let root = FieldPath::root();
        assert!(root.is_root());
    }

    #[test]
    fn local_is_not_root() {
        let local = FieldPath::local("user.email");
        assert!(!local.is_root());
    }

    #[test]
    fn starts_with_prefix() {
        let parent = FieldPath::local("user");
        let child = FieldPath::local("user.email");
        let other = FieldPath::local("other");

        assert!(child.starts_with(&parent));
        assert!(!child.starts_with(&other));
        assert!(child.starts_with(&FieldPath::root()));
    }

    #[test]
    fn parse_returns_result() {
        let p = FieldPath::parse("user.email").unwrap();
        assert_eq!(p.as_str(), "user.email");
    }

    #[test]
    fn display_shows_path() {
        let p = FieldPath::parse("x").unwrap();
        assert_eq!(format!("{p}"), "x");
    }
}
