//! Tests for ResourceGuard drop callback behavior

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nebula_resource::resource::ResourceGuard;

#[test]
fn drop_fires_callback() {
    let called = Arc::new(AtomicBool::new(false));
    let called_clone = Arc::clone(&called);

    let guard = ResourceGuard::new("guarded-value".to_string(), move |_instance| {
        called_clone.store(true, Ordering::SeqCst);
    });

    assert!(
        !called.load(Ordering::SeqCst),
        "callback should not fire before drop"
    );

    drop(guard);

    assert!(
        called.load(Ordering::SeqCst),
        "callback should fire on drop"
    );
}

#[test]
fn into_inner_prevents_callback() {
    let called = Arc::new(AtomicBool::new(false));
    let called_clone = Arc::clone(&called);

    let guard = ResourceGuard::new("guarded-value".to_string(), move |_instance| {
        called_clone.store(true, Ordering::SeqCst);
    });

    let released = guard.into_inner();
    assert_eq!(released, "guarded-value");

    assert!(
        !called.load(Ordering::SeqCst),
        "callback should NOT fire after into_inner()"
    );
}

#[test]
fn deref_accesses_inner_value() {
    let guard = ResourceGuard::new("guarded-value".to_string(), |_| {});
    let value: &String = &*guard;
    assert_eq!(value, "guarded-value");
}
