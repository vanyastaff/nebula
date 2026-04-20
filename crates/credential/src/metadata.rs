use nebula_core::{AuthPattern, CredentialKey};
use nebula_metadata::{BaseMetadata, Metadata};
use nebula_schema::ValidSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error returned by [`CredentialMetadataBuilder::build`] when a required
/// field is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CredentialMetadataBuildError {
    /// `key` was never set on the builder.
    #[error("credential metadata `key` is required")]
    MissingKey,
    /// `name` was never set on the builder.
    #[error("credential metadata `name` is required")]
    MissingName,
    /// `description` was never set on the builder.
    #[error("credential metadata `description` is required")]
    MissingDescription,
    /// `schema` was never set on the builder.
    #[error("credential metadata `schema` is required")]
    MissingSchema,
    /// `pattern` was never set on the builder.
    #[error("credential metadata `pattern` is required")]
    MissingPattern,
}

/// Describes a credential type (OAuth2, API Key, Database, etc.)
///
/// Used for UI form generation, input validation, type registry, and
/// auto-generated documentation. The shared catalog prefix (`key`, `name`,
/// `description`, `schema`, `icon`, `documentation_url`, `tags`,
/// `maturity`, `deprecation`) lives on the composed [`BaseMetadata`];
/// `pattern` is the credential-specific classifier.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMetadata {
    /// Shared catalog prefix.
    #[serde(flatten)]
    pub base: BaseMetadata<CredentialKey>,
    /// Authentication pattern classification for UI and tooling.
    pub pattern: AuthPattern,
}

impl Metadata for CredentialMetadata {
    type Key = CredentialKey;
    fn base(&self) -> &BaseMetadata<CredentialKey> {
        &self.base
    }
}

impl CredentialMetadata {
    /// Create credential metadata whose schema is pulled from a
    /// [`Credential`](crate::credential::Credential) implementation's
    /// `Input` type.
    #[must_use]
    pub fn for_credential<C>(
        key: CredentialKey,
        name: impl Into<String>,
        description: impl Into<String>,
        pattern: AuthPattern,
    ) -> Self
    where
        C: crate::credential::Credential,
    {
        Self {
            base: BaseMetadata::new(
                key,
                name,
                description,
                <C::Input as nebula_schema::HasSchema>::schema(),
            ),
            pattern,
        }
    }

    /// Start building credential metadata with the given required fields.
    #[must_use]
    pub fn new(
        key: CredentialKey,
        name: impl Into<String>,
        description: impl Into<String>,
        schema: ValidSchema,
        pattern: AuthPattern,
    ) -> Self {
        Self {
            base: BaseMetadata::new(key, name, description, schema),
            pattern,
        }
    }

    /// Builder entry point.
    #[must_use]
    pub fn builder() -> CredentialMetadataBuilder {
        CredentialMetadataBuilder::default()
    }
}

/// Imperative builder for [`CredentialMetadata`] — useful when the fields
/// come from a config file or generated catalog entry rather than a
/// compile-time [`Credential`](crate::credential::Credential) impl.
#[derive(Debug, Default)]
pub struct CredentialMetadataBuilder {
    key: Option<CredentialKey>,
    name: Option<String>,
    description: Option<String>,
    schema: Option<ValidSchema>,
    pattern: Option<AuthPattern>,
    icon: Option<nebula_metadata::Icon>,
    documentation_url: Option<String>,
}

impl CredentialMetadataBuilder {
    /// Set the typed credential key.
    #[must_use]
    pub fn key(mut self, key: CredentialKey) -> Self {
        self.key = Some(key);
        self
    }

    /// Set the human-readable name.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the description.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the schema.
    #[must_use]
    pub fn schema(mut self, schema: ValidSchema) -> Self {
        self.schema = Some(schema);
        self
    }

    /// Set the authentication pattern.
    #[must_use]
    pub fn pattern(mut self, pattern: AuthPattern) -> Self {
        self.pattern = Some(pattern);
        self
    }

