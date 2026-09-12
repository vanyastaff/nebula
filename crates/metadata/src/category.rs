//! Checked, stable catalog category identifiers.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize};

use crate::catalog_error::{CatalogValueError, deserialize_catalog_string};

const MAX_CATEGORY_KEY_BYTES: usize = 96;

/// A lowercase ASCII category key, ordered lexically by its canonical text.
///
/// Dot-separated segments contain letters and digits, optionally separated by
/// single underscores or hyphens. Leading, trailing, or adjacent separators are
/// rejected, as are uppercase text and keys longer than 96 bytes. Parsing never
/// trims or folds case.
///
/// ```
/// use nebula_metadata::CatalogCategoryKey;
/// let category: CatalogCategoryKey = "database.relational".parse()?;
/// assert_eq!(category.as_str(), "database.relational");
/// # Ok::<(), nebula_metadata::CatalogValueError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CatalogCategoryKey(String);

impl CatalogCategoryKey {
    /// Borrow the checked category identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[tracing::instrument(name = "metadata.validate_category_key", skip_all, err)]
fn validate_category_key(key: &str) -> Result<(), CatalogValueError> {
    if key.is_empty() {
        return Err(CatalogValueError::EmptyCategoryKey);
    }
    if key.len() > MAX_CATEGORY_KEY_BYTES {
        return Err(CatalogValueError::CategoryKeyTooLong);
    }
    // Reuse the core key alphabet and separator checks, then enforce the stricter
    // lowercase, nonempty segment grammar required for catalog filter keys.
    if !domain_key::is_valid_key_default(key, MAX_CATEGORY_KEY_BYTES)
        || key.bytes().any(|byte| byte.is_ascii_uppercase())
        || key.split(['.', '_', '-']).any(str::is_empty)
    {
        return Err(CatalogValueError::InvalidCategoryKey);
    }
    Ok(())
}

impl TryFrom<&str> for CatalogCategoryKey {
    type Error = CatalogValueError;

    fn try_from(key: &str) -> Result<Self, Self::Error> {
        validate_category_key(key)?;
        Ok(Self(key.to_owned()))
    }
}

impl TryFrom<String> for CatalogCategoryKey {
    type Error = CatalogValueError;

    fn try_from(key: String) -> Result<Self, Self::Error> {
        validate_category_key(&key)?;
        Ok(Self(key))
    }
}

impl FromStr for CatalogCategoryKey {
    type Err = CatalogValueError;

    fn from_str(key: &str) -> Result<Self, Self::Err> {
        Self::try_from(key)
    }
}

impl fmt::Display for CatalogCategoryKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CatalogCategoryKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_catalog_string(
            deserializer,
            CatalogValueError::InvalidCategoryKey,
            Self::from_str,
        )
    }
}
