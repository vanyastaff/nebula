//! Typed reference to a parameter within a schema.

use serde::{Deserialize, Serialize};

/// Typed reference to a parameter within a schema.
///
/// Supports sibling references (`"field_name"`), nested paths (`"obj.field"`),
/// and absolute root references (`"$root.field"`).
///
/// # Examples
///
/// ```
/// use nebula_parameter::path::ParameterPath;
///
/// let sibling = ParameterPath::sibling("email");
/// assert!(!sibling.is_absolute());
///
/// let root = ParameterPath::root("auth_mode");
/// assert!(root.is_absolute());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ParameterPath(String);

impl ParameterPath {
    /// Reference a sibling parameter in the same scope.
    #[must_use]
    pub fn sibling(id: &str) -> Self {
        Self(id.to_owned())
    }

    /// Reference a nested parameter via dot-separated path.
    #[must_use]
    pub fn nested(path: &str) -> Self {
        Self(path.to_owned())
    }

    /// Absolute reference from the root collection.
    #[must_use]
    pub fn root(id: &str) -> Self {
        Self(format!("$root.{id}"))
    }

    /// Whether this is an absolute root reference.
    #[must_use]
    pub fn is_absolute(&self) -> bool {
        self.0.starts_with("$root.")
    }

    /// Split the path into segments.
    #[must_use]
    pub fn segments(&self) -> Vec<&str> {
        let s = self.0.strip_prefix("$root.").unwrap_or(&self.0);
        s.split('.').collect()
    }

    /// The raw path string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ParameterPath {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl std::fmt::Display for ParameterPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
