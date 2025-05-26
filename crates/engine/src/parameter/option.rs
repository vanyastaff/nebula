use derive_builder::Builder;
use serde::{Deserialize, Serialize};

use crate::ParameterError;
use crate::types::Key;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Builder)]
#[builder(
    pattern = "owned",
    setter(strip_option, into),
    build_fn(error = "ParameterError")
)]
pub struct ParameterOption {
    #[builder(
        setter(strip_option, into),
        field(ty = "String", build = "Key::new(self.key)?")
    )]
    pub key: Key,

    pub name: String,

    pub value: ParameterOptionValue,

    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,

    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,

    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,

    #[builder(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum ParameterOptionValue {
    String(String),
    Number(f64),
    Boolean(bool),
}

impl From<&str> for ParameterOptionValue {
    fn from(value: &str) -> Self {
        ParameterOptionValue::String(value.to_string())
    }
}

impl From<String> for ParameterOptionValue {
    fn from(value: String) -> Self {
        ParameterOptionValue::String(value)
    }
}

impl From<f64> for ParameterOptionValue {
    fn from(value: f64) -> Self {
        ParameterOptionValue::Number(value)
    }
}

impl From<i32> for ParameterOptionValue {
    fn from(value: i32) -> Self {
        ParameterOptionValue::Number(value as f64)
    }
}

impl From<i64> for ParameterOptionValue {
    fn from(value: i64) -> Self {
        ParameterOptionValue::Number(value as f64)
    }
}

impl From<bool> for ParameterOptionValue {
    fn from(value: bool) -> Self {
        ParameterOptionValue::Boolean(value)
    }
}
