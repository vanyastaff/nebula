//! Typed catalog guidance that grants neither authority nor dependencies.

use std::fmt::{self, Write as _};

use nebula_core::{ActionKey, CredentialKey, PluginKey, ResourceKey};
use semver::VersionReq;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _, ser::Error as _};

use crate::bounded::{self, SHARED_BYTES};
use crate::catalog_error::{
    CatalogValueError, deserialize_catalog_object, deserialize_catalog_string,
};

// Match the pinned semver parser's ceiling before formatting native intent.
const MAX_REQUIREMENT_COMPARATORS: usize = 32;

/// A typed reference to another catalog definition, optionally constrained by
/// a target interface or bundle version requirement.
///
/// The referenced family may differ from the source family. References are
/// guidance only: they do not replace keys, prove assignability, create bindings,
/// or declare plugin dependencies. Missing targets remain unresolved guidance.
///
/// Native [`VersionReq`] values are authoring intent: their public comparator
/// fields can represent combinations that do not survive the wire grammar.
/// [`Self::validate`] and serialization check lossless representation within
/// catalog limits, including values constructed directly through enum variants.
/// Shared metadata admission must validate this intent before accepting it.
///
/// The wire form is a closed object with `kind`, `key`, and an optional
/// `version_requirement`, for example
/// `{"kind":"action","key":"http.request","version_requirement":"^2"}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CatalogReference {
    /// Guidance pointing to an action definition.
    Action {
        /// The action's typed catalog key.
        key: ActionKey,
        /// An optional requirement on the target interface version.
        version_requirement: Option<VersionReq>,
    },
    /// Guidance pointing to a credential definition.
    Credential {
        /// The credential's typed catalog key.
        key: CredentialKey,
        /// An optional requirement on the target interface version.
        version_requirement: Option<VersionReq>,
    },
    /// Guidance pointing to a resource definition.
    Resource {
        /// The resource's typed catalog key.
        key: ResourceKey,
        /// An optional requirement on the target interface version.
        version_requirement: Option<VersionReq>,
    },
    /// Guidance pointing to a plugin bundle.
    Plugin {
        /// The plugin's typed catalog key.
        key: PluginKey,
        /// An optional requirement on the target bundle version.
        version_requirement: Option<VersionReq>,
    },
}

impl CatalogReference {
    /// Reference an action without constraining its target version.
    #[must_use]
    pub const fn action(key: ActionKey) -> Self {
        Self::Action {
            key,
            version_requirement: None,
        }
    }

    /// Reference a credential without constraining its target version.
    #[must_use]
    pub const fn credential(key: CredentialKey) -> Self {
        Self::Credential {
            key,
            version_requirement: None,
        }
    }

    /// Reference a resource without constraining its target version.
    #[must_use]
    pub const fn resource(key: ResourceKey) -> Self {
        Self::Resource {
            key,
            version_requirement: None,
        }
    }

    /// Reference a plugin without constraining its target bundle version.
    #[must_use]
    pub const fn plugin(key: PluginKey) -> Self {
        Self::Plugin {
            key,
            version_requirement: None,
        }
    }

    /// Set or replace the requirement on the target version.
    ///
    /// The native value remains authoring intent until [`Self::validate`],
    /// serialization, or shared metadata admission checks its representability.
    ///
    /// ```
    /// use nebula_core::ResourceKey;
    /// use nebula_metadata::CatalogReference;
    /// let reference = CatalogReference::resource(ResourceKey::new("postgres.client")?)
    ///     .with_version_requirement("^2".parse()?);
    /// assert_eq!(reference.key(), "postgres.client");
    /// assert_eq!(reference.version_requirement(), Some(&"^2".parse()?));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn with_version_requirement(mut self, requirement: VersionReq) -> Self {
        match &mut self {
            Self::Action {
                version_requirement,
                ..
            }
            | Self::Credential {
                version_requirement,
                ..
            }
            | Self::Resource {
                version_requirement,
                ..
            }
            | Self::Plugin {
                version_requirement,
                ..
            } => {
                *version_requirement = Some(requirement);
            },
        }
        self
    }

    /// Borrow the referenced key's canonical text; the variant retains its family.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::Action { key, .. } => key.as_str(),
            Self::Credential { key, .. } => key.as_str(),
            Self::Resource { key, .. } => key.as_str(),
            Self::Plugin { key, .. } => key.as_str(),
        }
    }

    /// Borrow the optional requirement on the target version.
    #[must_use]
    pub const fn version_requirement(&self) -> Option<&VersionReq> {
        match self {
            Self::Action {
                version_requirement,
                ..
            }
            | Self::Credential {
                version_requirement,
                ..
            }
            | Self::Resource {
                version_requirement,
                ..
            }
            | Self::Plugin {
                version_requirement,
                ..
            } => version_requirement.as_ref(),
        }
    }

    /// Check that the native reference intent has a lossless catalog wire form.
    ///
    /// Valid requirements contain at most 32 comparators, fit within the shared
    /// authored-field byte ceiling when formatted, and parse back to exactly
    /// the same typed requirement. This does not resolve or authorize a target.
    ///
    /// # Errors
    /// Returns [`CatalogValueError::InvalidReferenceVersionRequirement`] when
    /// formatting, parsing, or exact typed comparison rejects the requirement.
    /// No submitted requirement text or parser error is retained.
    ///
    /// ```
    /// use nebula_metadata::CatalogReference;
    /// let reference = CatalogReference::action("http.request".parse()?)
    ///     .with_version_requirement("^2".parse()?);
    /// reference.validate()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[tracing::instrument(name = "metadata.validate_catalog_reference", skip_all, err)]
    pub fn validate(&self) -> Result<(), CatalogValueError> {
        self.version_requirement()
            .map(checked_requirement_text)
            .transpose()
            .map(|_| ())
    }
}

