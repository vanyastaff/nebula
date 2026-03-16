//! Static metadata for resource types (display name, description, icon, tags).
//!
//! Similar to `nebula_action::ActionMetadata`: used for discovery, UI labels,
//! and monitoring. Implement [`Resource::metadata`](crate::resource::Resource) to
//! provide rich metadata; a default builds from `id()` only.

use nebula_core::ResourceKey;
use serde::{Deserialize, Serialize};

/// Static metadata describing a resource type.
///
/// Used for UI (resources page name/type) and discovery.
/// Provide via [`Resource::metadata`](crate::resource::Resource); default uses the key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceMetadata {
    /// Unique key for this resource type (same as `Resource::id()`).
    ///
    /// This is a domain key (e.g. `"postgres"`, `"redis"`) with the
    /// `"resource"` domain baked in via [`ResourceKey`]. It is the
    /// canonical identifier used across manager, events, and errors.
    pub key: ResourceKey,
    /// Human-readable display name (e.g. `"PostgreSQL"`, `"Redis Cache"`).
    pub name: String,
    /// Short description of what this resource provides.
    pub description: String,
    /// Optional logical icon identifier for UI (e.g. `"postgres"`, `"telegram"`).
    ///
    /// The frontend is responsible for resolving this identifier to an actual
    /// icon asset (SVG, PNG, etc.).
    #[serde(default)]
    pub icon: Option<String>,
    /// Optional direct icon URL when an identifier is not sufficient.
    ///
    /// This is most useful for third-party or dynamically loaded resources
    /// that provide their own icon URLs.
    #[serde(default)]
    pub icon_url: Option<String>,
    /// Free-form tags for discovery and grouping.
    ///
    /// Recommended conventions:
    /// - `category:database`, `category:messaging`, `category:bot`
    /// - `protocol:http`, `protocol:websocket`
    /// - `service:postgres`, `service:telegram`
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Builder for [`ResourceMetadata`].
#[derive(Debug)]
pub struct ResourceMetadataBuilder {
    key: ResourceKey,
    name: String,
    description: String,
    icon: Option<String>,
    icon_url: Option<String>,
    tags: Vec<String>,
}

impl ResourceMetadataBuilder {
    /// Set the optional icon identifier for UI.
    #[must_use]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the optional icon URL for UI.
    #[must_use]
    pub fn icon_url(mut self, icon_url: impl Into<String>) -> Self {
        self.icon_url = Some(icon_url.into());
        self
    }

    /// Add a single tag.
    #[must_use]
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Extend tags with an iterator.
    #[must_use]
    pub fn tags<T, I>(mut self, tags: I) -> Self
    where
        T: Into<String>,
        I: IntoIterator<Item = T>,
    {
        self.tags.extend(tags.into_iter().map(Into::into));
        self
    }

    /// Build the metadata.
    #[must_use]
    pub fn build(self) -> ResourceMetadata {
        ResourceMetadata {
            key: self.key,
            name: self.name,
            description: self.description,
            icon: self.icon,
            icon_url: self.icon_url,
            tags: self.tags,
        }
    }
}

impl ResourceMetadata {
    /// Start a builder with required key, name, and description.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// ResourceMetadata::builder(key, "PostgreSQL", "Primary database")
    ///     .icon("postgres")
    ///     .tag("category:database")
    ///     .build()
    /// ```
    #[must_use]
    pub fn builder(
        key: ResourceKey,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> ResourceMetadataBuilder {
        ResourceMetadataBuilder {
            key,
            name: name.into(),
            description: description.into(),
            icon: None,
            icon_url: None,
            tags: Vec::new(),
        }
    }

    /// Create metadata with the minimum required fields.
    ///
    /// `name` defaults to `key` if you want id and display name to match.
    pub fn new(key: ResourceKey, name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            key,
            name: name.into(),
            description: description.into(),
            icon: None,
            icon_url: None,
            tags: Vec::new(),
        }
    }

    /// Build metadata from only the key (name and description set from key).
    pub fn from_key(key: ResourceKey) -> Self {
        let name = key.to_string();
        Self::new(key, name, String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::resource_key;

    #[test]
    fn metadata_new() {
        let key = resource_key!("postgres");
        let m = ResourceMetadata::new(key.clone(), "PostgreSQL", "Primary database");
        assert_eq!(m.key, key);
        assert_eq!(m.name, "PostgreSQL");
        assert_eq!(m.description, "Primary database");
        assert!(m.icon.is_none());
        assert!(m.icon_url.is_none());
        assert!(m.tags.is_empty());
    }

    #[test]
    fn metadata_from_key() {
        let key = resource_key!("redis");
        let m = ResourceMetadata::from_key(key.clone());
        assert_eq!(m.key, key);
        assert_eq!(m.name, "redis");
        assert!(m.description.is_empty());
    }

    #[test]
    fn metadata_build_with_icon_and_tags() {
        let key = resource_key!("http.client");
        let m = ResourceMetadata::builder(key.clone(), "HTTP Client", "REST API client")
            .icon("http")
            .tag("protocol:http")
            .build();
        assert_eq!(m.key, key);
        assert_eq!(m.name, "HTTP Client");
        assert_eq!(m.icon.as_deref(), Some("http"));
        assert!(m.tags.contains(&"protocol:http".to_string()));
    }
}
