//! Node metadata and builder.

use nebula_core::NodeKey;
use nebula_credential::CredentialDescription;
use nebula_parameter::collection::ParameterCollection;
use serde::{Deserialize, Serialize};

use crate::NodeError;

/// Static metadata describing a node type.
///
/// Built via the builder API:
///
/// ```
/// use nebula_node::NodeMetadata;
///
/// let meta = NodeMetadata::builder("http_request", "HTTP Request")
///     .description("Make HTTP calls to external APIs")
///     .group(vec!["network".into()])
///     .version(2)
///     .build()
///     .unwrap();
///
/// assert_eq!(meta.key().as_str(), "http_request");
/// assert_eq!(meta.version(), 2);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetadata {
    key: NodeKey,
    name: String,
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    group: Vec<String>,
    #[serde(default)]
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    documentation_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<ParameterCollection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    credentials: Vec<CredentialDescription>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    action_keys: Vec<String>,
}

fn default_version() -> u32 {
    1
}

impl NodeMetadata {
    /// Start building metadata with the minimum required fields.
    pub fn builder(key: impl AsRef<str>, name: impl Into<String>) -> NodeMetadataBuilder {
        NodeMetadataBuilder {
            key: key.as_ref().to_owned(),
            name: name.into(),
            version: 1,
            group: Vec::new(),
            description: String::new(),
            icon: None,
            icon_url: None,
            documentation_url: None,
            parameters: None,
            credentials: Vec::new(),
            action_keys: Vec::new(),
        }
    }

    /// The normalized key.
    #[inline]
    pub fn key(&self) -> &NodeKey {
        &self.key
    }

    /// Human-readable name.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Version number (1-based).
    #[inline]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Group hierarchy for UI categorization.
    #[inline]
    pub fn group(&self) -> &[String] {
        &self.group
    }

    /// Short description.
    #[inline]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Optional icon identifier.
    #[inline]
    pub fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    /// Optional icon URL.
    #[inline]
    pub fn icon_url(&self) -> Option<&str> {
        self.icon_url.as_deref()
    }

    /// Optional documentation URL.
    #[inline]
    pub fn documentation_url(&self) -> Option<&str> {
        self.documentation_url.as_deref()
    }

    /// User-facing parameter definitions, if any.
    #[inline]
    pub fn parameters(&self) -> Option<&ParameterCollection> {
        self.parameters.as_ref()
    }

    /// Credential descriptions required by this node.
    #[inline]
    pub fn credentials(&self) -> &[CredentialDescription] {
        &self.credentials
    }

    /// Action keys this node exposes.
    #[inline]
    pub fn action_keys(&self) -> &[String] {
        &self.action_keys
    }
}

/// Builder for [`NodeMetadata`].
pub struct NodeMetadataBuilder {
    key: String,
    name: String,
    version: u32,
    group: Vec<String>,
    description: String,
    icon: Option<String>,
    icon_url: Option<String>,
    documentation_url: Option<String>,
    parameters: Option<ParameterCollection>,
    credentials: Vec<CredentialDescription>,
    action_keys: Vec<String>,
}

impl NodeMetadataBuilder {
    /// Set the version number (defaults to 1).
    pub fn version(mut self, version: u32) -> Self {
        self.version = version;
        self
    }

    /// Set the group hierarchy.
    pub fn group(mut self, group: Vec<String>) -> Self {
        self.group = group;
        self
    }

    /// Set the description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set an icon identifier.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set an icon URL.
    pub fn icon_url(mut self, url: impl Into<String>) -> Self {
        self.icon_url = Some(url.into());
        self
    }

    /// Set a documentation URL.
    pub fn documentation_url(mut self, url: impl Into<String>) -> Self {
        self.documentation_url = Some(url.into());
        self
    }

    /// Set user-facing parameter definitions.
    pub fn parameters(mut self, params: ParameterCollection) -> Self {
        self.parameters = Some(params);
        self
    }

    /// Add a credential description.
    pub fn credential(mut self, cred: CredentialDescription) -> Self {
        self.credentials.push(cred);
        self
    }

    /// Set all credential descriptions at once.
    pub fn credentials(mut self, creds: Vec<CredentialDescription>) -> Self {
        self.credentials = creds;
        self
    }

    /// Add an action key this node exposes.
    pub fn action_key(mut self, key: impl Into<String>) -> Self {
        self.action_keys.push(key.into());
        self
    }

    /// Set all action keys at once.
    pub fn action_keys(mut self, keys: Vec<String>) -> Self {
        self.action_keys = keys;
        self
    }

    /// Validate and build the metadata.
    pub fn build(self) -> Result<NodeMetadata, NodeError> {
        let key: NodeKey = self.key.parse().map_err(NodeError::InvalidKey)?;

        Ok(NodeMetadata {
            key,
            name: self.name,
            version: self.version,
            group: self.group,
            description: self.description,
            icon: self.icon,
            icon_url: self.icon_url,
            documentation_url: self.documentation_url,
            parameters: self.parameters,
            credentials: self.credentials,
            action_keys: self.action_keys,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_minimal() {
        let meta = NodeMetadata::builder("slack", "Slack").build().unwrap();
        assert_eq!(meta.key().as_str(), "slack");
        assert_eq!(meta.name(), "Slack");
        assert_eq!(meta.version(), 1);
        assert!(meta.group().is_empty());
        assert!(meta.description().is_empty());
    }

    #[test]
    fn builder_full() {
        let meta = NodeMetadata::builder("http_request", "HTTP Request")
            .version(2)
            .group(vec!["network".into(), "api".into()])
            .description("Make HTTP calls")
            .icon("globe")
            .icon_url("https://example.com/icon.png")
            .documentation_url("https://docs.example.com/http")
            .action_key("http.get")
            .action_key("http.post")
            .build()
            .unwrap();

        assert_eq!(meta.version(), 2);
        assert_eq!(meta.group(), &["network", "api"]);
        assert_eq!(meta.icon(), Some("globe"));
        assert_eq!(meta.icon_url(), Some("https://example.com/icon.png"));
        assert_eq!(meta.action_keys(), &["http.get", "http.post"]);
    }

    #[test]
    fn builder_normalizes_key() {
        let meta = NodeMetadata::builder("HTTP Request", "HTTP Request")
            .build()
            .unwrap();
        assert_eq!(meta.key().as_str(), "http_request");
    }

    #[test]
    fn builder_rejects_invalid_key() {
        let result = NodeMetadata::builder("", "Empty").build();
        assert!(result.is_err());
    }

    #[test]
    fn serde_roundtrip() {
        let meta = NodeMetadata::builder("slack", "Slack")
            .version(3)
            .description("Send messages")
            .build()
            .unwrap();

        let json = serde_json::to_string(&meta).unwrap();
        let back: NodeMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(back.key().as_str(), "slack");
        assert_eq!(back.version(), 3);
        assert_eq!(back.description(), "Send messages");
    }
}
