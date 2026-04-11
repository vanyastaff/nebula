use nebula_core::AuthPattern;
use nebula_parameter::collection::ParameterCollection;
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
/// use nebula_core::AuthPattern;
/// use nebula_credential::CredentialDescription;
/// use nebula_parameter::{Parameter, ParameterCollection};
///
/// let properties = ParameterCollection::new()
///     .add(Parameter::string("client_id").label("Client ID").required())
///     .add(
///         Parameter::string("client_secret")
///             .label("Client Secret")
///             .required()
///             .secret(),
///     );
///
/// let github_oauth2 = CredentialDescription {
///     key: "github_oauth2".to_string(),
///     name: "GitHub OAuth2".to_string(),
///     description: "OAuth2 authentication for GitHub API".to_string(),
///     icon: Some("github".to_string()),
///     icon_url: None,
///     documentation_url: Some("https://docs.github.com/en/apps/oauth-apps".to_string()),
///     properties,
///     pattern: AuthPattern::OAuth2,
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Parameter definitions - what fields this credential type requires.
    pub properties: ParameterCollection,

    /// Authentication pattern classification for UI and tooling.
    pub pattern: AuthPattern,
}

impl CredentialDescription {
    /// Create a new credential description builder
    pub fn builder() -> CredentialDescriptionBuilder {
        CredentialDescriptionBuilder::default()
    }
}

/// Builder for CredentialDescription
#[derive(Debug, Default)]
pub struct CredentialDescriptionBuilder {
    key: Option<String>,
    name: Option<String>,
    description: Option<String>,
    icon: Option<String>,
    icon_url: Option<String>,
    documentation_url: Option<String>,
    properties: Option<ParameterCollection>,
    pattern: Option<AuthPattern>,
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
    pub fn properties(mut self, properties: ParameterCollection) -> Self {
        self.properties = Some(properties);
        self
    }

    /// Set the authentication pattern
    pub fn pattern(mut self, pattern: AuthPattern) -> Self {
        self.pattern = Some(pattern);
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
            pattern: self.pattern.ok_or("pattern is required")?,
        })
    }
}
