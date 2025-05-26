use derive_builder::Builder;
use serde::{Deserialize, Serialize};

use crate::ParameterCollection;
use crate::credential::CredentialError;
use crate::types::Key;

#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
#[builder(
    pattern = "owned",
    setter(strip_option, into),
    build_fn(error = "CredentialError")
)]
pub struct CredentialMetadata {
    #[builder(
        setter(strip_option, into),
        field(ty = "String", build = "Key::new(self.key.clone())?")
    )]
    pub key: Key,

    pub name: String,

    pub description: String,

    #[builder(setter(strip_option), default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    #[builder(setter(strip_option), default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,

    #[builder(setter(strip_option), default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,

    #[builder(setter(strip_option), default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<ParameterCollection>,
}

impl CredentialMetadata {
    pub fn builder() -> CredentialMetadataBuilder {
        CredentialMetadataBuilder::default()
    }
}
