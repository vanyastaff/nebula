//! Global registry for observability hooks
//!
//! This module provides a thread-safe global registry for managing
//! observability hooks and emitting events.

use super::HookPolicy;
use super::hooks::{ObservabilityEvent, ObservabilityHook};
use arc_swap::ArcSwap;
use parking_lot::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Instant;

/// Hook list stored inside [`ArcSwap`] for lock-free reads.
///
/// The outer `Arc<Vec<...>>` is managed by `ArcSwap`, and each hook is
/// individually `Arc`-wrapped so hooks can be shared across registry snapshots.
type HookList = Vec<Arc<dyn ObservabilityHook>>;

/// Emit an event to hooks with inline (panic-safe) dispatch.
///
/// Each hook's on_event() is wrapped in catch_unwind to ensure
/// one panicked hook doesn't poison others.
#[inline]
fn emit_to_hooks_inline(hooks: &HookList, event: &dyn ObservabilityEvent) {
    for hook in hooks.iter() {
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

/// Emit an event to hooks with bounded (timeout-aware) dispatch.
///
/// Batch timeout measurement instead of per-hook, reducing Instant::now() calls.
/// If batch exceeds timeout, remaining hooks are skipped.
#[inline]
fn emit_to_hooks_bounded(hooks: &HookList, event: &dyn ObservabilityEvent, timeout_ms: u64) {
    let started = Instant::now();

    for hook in hooks.iter() {
        if started.elapsed().as_millis() as u64 > timeout_ms {
            tracing::warn!(
                event_name = event.name(),
                timeout_ms = timeout_ms,
                "Hook dispatch exceeded configured execution budget"
            );
            break;
        }

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

/// Emit an event to a hook list (legacy, kept for compatibility).
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
#[deprecated(note = "use emit_to_hooks_inline or emit_to_hooks_bounded directly")]
#[allow(dead_code)]
fn emit_to_hooks(hooks: &HookList, event: &dyn ObservabilityEvent, policy: HookPolicy) {
    match policy {
        HookPolicy::Inline => emit_to_hooks_inline(hooks, event),
        HookPolicy::Bounded { timeout_ms, .. } => emit_to_hooks_bounded(hooks, event, timeout_ms),
    }
}

/// Initialize a hook, catching panics.
///
/// Returns `true` if initialization succeeded, `false` if the hook panicked.
/// Marked cold: hook registration is not in hot path.
#[cold]
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
/// Marked cold: shutdown is not in hot path.
#[cold]
fn shutdown_hooks_list(hooks: &HookList, policy: HookPolicy) {
    let timeout_ms = match policy {
        HookPolicy::Inline => None,
        HookPolicy::Bounded { timeout_ms, .. } => Some(timeout_ms),
    };

    // Shutdown in reverse registration order (LIFO) so dependencies can
    // be torn down in the opposite order of startup.
    for hook in hooks.iter().rev() {
        let started = Instant::now();
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            hook.shutdown();
        }));

        if let Err(panic_info) = result {
            tracing::error!(
                hook_type = std::any::type_name::<dyn ObservabilityHook>(),
                panic = ?panic_info,
                "Hook panicked during shutdown"
            );
            continue;
        }

        if let Some(timeout) = timeout_ms {
            let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            if elapsed > timeout {
                tracing::warn!(
                    elapsed_ms = elapsed,
                    timeout_ms = timeout,
                    "Hook shutdown exceeded configured execution budget"
                );
            }
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
static HOOK_POLICY: LazyLock<RwLock<HookPolicy>> =
    LazyLock::new(|| RwLock::new(HookPolicy::Inline));
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

#[inline]
fn lock_write_guard() -> MutexGuard<'static, ()> {
    WRITE_LOCK.lock()
}

#[inline]
fn policy_read_guard() -> RwLockReadGuard<'static, HookPolicy> {
    HOOK_POLICY.read()
}

#[inline]
fn policy_write_guard() -> RwLockWriteGuard<'static, HookPolicy> {
    HOOK_POLICY.write()
}

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
    // Initialize outside the lock — user code may be slow and this is
    // idempotent; the hook isn't visible to emit_event until HOOKS.store().
    if !try_initialize_hook(&*hook) {
        return;
    }

    let _guard = lock_write_guard();
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
#[inline(always)]
pub fn emit_event(event: &dyn ObservabilityEvent) {
    // Relaxed ordering is safe: SHUTTING_DOWN is only written during shutdown,
    // and we're already in a fire-and-forget path. No acquire needed.
    if SHUTTING_DOWN.load(Ordering::Relaxed) {
        return;
    }
    let hooks = HOOKS.load();
    let policy = *policy_read_guard();
    // Inline dispatch to reduce function call overhead in hot path
    match policy {
        HookPolicy::Inline => emit_to_hooks_inline(&hooks, event),
        HookPolicy::Bounded {
            timeout_ms,
            queue_capacity: _,
        } => {
            emit_to_hooks_bounded(&hooks, event, timeout_ms);
        }
    }
}

/// Set hook execution policy for subsequent emissions.
pub fn set_hook_policy(policy: HookPolicy) {
    let mut current = policy_write_guard();
    *current = policy;
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
    let _guard = lock_write_guard();
    SHUTTING_DOWN.store(true, Ordering::Release);
    let current = HOOKS.load();
    // Quiesce future dispatches first, then drain current snapshot.
    HOOKS.store(Arc::new(Vec::new()));
    let policy = *policy_read_guard();
    shutdown_hooks_list(&current, policy);
    SHUTTING_DOWN.store(false, Ordering::Release);
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
        let _guard = TEST_LOCK.lock();

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
        let _guard = TEST_LOCK.lock();
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

        let _guard = TEST_LOCK.lock();
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
        let _guard = TEST_LOCK.lock();

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
        let _guard = TEST_LOCK.lock();
        shutdown_hooks();

        let initial_count = hook_count();

        // Try to register a hook that panics during initialization
        let bad_hook = PanickingInitHook;
        register_hook(Arc::new(bad_hook));

        // Hook should not have been registered
        assert_eq!(hook_count(), initial_count);

        shutdown_hooks();
    }

    struct OrderedShutdownHook {
        id: u8,
        order: Arc<Mutex<Vec<u8>>>,
    }

    impl ObservabilityHook for OrderedShutdownHook {
        fn on_event(&self, _event: &dyn ObservabilityEvent) {}

        fn shutdown(&self) {
            self.order.lock().push(self.id);
        }
    }

    #[test]
    fn test_shutdown_order_is_lifo() {
        let _guard = TEST_LOCK.lock();
        shutdown_hooks();

        let order = Arc::new(Mutex::new(Vec::new()));
        register_hook(Arc::new(OrderedShutdownHook {
            id: 1,
            order: Arc::clone(&order),
        }));
        register_hook(Arc::new(OrderedShutdownHook {
            id: 2,
            order: Arc::clone(&order),
        }));
        register_hook(Arc::new(OrderedShutdownHook {
            id: 3,
            order: Arc::clone(&order),
        }));

        shutdown_hooks();
        assert_eq!(*order.lock(), vec![3, 2, 1]);
    }
}
