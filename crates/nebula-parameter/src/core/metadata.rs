use bon::Builder;
use serde::{Deserialize, Serialize};
use crate::{ParameterKey, ParameterError};


#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Builder)]
pub struct ParameterMetadata {
    pub key: ParameterKey,
    pub name: String,
    pub description: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    pub hint: Option<String>
}