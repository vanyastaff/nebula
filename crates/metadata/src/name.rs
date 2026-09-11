//! Checked catalog display names, including compile-time literal construction.

use std::{borrow::Cow, fmt};

use nebula_core::{ActionKey, CredentialKey, ResourceKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

use crate::{MetadataError, definition::validate_name};

/// A nonblank catalog display name. Whitespace surrounding text is preserved.
///
/// Use [`metadata_name!`](crate::metadata_name) for literals or [`TryFrom`] for
/// dynamic text. Existing typed catalog keys also supply valid default names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MetadataName(Cow<'static, str>);

/// Compile-time name proof used by [`metadata_name!`](crate::metadata_name).
///
/// Unlike an owned name, this proof has no destructor and can be matched in a
/// constant expression on stable Rust. Its field remains private.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct MetadataNameLiteral(&'static str);

impl MetadataNameLiteral {
    /// Check a literal, returning an error for blank text.
    pub const fn new(name: &'static str) -> Result<Self, MetadataError> {
        if is_blank(name) {
            Err(MetadataError::BlankName)
        } else {
            Ok(Self(name))
        }
    }

    /// Turn the proof into a name without further validation or allocation.
    pub const fn into_name(self) -> MetadataName {
        MetadataName(Cow::Borrowed(self.0))
    }
}

impl MetadataName {
    /// Check static text without allocating.
    ///
    /// # Errors
    /// Returns [`MetadataError::BlankName`] for empty or whitespace-only text.
    pub const fn from_static(name: &'static str) -> Result<Self, MetadataError> {
        if is_blank(name) {
            Err(MetadataError::BlankName)
        } else {
            Ok(Self(Cow::Borrowed(name)))
        }
    }

    /// Borrow the original display text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for MetadataName {
    type Error = MetadataError;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        validate_name(&name)?;
        Ok(Self(Cow::Owned(name)))
    }
}

impl TryFrom<&str> for MetadataName {
    type Error = MetadataError;

    fn try_from(name: &str) -> Result<Self, Self::Error> {
        validate_name(name)?;
        Ok(Self(Cow::Owned(name.to_owned())))
    }
}

impl From<MetadataName> for String {
    fn from(name: MetadataName) -> Self {
        name.0.into_owned()
    }
}

macro_rules! name_from_key {
    ($($key:ty),+ $(,)?) => {$ (
        impl From<$key> for MetadataName {
            fn from(key: $key) -> Self {
                // Typed catalog keys require nonblank ASCII identifiers.
                Self(Cow::Owned(key.to_string()))
            }
        }
    )+};
}

name_from_key!(ActionKey, CredentialKey, ResourceKey);

impl fmt::Display for MetadataName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for MetadataName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MetadataName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::try_from(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// Construct a checked catalog display name from a string literal.
///
/// Invalid literals fail at compile time, including Unicode-only whitespace.
///
/// ```
/// let name = nebula_metadata::metadata_name!("HTTP Request");
/// assert_eq!(name.as_str(), "HTTP Request");
/// ```
///
/// ```compile_fail
/// let name = nebula_metadata::metadata_name!(" \t\u{3000}");
/// ```
#[macro_export]
macro_rules! metadata_name {
    ($name:expr) => {
        const {
            match $crate::MetadataNameLiteral::new($name) {
                Ok(name) => name.into_name(),
                Err(_) => panic!("metadata name must contain non-whitespace text"),
            }
        }
    };
}

// str::trim is not const. Match only UTF-8 encodings of Unicode White_Space;
// exhaustive scalar and arbitrary-string tests compare this with std's oracle.
pub(crate) const fn is_blank(name: &str) -> bool {
    let bytes = name.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        let remaining = bytes.len() - offset;
        let width = match bytes[offset] {
            b' ' | b'\t'..=b'\r' => 1,
            0xc2 if remaining >= 2 && matches!(bytes[offset + 1], 0x85 | 0xa0) => 2,
            0xe1 if remaining >= 3 && bytes[offset + 1] == 0x9a && bytes[offset + 2] == 0x80 => 3,
            0xe2 if remaining >= 3
                && ((bytes[offset + 1] == 0x80
                    && matches!(bytes[offset + 2], 0x80..=0x8a | 0xa8 | 0xa9 | 0xaf))
                    || (bytes[offset + 1] == 0x81 && bytes[offset + 2] == 0x9f)) =>
            {
                3
            },
            0xe3 if remaining >= 3 && bytes[offset + 1] == 0x80 && bytes[offset + 2] == 0x80 => 3,
            _ => return false,
        };
        offset += width;
    }
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn const_whitespace_check_matches_std_for_every_unicode_scalar() {
        for scalar in (0..=0x0010_ffff).filter_map(char::from_u32) {
            let mut buffer = [0; 4];
            assert_eq!(
                super::is_blank(scalar.encode_utf8(&mut buffer)),
                scalar.is_whitespace(),
                "scalar U+{:04X}",
                u32::from(scalar)
            );
        }
    }
}
