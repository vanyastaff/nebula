//! Global registry for observability hooks
//!
//! This module provides a thread-safe global registry for managing
//! observability hooks and emitting events.

use super::hooks::{ObservabilityEvent, ObservabilityHook};
use arc_swap::ArcSwap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, LazyLock, Mutex};

/// Hook list stored inside [`ArcSwap`] for lock-free reads.
///
/// The outer `Arc<Vec<...>>` is managed by `ArcSwap`, and each hook is
/// individually `Arc`-wrapped so hooks can be shared across registry snapshots.
type HookList = Vec<Arc<dyn ObservabilityHook>>;

/// Emit an event to a hook list.
///
/// Calls `on_event()` on each registered hook with the provided event.
/// If a hook panics, the panic is caught and logged, and other hooks
/// continue to receive events.
///
/// # Panic Safety
///
/// Hooks **must** be internally panic-safe. If a hook panics while holding
/// internal locks (e.g., a `Mutex`), those locks will be poisoned. The
/// registry catches the panic and continues dispatching to remaining hooks,
/// but the panicked hook's internal state may be corrupted. Consider wrapping
/// fallible hook internals in `catch_unwind` or using lock-free data structures.
fn emit_to_hooks(hooks: &HookList, event: &dyn ObservabilityEvent) {
    for hook in hooks.iter() {
        // No Arc::clone needed — we borrow through the slice reference,
        // which is kept alive by the ArcSwap guard for the duration of emit.
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            hook.on_event(event);
        }));

        if let Err(panic_info) = result {
            tracing::error!(
                event_name = event.name(),
                hook_type = std::any::type_name::<dyn ObservabilityHook>(),
                panic = ?panic_info,
                "Hook panicked while processing event"
            );
        }
    }
}

/// Initialize a hook, catching panics.
///
/// Returns `true` if initialization succeeded, `false` if the hook panicked.
fn try_initialize_hook(hook: &dyn ObservabilityHook) -> bool {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        hook.initialize();
    }));

    if let Err(panic_info) = result {
        tracing::error!(
            hook_type = std::any::type_name::<dyn ObservabilityHook>(),
            panic = ?panic_info,
            "Hook initialization panicked"
        );
        return false;
    }

    true
}

/// Shutdown all hooks in a list, catching panics.
fn shutdown_hooks_list(hooks: &HookList) {
    for hook in hooks.iter() {
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            hook.shutdown();
        }));

        if let Err(panic_info) = result {
            tracing::error!(
                hook_type = std::any::type_name::<dyn ObservabilityHook>(),
                panic = ?panic_info,
                "Hook panicked during shutdown"
            );
        }
    }
}

/// Global static hook list using ArcSwap for lock-free reads.
///
/// # Performance Characteristics
///
/// - **Emit (read)**: Lock-free, zero contention across threads
/// - **Register/Shutdown (write)**: Mutex-coordinated, infrequent operations
///
/// This design optimizes for the common case (emit) at the expense of
/// rare operations (register).
static HOOKS: LazyLock<ArcSwap<HookList>> = LazyLock::new(|| ArcSwap::from_pointee(Vec::new()));

/// Mutex for coordinating write operations (register/shutdown).
static WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Register a global observability hook.
///
/// The hook will receive all events emitted via [`emit_event`].
///
/// # Performance
///
/// This is a write operation that requires acquiring a mutex.
/// It's designed for infrequent calls (during initialization).
///
/// # Panic Safety
///
/// The hook's `initialize()` method is called immediately. If it panics,
/// the panic is caught, logged, and the hook is **not** registered.
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
    let _guard = WRITE_LOCK.lock().expect("registry write lock poisoned");

    // Initialize the hook; skip registration if it panics
    if !try_initialize_hook(&*hook) {
        return;
    }

    let current = HOOKS.load();
    let mut new_hooks = (**current).clone();
    new_hooks.push(hook);
    HOOKS.store(Arc::new(new_hooks));
}

/// Emit an event to all registered hooks.
///
/// All registered hooks will receive this event via their `on_event()` method.
///
/// # Performance
///
/// This is a **lock-free** operation. Multiple threads can emit events
/// concurrently without any contention. This is critical for high-throughput
/// workflow execution.
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
    let hooks = HOOKS.load();
    emit_to_hooks(&hooks, event);
}

