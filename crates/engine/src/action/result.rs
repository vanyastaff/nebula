use std::collections::HashMap;
use std::sync::Arc;
use std::task::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::action::ActionError;
use crate::instance::{LazyInstance, ResolvableInstance};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult<T = Value> {
    /// The actual output/data from the action
    pub output: ActionOutput<T>,

    /// Optional metadata about execution, timing, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, Value>>,
}

pub enum ActionOutput<T> {
    Single(T),
    Collection(Vec<T>),
}

