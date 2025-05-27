use crate::ParameterCollection;
use crate::action::error::ActionError;
use crate::types::Key;
use derive_builder::Builder;
use serde::{Deserialize, Serialize};
use crate::connection::Connections;

#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
#[builder(
    pattern = "owned",
    setter(strip_option, into),
    build_fn(error = "ActionError")
)]
pub struct ActionMetadata {
    #[builder(
        setter(strip_option, into),
        field(ty = "String", build = "Key::new(self.key.clone())?")
    )]
    pub key: Key,

    pub name: String,

    pub description: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    pub supported_auth: Option<Vec<Key>>,

    #[builder(default = "false")]
    pub require_auth: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    pub parameters: Option<ParameterCollection>,

    pub inputs: Option<Connections>,
    
    pub output: Option<Connections>,
}

impl ActionMetadata {
    pub fn builder() -> ActionMetadataBuilder {
        ActionMetadataBuilder::default()
    }
}
