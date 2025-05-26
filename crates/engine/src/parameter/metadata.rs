use derive_builder::Builder;
use serde::{Deserialize, Serialize};

use crate::ParameterError;
use crate::types::Key;

#[derive(Debug, Clone, Serialize, Deserialize, Builder, PartialEq)]
#[builder(
    pattern = "owned",
    setter(strip_option, into),
    build_fn(error = "ParameterError")
)]
pub struct ParameterMetadata {
    #[builder(
        setter(strip_option, into),
        field(ty = "String", build = "Key::new(self.key.clone())?")
    )]
    pub key: Key,

    pub name: String,

    pub required: bool,

    #[builder(setter(strip_option), default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[builder(setter(strip_option), default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,

    #[builder(setter(strip_option), default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl ParameterMetadata {
    pub fn builder() -> ParameterMetadataBuilder {
        ParameterMetadataBuilder::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::KeyParseError;

    #[test]
    fn test_success_with_all_required() {
        let meta = ParameterMetadata::builder()
            .key("SomeKey")
            .name("My Name")
            .required(true)
            .build()
            .expect("builder should succeed when all required fields provided");

        assert_eq!(meta.key, "somekey");
        assert_eq!(meta.name, "My Name");
        assert!(meta.required);

        assert!(meta.description.is_none());
        assert!(meta.placeholder.is_none());
        assert!(meta.hint.is_none());
    }

    #[test]
    fn test_error_missing_required_fields() {
        let err = ParameterMetadata::builder().build().unwrap_err();
        match err {
            ParameterError::InvalidKeyFormat(e) => {
                assert_eq!(e, KeyParseError::Empty);
            }
            _ => panic!("expected BuildError, got {:?}", err),
        }
    }

    #[test]
    fn test_optional_fields_are_set() {
        let meta = ParameterMetadata::builder()
            .key("SomeKey  ")
            .name("Name")
            .required(false)
            .description("Some description")
            .placeholder("Enter value")
            .hint("Helpful hint")
            .build()
            .unwrap();

        assert_eq!(meta.key, "somekey");
        assert_eq!(meta.name, "Name");
        assert!(!meta.required);

        assert_eq!(meta.description.as_deref(), Some("Some description"));
        assert_eq!(meta.placeholder.as_deref(), Some("Enter value"));
        assert_eq!(meta.hint.as_deref(), Some("Helpful hint"));
    }

    #[test]
    fn test_key_invalid() {
        let err = ParameterMetadata::builder()
            .key("  555  ")
            .name("My Name")
            .required(true)
            .build()
            .unwrap_err();

        match err {
            ParameterError::InvalidKeyFormat(_) => {}
            _ => panic!("expected BuildError, got {:?}", err),
        }
    }
}