    /// Set an inline icon identifier.
    #[must_use]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(nebula_metadata::Icon::inline(icon));
        self
    }

    /// Set a URL-backed icon.
    #[must_use]
    pub fn icon_url(mut self, url: impl Into<String>) -> Self {
        self.icon = Some(nebula_metadata::Icon::url(url));
        self
    }

    /// Set the documentation URL.
    #[must_use]
    pub fn documentation_url(mut self, url: impl Into<String>) -> Self {
        self.documentation_url = Some(url.into());
        self
    }

    /// Finalise, returning a typed [`CredentialMetadataBuildError`] variant
    /// when a required field is missing.
    pub fn build(self) -> Result<CredentialMetadata, CredentialMetadataBuildError> {
        let mut base = BaseMetadata::new(
            self.key.ok_or(CredentialMetadataBuildError::MissingKey)?,
            self.name.ok_or(CredentialMetadataBuildError::MissingName)?,
            self.description
                .ok_or(CredentialMetadataBuildError::MissingDescription)?,
            self.schema
                .ok_or(CredentialMetadataBuildError::MissingSchema)?,
        );
        if let Some(icon) = self.icon {
            base.icon = icon;
        }
        base.documentation_url = self.documentation_url;
        Ok(CredentialMetadata {
            base,
            pattern: self
                .pattern
                .ok_or(CredentialMetadataBuildError::MissingPattern)?,
        })
    }
}

/// Compatibility validation errors for credential metadata evolution.
///
/// Wraps [`nebula_metadata::BaseCompatError`] (shared catalog-entity rules)
/// and layers the credential-specific auth-pattern rule on top.
///
/// The pattern rule lives on this type rather than in `nebula-metadata`
/// because no other catalog citizen has an auth-pattern classifier —
/// `AuthPattern` is credential-specific and changing it is semantically
/// equivalent to replacing the credential, so it requires a major bump.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MetadataCompatibilityError {
    /// A generic catalog-citizen rule fired (key / version / schema).
    #[error(transparent)]
    Base(#[from] nebula_metadata::BaseCompatError<CredentialKey>),

    /// Auth pattern changed without a major version bump.
    #[error("credential auth pattern changed without a major version bump")]
    PatternChangeWithoutMajorBump,
}

impl CredentialMetadata {
    /// Validate that this metadata update is version-compatible with `previous`.
    ///
    /// Delegates `key immutable / version monotonic / schema-break-requires-
    /// major` to [`nebula_metadata::validate_base_compat`]; layers the
    /// credential-specific auth-pattern rule on top.
    pub fn validate_compatibility(
        &self,
        previous: &Self,
    ) -> Result<(), MetadataCompatibilityError> {
        nebula_metadata::validate_base_compat(&self.base, &previous.base)?;

        if self.pattern != previous.pattern
            && self.base.version.major == previous.base.version.major
        {
            return Err(MetadataCompatibilityError::PatternChangeWithoutMajorBump);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::{AuthPattern, credential_key};
    use nebula_metadata::BaseCompatError;
    use nebula_schema::Schema;
    use semver::Version;

    use super::{CredentialMetadata, MetadataCompatibilityError};

    fn empty_schema() -> nebula_schema::ValidSchema {
        Schema::builder().build().unwrap()
    }

    fn cred(pattern: AuthPattern, major: u64, minor: u64) -> CredentialMetadata {
        let mut m =
            CredentialMetadata::new(credential_key!("cred"), "C", "d", empty_schema(), pattern);
        m.base.version = Version::new(major, minor, 0);
        m
    }

    #[test]
    fn pattern_change_requires_major_bump() {
        let prev = cred(AuthPattern::SecretToken, 1, 0);
        let next = cred(AuthPattern::OAuth2, 1, 1);
        let err = next.validate_compatibility(&prev).unwrap_err();
        assert_eq!(
            err,
            MetadataCompatibilityError::PatternChangeWithoutMajorBump
        );
    }

    #[test]
    fn pattern_change_with_major_accepted() {
        let prev = cred(AuthPattern::SecretToken, 1, 0);
        let next = cred(AuthPattern::OAuth2, 2, 0);
        assert!(next.validate_compatibility(&prev).is_ok());
    }

    #[test]
    fn key_change_via_base_rejected() {
        let prev = CredentialMetadata::new(
            credential_key!("a"),
            "A",
            "d",
            empty_schema(),
            AuthPattern::SecretToken,
        );
        let next = CredentialMetadata::new(
            credential_key!("b"),
            "A",
            "d",
            empty_schema(),
            AuthPattern::SecretToken,
        );
        let err = next.validate_compatibility(&prev).unwrap_err();
        assert_eq!(
            err,
            MetadataCompatibilityError::Base(BaseCompatError::KeyChanged {
                previous: credential_key!("a"),
                current: credential_key!("b"),
            })
        );
    }
}
