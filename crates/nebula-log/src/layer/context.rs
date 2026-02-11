//! Context management for structured logging
//!
//! # Thread-Local Storage
//!
//! Contexts are stored in thread-local storage and are **not** propagated across
//! `.await` points in async runtimes with work-stealing (e.g., Tokio multi-thread).
//! For async context propagation, use `tracing::Span` fields instead.

use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

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
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set request ID
    #[must_use]
    pub fn with_request_id(mut self, id: impl Into<String>) -> Self {
        self.request_id = Some(id.into());
        self
    }

    /// Set user ID
    #[must_use]
    pub fn with_user_id(mut self, id: impl Into<String>) -> Self {
        self.user_id = Some(id.into());
        self
    }

    /// Add a field
    #[must_use]
    pub fn with_field(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        if let Ok(v) = serde_json::to_value(value) {
            self.fields.insert(key.into(), v);
        }
        self
    }

    /// Set as current context
    #[must_use]
    pub fn set_current(self) -> ContextGuard {
        CONTEXT.with(|ctx| {
            let old = ctx.borrow().clone();
            *ctx.borrow_mut() = Arc::new(self);
            ContextGuard {
                old: Some(old),
                _not_send: PhantomData,
            }
        })
    }

    /// Get current context (cheap `Arc::clone`, no deep copy)
    #[inline]
    #[must_use]
    pub fn current() -> Arc<Self> {
        CONTEXT.with(|ctx| Arc::clone(&ctx.borrow()))
    }

    /// Run a closure with this context
    pub fn scope<R>(self, f: impl FnOnce() -> R) -> R {
        let _guard = self.set_current();
        f()
    }
}

thread_local! {
    static CONTEXT: RefCell<Arc<Context>> = RefCell::new(Arc::new(Context::default()));
}

/// RAII guard that restores previous context on drop
///
/// This guard is `!Send` because it references thread-local storage.
/// Sending it across threads would restore context on the wrong thread.
#[derive(Debug)]
pub struct ContextGuard {
    old: Option<Arc<Context>>,
    /// Explicit `!Send` marker — thread-local guards must not cross threads
    _not_send: PhantomData<*const ()>,
}

impl Drop for ContextGuard {
    fn drop(&mut self) {
        if let Some(old) = self.old.take() {
            CONTEXT.with(|ctx| {
                *ctx.borrow_mut() = old;
            });
        }
    }
}

/// Re-export Fields for convenience
pub use crate::config::Fields;
