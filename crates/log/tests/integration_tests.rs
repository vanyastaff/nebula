//! Integration tests for nebula-log
//!
//! These tests verify that different components work together correctly.

use nebula_log::observability::{
    ObservabilityEvent, ObservabilityHook, OperationTracker, emit_event, register_hook,
    shutdown_hooks,
};
use nebula_log::{info, warn};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

// Serialization lock for tests using global state
static TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Test that observability hooks integrate with standard logging
#[test]
fn test_observability_with_logging() {
    let _guard = TEST_LOCK.lock().unwrap();
    shutdown_hooks();

    // Initialize logging for test (using default config)
    let _ = nebula_log::init();

    // Register a hook
    let count = Arc::new(AtomicUsize::new(0));
    let hook = CountingHook {
        count: Arc::clone(&count),
    };
    register_hook(Arc::new(hook));

    // Emit some log events
    info!("Test log message");
    warn!(component = "test", "Warning message");

    // Emit observability events
    let event = TestEvent {
        name: "test_event".to_string(),
    };
    emit_event(&event);

    // Hook should have received the observability event
    assert_eq!(count.load(Ordering::SeqCst), 1);

    shutdown_hooks();
}

/// Test that multiple hooks fire on the same event
#[test]
fn test_multiple_hooks_same_event() {
    let _guard = TEST_LOCK.lock().unwrap();
    shutdown_hooks();

    let count1 = Arc::new(AtomicUsize::new(0));
    let count2 = Arc::new(AtomicUsize::new(0));
    let count3 = Arc::new(AtomicUsize::new(0));

    register_hook(Arc::new(CountingHook {
        count: Arc::clone(&count1),
    }));
    register_hook(Arc::new(CountingHook {
        count: Arc::clone(&count2),
    }));
    register_hook(Arc::new(CountingHook {
        count: Arc::clone(&count3),
    }));

    // Emit single event
    let event = TestEvent {
        name: "multi_hook_test".to_string(),
    };
    emit_event(&event);

    // All hooks should fire
    assert_eq!(count1.load(Ordering::SeqCst), 1);
    assert_eq!(count2.load(Ordering::SeqCst), 1);
    assert_eq!(count3.load(Ordering::SeqCst), 1);

    // Emit another event
    emit_event(&event);

    // All counts should increment
    assert_eq!(count1.load(Ordering::SeqCst), 2);
    assert_eq!(count2.load(Ordering::SeqCst), 2);
    assert_eq!(count3.load(Ordering::SeqCst), 2);

    shutdown_hooks();
}

/// Test concurrent hook registration
#[test]
fn test_concurrent_registration() {
    use std::thread;

    let _guard = TEST_LOCK.lock().unwrap();
    shutdown_hooks();

    // Spawn multiple threads registering hooks concurrently
    let handles: Vec<_> = (0..5)
        .map(|_i| {
            thread::spawn(move || {
                let count = Arc::new(AtomicUsize::new(0));
                let hook = CountingHook {
                    count: Arc::clone(&count),
                };
                register_hook(Arc::new(hook));
                count
            })
        })
        .collect();

    let counts: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    // Emit event to all hooks
    let event = TestEvent {
        name: "concurrent_test".to_string(),
    };
    emit_event(&event);

    // Verify all hooks received the event
    for count in &counts {
        assert!(count.load(Ordering::SeqCst) > 0);
    }

    shutdown_hooks();
}

