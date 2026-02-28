use nebula_log::observability::{
    HookPolicy, ObservabilityEvent, ObservabilityHook, emit_event, register_hook, set_hook_policy,
    shutdown_hooks,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

static TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

struct SlowEvent;

impl ObservabilityEvent for SlowEvent {
    fn name(&self) -> &str {
        "slow_event"
    }
}

struct SlowHook {
    count: Arc<AtomicUsize>,
}

impl ObservabilityHook for SlowHook {
    fn on_event(&self, _event: &dyn ObservabilityEvent) {
        std::thread::sleep(Duration::from_millis(5));
        self.count.fetch_add(1, Ordering::SeqCst);
    }

    fn shutdown(&self) {
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn hook_policy_can_be_set_to_bounded() {
    let _guard = TEST_LOCK.lock().expect("test lock");
    shutdown_hooks();

    set_hook_policy(HookPolicy::Bounded {
        timeout_ms: 10,
        queue_capacity: 64,
    });

    let count = Arc::new(AtomicUsize::new(0));
    register_hook(Arc::new(SlowHook {
        count: Arc::clone(&count),
    }));
    emit_event(&SlowEvent);
    assert_eq!(count.load(Ordering::SeqCst), 1);

    shutdown_hooks();
}

#[test]
fn bounded_policy_shutdown_does_not_panic() {
    let _guard = TEST_LOCK.lock().expect("test lock");
    shutdown_hooks();

    set_hook_policy(HookPolicy::Bounded {
        timeout_ms: 1,
        queue_capacity: 8,
    });
    register_hook(Arc::new(SlowHook {
        count: Arc::new(AtomicUsize::new(0)),
    }));

    shutdown_hooks();
}
