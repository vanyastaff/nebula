use derive_builder::Builder;
use serde::{Deserialize, Serialize};

use crate::node::error::NodeError;
use crate::types::Key;

#[derive(Debug, Clone, Serialize, Deserialize, Builder, PartialEq)]
#[builder(
    pattern = "owned",
    setter(strip_option, into),
    build_fn(error = "NodeError")
)]
pub struct NodeMetadata {
    #[builder(
        setter(strip_option, into),
        field(ty = "String", build = "Key::new(self.key.clone())?")
    )]
    pub key: Key,

    pub name: String,

    pub version: u32,

    pub group: Vec<String>,

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
}

impl NodeMetadata {
    pub fn builder() -> NodeMetadataBuilder {
        NodeMetadataBuilder::default()
    }
}
