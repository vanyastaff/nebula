use derive_builder::Builder;
use serde::{Deserialize, Serialize};
use crate::{ParameterKey, ParameterError};


#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Builder)]
#[builder(pattern="owned", setter(strip_option, into), build_fn(error = "ParameterError"))]
pub struct ParameterMetadata {
    #[builder(
        setter(strip_option, into),
        field(ty = "String", build = "ParameterKey::new(self.key.clone())?")
    )]
    pub key: ParameterKey,
    pub name: String,
    pub description: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    pub hint: Option<String>
}


impl ParameterMetadata {
    pub fn new(key: ParameterKey, name: impl Into<String>, description: impl Into<String>) -> Self {
        ParameterMetadata {
            key,
            name: name.into(),
            description: description.into(),
            required: false,
            placeholder: None,
            hint: None
        }
    }

    pub fn builder() -> ParameterMetadataBuilder {
        ParameterMetadataBuilder::default()
    }
}