/// Test operation tracker RAII behavior
#[test]
fn test_operation_tracker_raii() {
    let _guard = TEST_LOCK.lock().unwrap();
    shutdown_hooks();

    let completed_count = Arc::new(AtomicUsize::new(0));
    let failed_count = Arc::new(AtomicUsize::new(0));

    let hook = TrackingHook {
        completed: Arc::clone(&completed_count),
        failed: Arc::clone(&failed_count),
    };
    register_hook(Arc::new(hook));

    // Test success case
    {
        let tracker = OperationTracker::new("test_op", "test");
        tracker.success(); // Must call success() explicitly
    }

    assert_eq!(completed_count.load(Ordering::SeqCst), 1);
    assert_eq!(failed_count.load(Ordering::SeqCst), 0);

    // Test failure case
    {
        let tracker = OperationTracker::new("test_op_fail", "test");
        tracker.fail("Test error");
    }

    assert_eq!(completed_count.load(Ordering::SeqCst), 1);
    assert_eq!(failed_count.load(Ordering::SeqCst), 1);

    shutdown_hooks();
}

/// Test hook shutdown cleanup
#[test]
fn test_hook_shutdown_cleanup() {
    let _guard = TEST_LOCK.lock().unwrap();
    shutdown_hooks();

    let shutdown_count = Arc::new(AtomicUsize::new(0));

    let hook = ShutdownCountingHook {
        count: Arc::clone(&shutdown_count),
    };
    register_hook(Arc::new(hook));

    // Shutdown should call hook's shutdown method
    shutdown_hooks();

    assert_eq!(shutdown_count.load(Ordering::SeqCst), 1);

    // Registry should be empty (we can't easily check count from integration tests)
}

/// Test high-frequency event emission
#[test]
fn test_high_frequency_events() {
    let _guard = TEST_LOCK.lock().unwrap();
    shutdown_hooks();

    let count = Arc::new(AtomicUsize::new(0));
    let hook = CountingHook {
        count: Arc::clone(&count),
    };
    register_hook(Arc::new(hook));

    // Emit 1000 events rapidly
    for i in 0..1000 {
        let event = TestEvent {
            name: format!("event_{}", i),
        };
        emit_event(&event);
    }

    // All events should be processed
    assert_eq!(count.load(Ordering::SeqCst), 1000);

    shutdown_hooks();
}

/// Test empty event (minimal data)
#[test]
fn test_empty_event() {
    let _guard = TEST_LOCK.lock().unwrap();
    shutdown_hooks();

    let count = Arc::new(AtomicUsize::new(0));
    register_hook(Arc::new(CountingHook {
        count: Arc::clone(&count),
    }));

    // Event with minimal data
    let event = EmptyEvent;
    emit_event(&event);

    assert_eq!(count.load(Ordering::SeqCst), 1);

    shutdown_hooks();
}

/// Test very long event names
#[test]
fn test_long_event_name() {
    let _guard = TEST_LOCK.lock().unwrap();
    shutdown_hooks();

    let count = Arc::new(AtomicUsize::new(0));
    register_hook(Arc::new(CountingHook {
        count: Arc::clone(&count),
    }));

    // Very long event name (10KB)
    let long_name = "x".repeat(10_000);
    let event = TestEvent { name: long_name };
    emit_event(&event);

    assert_eq!(count.load(Ordering::SeqCst), 1);

    shutdown_hooks();
}

// ============================================================================
// Test Helpers
// ============================================================================

struct TestEvent {
    name: String,
}

impl ObservabilityEvent for TestEvent {
    fn name(&self) -> &str {
        &self.name
    }
}

struct EmptyEvent;

impl ObservabilityEvent for EmptyEvent {
    fn name(&self) -> &str {
        ""
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

struct TrackingHook {
    completed: Arc<AtomicUsize>,
    failed: Arc<AtomicUsize>,
}

impl ObservabilityHook for TrackingHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        match event.name() {
            name if name.contains("completed") => {
                self.completed.fetch_add(1, Ordering::SeqCst);
            }
            name if name.contains("failed") => {
                self.failed.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        }
    }
}

struct ShutdownCountingHook {
    count: Arc<AtomicUsize>,
}

impl ObservabilityHook for ShutdownCountingHook {
    fn on_event(&self, _event: &dyn ObservabilityEvent) {}

    fn shutdown(&self) {
        self.count.fetch_add(1, Ordering::SeqCst);
    }
}
