//! Context management for structured logging
//!
//! # Async-Safe Storage
//!
//! When the `async` feature is enabled, the context uses `tokio::task_local!`
//! and survives across `.await` points in multi-thread Tokio runtimes.
//!
//! When the `async` feature is disabled, the context uses `thread_local!`
//! (suitable for synchronous code or single-thread runtimes).

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Storage backend
// ---------------------------------------------------------------------------

#[cfg(feature = "async")]
mod storage {
    use super::*;
    use std::future::Future;

    tokio::task_local! {
        static CTX: Arc<Context>;
    }

    #[inline]
    pub fn current() -> Arc<Context> {
        CTX.try_with(|c| c.clone())
            .unwrap_or_else(|_| Arc::new(Context::default()))
    }

    pub async fn with_ctx<F: Future>(ctx: Arc<Context>, f: F) -> F::Output {
        CTX.scope(ctx, f).await
    }

    pub fn with_ctx_sync<R>(ctx: Arc<Context>, f: impl FnOnce() -> R) -> R {
        CTX.sync_scope(ctx, f)
    }
}

#[cfg(not(feature = "async"))]
mod storage {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static CTX: RefCell<Arc<Context>> = RefCell::new(Arc::new(Context::default()));
    }

    #[inline]
    pub fn current() -> Arc<Context> {
        CTX.with(|c| c.borrow().clone())
    }

    pub fn with_ctx_sync<R>(ctx: Arc<Context>, f: impl FnOnce() -> R) -> R {
        CTX.with(|cell| {
            let prev = cell.borrow().clone();
            *cell.borrow_mut() = ctx;
            let result = f();
            *cell.borrow_mut() = prev;
            result
        })
    }
}

// ---------------------------------------------------------------------------
// Context type
// ---------------------------------------------------------------------------

/// Context for structured logging
///
/// Contains request-scoped fields like request ID, user ID, etc.
/// Activate via `scope()` (async) or `scope_sync()` (sync).
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

    /// Get current context (cheap `Arc::clone`, no deep copy)
    #[inline]
    #[must_use]
    pub fn current() -> Arc<Self> {
        storage::current()
    }

    /// Run a synchronous closure with this context active.
    ///
    /// Nesting is supported — inner scopes shadow outer ones and restore on return.
    pub fn scope_sync<R>(self, f: impl FnOnce() -> R) -> R {
        storage::with_ctx_sync(Arc::new(self), f)
    }

    /// Run a future with this context active.
    ///
    /// The context survives across `.await` points, even in multi-thread
    /// Tokio runtimes with work-stealing.
    #[cfg(feature = "async")]
    pub async fn scope<F: std::future::Future>(self, f: F) -> F::Output {
        storage::with_ctx(Arc::new(self), f).await
    }
}

/// Re-export Fields for convenience
pub use crate::config::Fields;
