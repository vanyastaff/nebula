//! Global registry for observability hooks
//!
//! This module provides a thread-safe global registry for managing
//! observability hooks and emitting events.

use super::hooks::{ObservabilityEvent, ObservabilityHook};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::sync::Arc;

/// Global registry for observability hooks
///
/// This registry maintains a collection of hooks and dispatches
/// events to all registered hooks.
///
/// # Thread Safety
///
/// The registry uses `parking_lot::RwLock` for concurrent access,
/// allowing multiple readers or a single writer.
pub struct ObservabilityRegistry {
    hooks: Vec<Arc<dyn ObservabilityHook>>,
}

impl ObservabilityRegistry {
    /// Create a new empty registry
    fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Register a new hook
    ///
    /// The hook's `initialize()` method will be called immediately.
    pub fn register(&mut self, hook: Arc<dyn ObservabilityHook>) {
        hook.initialize();
        self.hooks.push(hook);
    }

    /// Emit an event to all registered hooks
    ///
    /// Calls `on_event()` on each registered hook with the provided event.
    pub fn emit(&self, event: &dyn ObservabilityEvent) {
        for hook in &self.hooks {
            hook.on_event(event);
        }
    }

    /// Shutdown all hooks
    ///
    /// Calls `shutdown()` on each hook and clears the registry.
    pub fn shutdown(&mut self) {
        for hook in &self.hooks {
            hook.shutdown();
        }
        self.hooks.clear();
    }

    /// Get the number of registered hooks
    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }
}

/// Global static registry instance
static REGISTRY: Lazy<RwLock<ObservabilityRegistry>> =
    Lazy::new(|| RwLock::new(ObservabilityRegistry::new()));

/// Register a global observability hook
///
/// The hook will receive all events emitted via [`emit_event`].
///
/// # Example
///
/// ```rust
/// use nebula_log::observability::{LoggingHook, register_hook};
/// use std::sync::Arc;
///
/// let hook = LoggingHook::new(tracing::Level::INFO);
/// register_hook(Arc::new(hook));
/// ```
pub fn register_hook(hook: Arc<dyn ObservabilityHook>) {
    REGISTRY.write().register(hook);
}

/// Emit an event to all registered hooks
///
/// All registered hooks will receive this event via their `on_event()` method.
///
/// # Example
///
/// ```rust
/// use nebula_log::observability::{ObservabilityEvent, emit_event};
///
/// struct MyEvent;
///
/// impl ObservabilityEvent for MyEvent {
///     fn name(&self) -> &str {
///         "my_event"
///     }
/// }
///
/// emit_event(&MyEvent);
/// ```
pub fn emit_event(event: &dyn ObservabilityEvent) {
    REGISTRY.read().emit(event);
}

/// Shutdown all registered hooks
///
/// Calls `shutdown()` on each hook and clears the registry.
/// This should typically be called during application shutdown.
///
/// # Example
///
/// ```rust
/// use nebula_log::observability::shutdown_hooks;
///
/// // At application shutdown
/// shutdown_hooks();
/// ```
pub fn shutdown_hooks() {
    REGISTRY.write().shutdown();
}

/// Get the number of registered hooks (for testing)
#[doc(hidden)]
pub fn hook_count() -> usize {
    REGISTRY.read().hook_count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct TestEvent {
        name: String,
    }

    impl ObservabilityEvent for TestEvent {
        fn name(&self) -> &str {
            &self.name
        }
    }

    struct CountingHook {
        count: Arc<AtomicUsize>,
    }

    impl ObservabilityHook for CountingHook {
        fn on_event(&self, _event: &dyn ObservabilityEvent) {
            self.count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn test_registry_basic() {
        let count = Arc::new(AtomicUsize::new(0));
        let hook = CountingHook {
            count: Arc::clone(&count),
        };

        // Note: This test may interfere with other tests due to global state
        let initial_hooks = hook_count();

        register_hook(Arc::new(hook));
        assert_eq!(hook_count(), initial_hooks + 1);

        let event = TestEvent {
            name: "test".to_string(),
        };
        emit_event(&event);

        // The hook should have been called
        assert!(count.load(Ordering::SeqCst) > 0);

        // Clean up
        shutdown_hooks();
        assert_eq!(hook_count(), 0);
    }

    #[test]
    fn test_multiple_hooks() {
        let count1 = Arc::new(AtomicUsize::new(0));
        let count2 = Arc::new(AtomicUsize::new(0));

        let hook1 = CountingHook {
            count: Arc::clone(&count1),
        };
        let hook2 = CountingHook {
            count: Arc::clone(&count2),
        };

        register_hook(Arc::new(hook1));
        register_hook(Arc::new(hook2));

        let event = TestEvent {
            name: "multi_test".to_string(),
        };
        emit_event(&event);

        // Both hooks should have been called
        assert!(count1.load(Ordering::SeqCst) > 0);
        assert!(count2.load(Ordering::SeqCst) > 0);

        // Clean up
        shutdown_hooks();
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;

        let count = Arc::new(AtomicUsize::new(0));
        let hook = CountingHook {
            count: Arc::clone(&count),
        };

        register_hook(Arc::new(hook));

        // Spawn multiple threads emitting events
        let handles: Vec<_> = (0..10)
            .map(|i| {
                thread::spawn(move || {
                    let event = TestEvent {
                        name: format!("thread_{}", i),
                    };
                    emit_event(&event);
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // All events should have been processed
        assert!(count.load(Ordering::SeqCst) >= 10);

        // Clean up
        shutdown_hooks();
    }
}
