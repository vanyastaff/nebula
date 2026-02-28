//! Observability event system for nebula ecosystem
//!
//! This module provides a unified event system that allows different nebula crates
//! to emit domain-specific events and register hooks to observe them.
//!
//! # Architecture
//!
//! - **Events**: Implement [`ObservabilityEvent`] trait to define custom events
//! - **Hooks**: Implement [`ObservabilityHook`] trait to receive events
//! - **Registry**: Global registry manages hooks and event emission
//!
//! # Example
//!
//! ```rust
//! use nebula_log::observability::{ObservabilityEvent, ObservabilityHook, register_hook, emit_event};
//! use std::sync::Arc;
//!
//! // Define custom event
//! struct MyEvent {
//!     name: String,
//! }
//!
//! impl ObservabilityEvent for MyEvent {
//!     fn name(&self) -> &str {
//!         &self.name
//!     }
//! }
//!
//! // Define custom hook
//! struct MyHook;
//!
//! impl ObservabilityHook for MyHook {
//!     fn on_event(&self, event: &dyn ObservabilityEvent) {
//!         println!("Received event: {}", event.name());
//!     }
//! }
//!
//! // Register and emit
//! register_hook(Arc::new(MyHook));
//! emit_event(&MyEvent { name: "test".to_string() });
//! ```

pub mod context;
mod events;
mod filter;
mod hooks;
mod registry;
mod resources;
mod span;

/// Hook execution policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum HookPolicy {
    /// Dispatch hooks inline on the caller thread.
    #[default]
    Inline,
    /// Dispatch hooks with bounded budget.
    Bounded {
        /// Maximum budget for a hook callback in milliseconds.
        timeout_ms: u64,
        /// Queue capacity reserved for offloaded callbacks.
        queue_capacity: usize,
    },
}

// Re-export main types
pub use context::{
    ContextSnapshot, ExecutionContext, GlobalContext, NodeContext, ResourceMap, current_contexts,
};
pub use events::{OperationCompleted, OperationFailed, OperationStarted, OperationTracker};
pub use filter::{EventFilter, FilteredHook};
pub use hooks::{
    LoggingHook, ObservabilityEvent, ObservabilityHook, ResourceAwareAdapter, ResourceAwareHook,
};
pub use registry::{emit_event, register_hook, set_hook_policy, shutdown_hooks};
pub use resources::{LogLevel, LoggerResource, NotificationPrefs, NotificationSeverity};
pub use span::get_current_logger_resource;

#[cfg(feature = "observability")]
pub use hooks::MetricsHook;
