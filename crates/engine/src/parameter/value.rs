use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::parameter::types::group::GroupValue;
use crate::parameter::types::mode::ModeValue;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum ParameterValue {
    Value(Value),
    Mode(ModeValue),
    Group(GroupValue),
    Expression(String),
    Expirable(Value),
}
