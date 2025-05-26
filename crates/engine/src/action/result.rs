use std::collections::HashMap;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::action::ActionError;
use crate::instance::{LazyInstance, ResolvableInstance};

#[derive(Debug, Clone)]
pub enum ActionResult<T> {
    /// Successfully computed value
    Value(T: Send + Sync + Clone + Serialize + for<'de> Deserialize<'de> ),

    /// Error during computation
    Error(String),

    /// Binary data
    Binary(Vec<u8>),

    /// Route to another node
    Route { target_id: String},

    /// Reference to a resolvable instance
    Instance(Arc<dyn ResolvableInstance<Output = T>>),

    /// Lazily computed value
    LazyInstance(LazyResolver<T>),
}