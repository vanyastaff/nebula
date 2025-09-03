//! Context management for structured logging

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Thread-local context for structured logging
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Context {
    /// Request ID
    pub request_id: Option<String>,
    /// User ID
    pub user_id: Option<String>,
    /// Session ID
    pub session_id: Option<String>,
    /// Additional fields
    #[serde(flatten)]
    pub fields: HashMap<String, serde_json::Value>,
}

impl Context {
    /// Create a new empty context
    pub fn new() -> Self {
        Self::default()
    }

    /// Set request ID
    pub fn with_request_id(mut self, id: impl Into<String>) -> Self {
        self.request_id = Some(id.into());
        self
    }

    /// Set user ID
    pub fn with_user_id(mut self, id: impl Into<String>) -> Self {
        self.user_id = Some(id.into());
        self
    }

    /// Add a field
    pub fn with_field(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        if let Ok(v) = serde_json::to_value(value) {
            self.fields.insert(key.into(), v);
        }
        self
    }

    /// Set as current context
    pub fn set_current(self) -> ContextGuard {
        CONTEXT.with(|ctx| {
            let old = ctx.write().clone();
            *ctx.write() = self;
            ContextGuard { old: Some(old) }
        })
    }

    /// Get current context
    pub fn current() -> Self {
        CONTEXT.with(|ctx| ctx.read().clone())
    }

    /// Run a closure with this context
    pub fn scope<R>(self, f: impl FnOnce() -> R) -> R {
        let _guard = self.set_current();
        f()
    }
}

thread_local! {
    static CONTEXT: Arc<RwLock<Context>> = Arc::new(RwLock::new(Context::default()));
}

/// RAII guard that restores previous context on drop
pub struct ContextGuard {
    old: Option<Context>,
}

impl Drop for ContextGuard {
    fn drop(&mut self) {
        if let Some(old) = self.old.take() {
            CONTEXT.with(|ctx| {
                *ctx.write() = old;
            });
        }
    }
}

/// Re-export Fields for convenience
pub use crate::config::Fields;
