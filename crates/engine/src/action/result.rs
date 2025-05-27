use std::collections::HashMap;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::action::ActionError;
use crate::instance::{LazyInstance, ResolvableInstance};

#[derive(Debug, Clone)]
pub enum ActionResult<T> {
    /// Successfully computed value
    Value(SerializeValue),

    /// Error during computation
    Error(String),

    /// Binary data
    Binary(BinaryResult),

    /// Route to another node
    Route(RouteResult),
    
    Loop(LoopResult<T>),
    
    /// Reference to a resolvable instance
    Instance(Arc<dyn ResolvableInstance<Output = T>>),

    /// Lazily computed value
    LazyInstance(LazyResolver<T>),
}