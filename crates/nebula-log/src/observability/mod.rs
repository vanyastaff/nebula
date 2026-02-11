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

// Re-export main types
pub use context::{
    ContextSnapshot, ExecutionContext, ExecutionGuard, GlobalContext, GlobalGuard, NodeContext,
    NodeGuard, ResourceMap, current_contexts,
};
pub use events::{OperationCompleted, OperationFailed, OperationStarted, OperationTracker};
pub use filter::{EventFilter, FilteredHook};
pub use hooks::{
    LoggingHook, ObservabilityEvent, ObservabilityHook, ResourceAwareAdapter, ResourceAwareHook,
};
pub use registry::{emit_event, register_hook, shutdown_hooks};
pub use resources::{LogLevel, LoggerResource, NotificationPrefs, NotificationSeverity};
pub use span::get_current_logger_resource;

#[cfg(feature = "observability")]
pub use hooks::MetricsHook;
