//! Catalog icon — exactly one valid representation at a time.

use crate::{
    MetadataError,
    bounded::{self, SHARED_BYTES},
};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{Error as _, MapAccess, Visitor},
};

/// Icon for a catalog entity (action, credential, resource, …).
///
/// Replaces the earlier `icon: Option<String> + icon_url: Option<String>`
/// pair, which allowed invalid combinations (both set, inconsistent).
///
/// Serialized untagged so the wire format stays compact:
/// - [`Icon::None`] → omitted when the field uses `skip_serializing_if`; otherwise serializes as
///   `null`.
/// - [`Icon::Inline`] → a bare string, e.g. `"github"` or `"🔑"`.
/// - [`Icon::Url`] → a `{ "url": "https://..." }` object.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Icon {
    /// No icon declared.
    #[default]
    None,
    /// Inline identifier — an icon name understood by the UI catalog, a
    /// Material icon id, or a raw emoji glyph.
    Inline(String),
    /// Absolute or root-relative URL pointing to a custom icon asset.
    Url {
        /// Icon URL (absolute or root-relative).
        url: String,
    },
}

impl<'de> Deserialize<'de> for Icon {
    #[tracing::instrument(name = "metadata.deserialize_icon", skip_all)]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct IconVisitor;
        impl<'de> Visitor<'de> for IconVisitor {
            type Value = Icon;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a bounded metadata icon")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Icon, E> {
                Ok(Icon::None)
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Icon, E> {
                bounded::check_bytes(value, SHARED_BYTES, crate::MetadataField::Icon)
                    .map_err(E::custom)?;
                Ok(Icon::inline(value))
            }
            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Icon, E> {
                bounded::check_bytes(&value, SHARED_BYTES, crate::MetadataField::Icon)
                    .map_err(E::custom)?;
                Ok(Icon::inline(value))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Icon, A::Error> {
                #[derive(Deserialize)]
                struct Text(
                    #[serde(deserialize_with = "bounded::string::<_, SHARED_BYTES>")] String,
                );
                let Some(Text(key)) = map.next_key()? else {
                    return Err(A::Error::custom(MetadataError::InvalidWire));
                };
                if key != "url" {
                    return Err(A::Error::custom(MetadataError::InvalidWire));
                }
                let Text(url) = map.next_value()?;
                if map.next_key::<Text>()?.is_some() {
                    return Err(A::Error::custom(MetadataError::InvalidWire));
                }
                Ok(Icon::url(url))
            }
        }
        deserializer
            .deserialize_any(IconVisitor)
            .map_err(|_| D::Error::custom(MetadataError::InvalidWire))
    }
}

impl Icon {
    /// Build an inline icon (e.g. `"github"`, `"🔑"`).
    #[must_use]
    pub fn inline(name: impl Into<String>) -> Self {
        Self::Inline(name.into())
    }

    /// Build a URL-backed icon.
    #[must_use]
    pub fn url(url: impl Into<String>) -> Self {
        Self::Url { url: url.into() }
    }

    /// Return the inline icon name, if this variant is [`Icon::Inline`].
    #[must_use]
    pub fn as_inline(&self) -> Option<&str> {
        match self {
            Icon::Inline(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Return the icon URL, if this variant is [`Icon::Url`].
    #[must_use]
    pub fn as_url(&self) -> Option<&str> {
        match self {
            Icon::Url { url } => Some(url.as_str()),
            _ => None,
        }
    }

    /// Returns `true` iff this is [`Icon::None`].
    #[must_use]
    pub fn is_none(&self) -> bool {
        matches!(self, Icon::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_roundtrip() {
        let i = Icon::inline("github");
        let s = serde_json::to_string(&i).unwrap();
        assert_eq!(s, r#""github""#);
        let back: Icon = serde_json::from_str(&s).unwrap();
        assert_eq!(back, i);
    }

    #[test]
    fn url_roundtrip() {
        let i = Icon::url("https://example.com/icon.svg");
        let s = serde_json::to_string(&i).unwrap();
        assert_eq!(s, r#"{"url":"https://example.com/icon.svg"}"#);
        let back: Icon = serde_json::from_str(&s).unwrap();
        assert_eq!(back, i);
    }

    #[test]
    fn accessors_match_variant() {
        let inline = Icon::inline("x");
        assert_eq!(inline.as_inline(), Some("x"));
        assert_eq!(inline.as_url(), None);

        let url = Icon::url("/a");
        assert_eq!(url.as_url(), Some("/a"));
        assert_eq!(url.as_inline(), None);

        assert!(Icon::None.is_none());
    }
}
