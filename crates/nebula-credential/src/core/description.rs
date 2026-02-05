use paramdef::Schema;
use serde::{Deserialize, Serialize};

/// Describes a credential type (OAuth2, API Key, Database, etc.)
///
/// This is the static schema that defines what fields a credential type requires.
/// Used for:
/// - UI form generation
/// - Validation of user input
/// - Type registry
/// - Auto-generated documentation
///
/// # Example
///
/// ```
/// use nebula_credential::core::CredentialDescription;
/// use paramdef::Schema;
///
/// let github_oauth2 = CredentialDescription {
///     key: "github_oauth2".to_string(),
///     name: "GitHub OAuth2".to_string(),
///     description: "OAuth2 authentication for GitHub API".to_string(),
///     icon: Some("github".to_string()),
///     icon_url: None,
///     documentation_url: Some("https://docs.github.com/en/apps/oauth-apps".to_string()),
///     properties: Schema::default(),
/// };
/// ```
#[derive(Debug, Clone)]
pub struct CredentialDescription {
    /// Unique identifier for this credential type (e.g., "github_oauth2", "postgres_db")
    pub key: String,

    /// Human-readable name (e.g., "GitHub OAuth2", "PostgreSQL Database")
    pub name: String,

    /// Description of what this credential is used for
    pub description: String,

    /// Optional icon identifier (e.g., "github", "database")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// Optional icon URL for custom icons
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,

    /// Optional documentation URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,

    /// Parameter definitions - what fields this credential type requires
    ///
    /// Uses paramdef::Schema for type-safe parameter definitions.
    ///
    /// Example for GitHub OAuth2:
    /// - client_id: String (required)
    /// - client_secret: SecretString (required, sensitive)
    /// - scopes: Array<String> (optional)
    pub properties: Schema,
}

impl CredentialDescription {
    /// Create a new credential description builder
    pub fn builder() -> CredentialDescriptionBuilder {
        CredentialDescriptionBuilder::default()
    }
}

/// Builder for CredentialDescription
#[derive(Default)]
pub struct CredentialDescriptionBuilder {
    key: Option<String>,
    name: Option<String>,
    description: Option<String>,
    icon: Option<String>,
    icon_url: Option<String>,
    documentation_url: Option<String>,
    properties: Option<Schema>,
}

impl CredentialDescriptionBuilder {
    /// Set the unique identifier for this credential type
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the human-readable name
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the description
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the icon identifier
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the icon URL
    pub fn icon_url(mut self, icon_url: impl Into<String>) -> Self {
        self.icon_url = Some(icon_url.into());
        self
    }

    /// Set the documentation URL
    pub fn documentation_url(mut self, documentation_url: impl Into<String>) -> Self {
        self.documentation_url = Some(documentation_url.into());
        self
    }

    /// Set the parameter schema
    pub fn properties(mut self, properties: Schema) -> Self {
        self.properties = Some(properties);
        self
    }

    /// Build the CredentialDescription
    ///
    /// # Errors
    ///
    /// Returns an error if required fields (key, name, description, properties) are not set
    pub fn build(self) -> Result<CredentialDescription, String> {
        Ok(CredentialDescription {
            key: self.key.ok_or("key is required")?,
            name: self.name.ok_or("name is required")?,
            description: self.description.ok_or("description is required")?,
            icon: self.icon,
            icon_url: self.icon_url,
            documentation_url: self.documentation_url,
            properties: self.properties.ok_or("properties is required")?,
        })
    }
}
