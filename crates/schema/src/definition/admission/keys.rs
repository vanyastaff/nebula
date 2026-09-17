//! Checked declaration keys and authored address roles.

use std::fmt;

use super::super::model::DefinitionKey;
use crate::{FieldKey, ValidationError};

/// A checked local declaration key used by address roles.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct DefinitionMemberKey(FieldKey);

impl DefinitionMemberKey {
    /// Parses a checked local declaration key.
    ///
    /// # Errors
    ///
    /// Returns `invalid_key` when the value is not a valid schema identifier.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_schema::DefinitionMemberKey;
    ///
    /// let key = DefinitionMemberKey::new("line_items")?;
    /// assert_eq!(key.as_str(), "line_items");
    /// # Ok::<(), nebula_schema::ValidationError>(())
    /// ```
    pub fn new(value: impl AsRef<str>) -> Result<Self, ValidationError> {
        FieldKey::new(value).map(Self)
    }

    /// Borrows the key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for DefinitionMemberKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DefinitionMemberKey(<redacted>)")
    }
}

/// An explicit authored declaration-use role.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DeclarationUse {
    /// The graph root use.
    Root,
    /// An alias body use.
    Alias,
    /// A record property use.
    Property(DefinitionMemberKey),
    /// The typed value use for undeclared keys in a record.
    AdditionalProperty,
    /// An array element use.
    Element,
    /// A unit union variant declaration.
    Variant(DefinitionMemberKey),
    /// A data union variant payload use.
    VariantPayload(DefinitionMemberKey),
}

/// A key-based address from an external companion document.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeclarationAddress {
    definition: DefinitionKey,
    use_site: DeclarationUse,
}

impl DeclarationAddress {
    /// Creates a checked key-based declaration address.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_schema::{DeclarationAddress, DeclarationUse, DefinitionKey};
    ///
    /// let address = DeclarationAddress::new(
    ///     DefinitionKey::new("root")?,
    ///     DeclarationUse::Root,
    /// );
    /// assert_eq!(address.definition().as_str(), "root");
    /// # Ok::<(), nebula_schema::ValidationError>(())
    /// ```
    #[must_use]
    pub const fn new(definition: DefinitionKey, use_site: DeclarationUse) -> Self {
        Self {
            definition,
            use_site,
        }
    }

    /// Borrows the authored definition anchor.
    #[must_use]
    pub const fn definition(&self) -> &DefinitionKey {
        &self.definition
    }

    /// Borrows the explicit use role.
    #[must_use]
    pub const fn use_site(&self) -> &DeclarationUse {
        &self.use_site
    }
}