/// Shutdown all registered hooks.
///
/// Calls `shutdown()` on each hook and clears the registry.
/// This should typically be called during application shutdown.
///
/// # Performance
///
/// This is a write operation that requires acquiring a mutex.
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
    let _guard = WRITE_LOCK.lock().expect("registry write lock poisoned");
    let current = HOOKS.load();
    shutdown_hooks_list(&current);
    HOOKS.store(Arc::new(Vec::new()));
}

/// Get the number of registered hooks (for testing)
#[cfg(test)]
fn hook_count() -> usize {
    HOOKS.load().len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Serialize all tests to prevent interference via global state
    static TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

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
        let _guard = TEST_LOCK.lock().unwrap();

        // Clean up any hooks from previous tests
        shutdown_hooks();

        let count = Arc::new(AtomicUsize::new(0));
        let hook = CountingHook {
            count: Arc::clone(&count),
        };

        register_hook(Arc::new(hook));
        assert_eq!(hook_count(), 1);

        let event = TestEvent {
            name: "test".to_string(),
        };
        emit_event(&event);

        // The hook should have been called
        assert_eq!(count.load(Ordering::SeqCst), 1);

        // Clean up
        shutdown_hooks();
        assert_eq!(hook_count(), 0);
    }

    #[test]
    fn test_multiple_hooks() {
        let _guard = TEST_LOCK.lock().unwrap();
        shutdown_hooks();

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

        // Both hooks should have been called exactly once
        assert_eq!(count1.load(Ordering::SeqCst), 1);
        assert_eq!(count2.load(Ordering::SeqCst), 1);

        // Clean up
        shutdown_hooks();
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;

        let _guard = TEST_LOCK.lock().unwrap();
        shutdown_hooks();

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

        // At least 10 events should have been processed (could be more if other hooks active)
        assert!(
            count.load(Ordering::SeqCst) >= 10,
            "Expected at least 10 events"
        );

        // Clean up
        shutdown_hooks();
    }

    struct PanickingHook {
        panic_on: &'static str,
    }

    impl ObservabilityHook for PanickingHook {
        fn on_event(&self, event: &dyn ObservabilityEvent) {
            if event.name() == self.panic_on {
                panic!("Intentional panic for testing");
            }
        }
    }

    #[test]
    fn test_panic_safety() {
        let _guard = TEST_LOCK.lock().unwrap();

        // Clear any existing hooks
        shutdown_hooks();

        let count = Arc::new(AtomicUsize::new(0));
        let good_hook = CountingHook {
            count: Arc::clone(&count),
        };
        let bad_hook = PanickingHook {
            panic_on: "panic_event",
        };

        register_hook(Arc::new(good_hook));
        register_hook(Arc::new(bad_hook));

        // Emit event that will cause panic
        let panic_event = TestEvent {
            name: "panic_event".to_string(),
        };
        emit_event(&panic_event);

        // Good hook should still have been called
        assert!(count.load(Ordering::SeqCst) > 0);

        // System should still be functional - emit another event
        let normal_event = TestEvent {
            name: "normal_event".to_string(),
        };
        let before = count.load(Ordering::SeqCst);
        emit_event(&normal_event);
        let after = count.load(Ordering::SeqCst);

        // Good hook should have processed the second event
        assert!(after > before);

        // Clean up
        shutdown_hooks();
    }

    struct PanickingInitHook;

    impl ObservabilityHook for PanickingInitHook {
        fn initialize(&self) {
            panic!("Panic during initialization");
        }

        fn on_event(&self, _event: &dyn ObservabilityEvent) {
            // Should never be called
        }
    }

    #[test]
    fn test_panic_during_initialization() {
        let _guard = TEST_LOCK.lock().unwrap();
        shutdown_hooks();

        let initial_count = hook_count();

        // Try to register a hook that panics during initialization
        let bad_hook = PanickingInitHook;
        register_hook(Arc::new(bad_hook));

        // Hook should not have been registered
        assert_eq!(hook_count(), initial_count);

        shutdown_hooks();
    }
}
