//! Static metadata for resource types (display name, description, icon, tags).
//!
//! Similar to [`nebula_action::ActionMetadata`]: used for discovery, UI labels,
//! and monitoring. Implement [`Resource::metadata`](crate::resource::Resource) to
//! provide rich metadata; a default builds from `id()` only.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Static metadata describing a resource type.
///
/// Used for UI (resources page name/type), discovery, and categorization.
/// Provide via [`Resource::metadata`](crate::resource::Resource); default uses `id()`.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ResourceMetadata {
    /// Unique key (same as `Resource::id()`, e.g. `"postgres"`, `"redis"`).
    pub key: String,
    /// Human-readable display name (e.g. `"PostgreSQL"`, `"Redis Cache"`).
    pub name: String,
    /// Short description of what this resource provides.
    pub description: String,
    /// Optional logical icon identifier for UI (e.g. `"postgres"`, `"telegram"`).
    ///
    /// The frontend is responsible for resolving this identifier to an actual
    /// icon asset (SVG, PNG, etc.).
    #[cfg_attr(feature = "serde", serde(default))]
    pub icon: Option<String>,
    /// Optional direct icon URL when an identifier is not sufficient.
    ///
    /// This is most useful for third-party or dynamically loaded resources
    /// that provide their own icon URLs.
    #[cfg_attr(feature = "serde", serde(default))]
    pub icon_url: Option<String>,
    /// Free-form tags for discovery and grouping.
    ///
    /// Recommended conventions:
    /// - `category:database`, `category:messaging`, `category:bot`
    /// - `protocol:http`, `protocol:websocket`
    /// - `service:postgres`, `service:telegram`
    #[cfg_attr(feature = "serde", serde(default))]
    pub tags: Vec<String>,
}

impl ResourceMetadata {
    /// Create metadata with the minimum required fields.
    ///
    /// `name` defaults to `key` if you want id and display name to match.
    pub fn new(
        key: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            description: description.into(),
            icon: None,
            icon_url: None,
            tags: Vec::new(),
        }
    }

    /// Build metadata from only the key (name and description set from key).
    pub fn from_key(key: impl Into<String>) -> Self {
        let key = key.into();
        Self::new(key.clone(), key, String::new())
    }

    /// Set the optional icon identifier for UI.
    #[must_use]
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the optional icon URL for UI.
    #[must_use]
    pub fn with_icon_url(mut self, icon_url: impl Into<String>) -> Self {
        self.icon_url = Some(icon_url.into());
        self
    }

    /// Add a single tag.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Extend tags with an iterator.
    #[must_use]
    pub fn with_tags<T, I>(mut self, tags: I) -> Self
    where
        T: Into<String>,
        I: IntoIterator<Item = T>,
    {
        self.tags.extend(tags.into_iter().map(Into::into));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_new() {
        let m = ResourceMetadata::new("postgres", "PostgreSQL", "Primary database");
        assert_eq!(m.key, "postgres");
        assert_eq!(m.name, "PostgreSQL");
        assert_eq!(m.description, "Primary database");
        assert!(m.icon.is_none());
        assert!(m.icon_url.is_none());
        assert!(m.tags.is_empty());
    }

    #[test]
    fn metadata_from_key() {
        let m = ResourceMetadata::from_key("redis");
        assert_eq!(m.key, "redis");
        assert_eq!(m.name, "redis");
        assert!(m.description.is_empty());
    }
}