impl Serialize for CatalogReference {
    #[tracing::instrument(name = "metadata.serialize_catalog_reference", skip_all)]
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Fields<'a> {
            kind: &'static str,
            key: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            version_requirement: Option<&'a str>,
        }

        // Share validation with validate(), retaining its checked text so the
        // serializer cannot format unvalidated native intent or write a prefix.
        let requirement = self
            .version_requirement()
            .map(checked_requirement_text)
            .transpose()
            .map_err(S::Error::custom)?;
        let kind = match self {
            Self::Action { .. } => "action",
            Self::Credential { .. } => "credential",
            Self::Resource { .. } => "resource",
            Self::Plugin { .. } => "plugin",
        };
        Fields {
            kind,
            key: self.key(),
            version_requirement: requirement.as_deref(),
        }
        .serialize(serializer)
    }
}

struct RequirementText(String);

impl fmt::Write for RequirementText {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self
            .0
            .len()
            .checked_add(text.len())
            .is_none_or(|length| length > SHARED_BYTES)
        {
            return Err(fmt::Error);
        }
        self.0.push_str(text);
        Ok(())
    }
}

/// Check native requirement intent and return its bounded, lossless wire text.
/// Shared by catalog references and plugin dependency admission/serialization.
#[tracing::instrument(name = "metadata.check_version_requirement", skip_all, err)]
pub(crate) fn checked_requirement_text(
    requirement: &VersionReq,
) -> Result<String, CatalogValueError> {
    if requirement.comparators.len() > MAX_REQUIREMENT_COMPARATORS {
        return Err(CatalogValueError::InvalidReferenceVersionRequirement);
    }
    let mut text = RequirementText(String::new());
    write!(text, "{requirement}")
        .map_err(|_| CatalogValueError::InvalidReferenceVersionRequirement)?;
    let reparsed: VersionReq = text
        .0
        .parse()
        .map_err(|_| CatalogValueError::InvalidReferenceVersionRequirement)?;
    // Native Comparator fields can be omitted or change operator when formatted.
    // Successful parsing alone does not establish preservation of that intent.
    if &reparsed != requirement {
        return Err(CatalogValueError::InvalidReferenceVersionRequirement);
    }
    Ok(text.0)
}

enum ReferenceKind {
    Action,
    Credential,
    Resource,
    Plugin,
}

impl<'de> Deserialize<'de> for ReferenceKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_catalog_string(deserializer, CatalogValueError::InvalidReference, |kind| {
            match kind {
                "action" => Ok(Self::Action),
                "credential" => Ok(Self::Credential),
                "resource" => Ok(Self::Resource),
                "plugin" => Ok(Self::Plugin),
                _ => Err(CatalogValueError::InvalidReference),
            }
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReferenceFields {
    kind: ReferenceKind,
    key: String,
    #[serde(
        default,
        deserialize_with = "bounded::optional_string::<_, SHARED_BYTES>"
    )]
    version_requirement: Option<String>,
}

impl<'de> Deserialize<'de> for CatalogReference {
    #[tracing::instrument(name = "metadata.deserialize_catalog_reference", skip_all)]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let fields = deserialize_catalog_object(deserializer, CatalogValueError::InvalidReference)?;
        parse_reference(fields).map_err(D::Error::custom)
    }
}

#[tracing::instrument(name = "metadata.parse_catalog_reference", skip_all, err)]
fn parse_reference(fields: ReferenceFields) -> Result<CatalogReference, CatalogValueError> {
    // Parse each core key explicitly so ingress is checked and owned JSON/readers
    // are supported regardless of the core key's own serde implementation.
    let reference = match fields.kind {
        ReferenceKind::Action => CatalogReference::action(
            fields
                .key
                .parse()
                .map_err(|_| CatalogValueError::InvalidActionReferenceKey)?,
        ),
        ReferenceKind::Credential => CatalogReference::credential(
            fields
                .key
                .parse()
                .map_err(|_| CatalogValueError::InvalidCredentialReferenceKey)?,
        ),
        ReferenceKind::Resource => CatalogReference::resource(
            fields
                .key
                .parse()
                .map_err(|_| CatalogValueError::InvalidResourceReferenceKey)?,
        ),
        ReferenceKind::Plugin => CatalogReference::plugin(
            fields
                .key
                .parse()
                .map_err(|_| CatalogValueError::InvalidPluginReferenceKey)?,
        ),
    };
    let reference = match fields.version_requirement {
        Some(requirement) => reference.with_version_requirement(
            requirement
                .parse()
                .map_err(|_| CatalogValueError::InvalidReferenceVersionRequirement)?,
        ),
        None => reference,
    };
    reference.validate()?;
    Ok(reference)
}
