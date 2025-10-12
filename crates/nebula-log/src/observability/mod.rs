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

pub mod events;
pub mod hooks;
pub mod registry;

// Re-export main types
pub use events::{OperationCompleted, OperationFailed, OperationStarted, OperationTracker};
pub use hooks::{LoggingHook, ObservabilityEvent, ObservabilityHook};
pub use registry::{emit_event, register_hook, shutdown_hooks};

#[cfg(feature = "observability")]
pub use hooks::MetricsHook;
