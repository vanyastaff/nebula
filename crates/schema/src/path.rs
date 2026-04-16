use serde::{Deserialize, Serialize};

/// Typed reference to a field key path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldPath(String);

impl FieldPath {
    /// Build a local path resolved from the current object context.
    pub fn local(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    /// Build a root-anchored path resolved from schema root.
    pub fn root(path: impl AsRef<str>) -> Self {
        Self(format!("$root.{}", path.as_ref()))
    }

    /// Parse path without extra validation.
    pub fn parse(path: &str) -> Self {
        Self(path.to_owned())
    }

    /// Returns true when this path is root-anchored.
    pub fn is_root(&self) -> bool {
        self.0.starts_with("$root.")
    }

    /// Borrow path as string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::FieldPath;

    #[test]
    fn root_constructor_marks_root_paths() {
        let root = FieldPath::root("user.email");
        let local = FieldPath::local("user.email");

        assert!(root.is_root());
        assert!(!local.is_root());
    }
}
