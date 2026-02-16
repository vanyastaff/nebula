//! Tests for Guard behavior

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nebula_resource::Guard;

#[test]
fn guard_drop_calls_callback() {
    let called = Arc::new(AtomicBool::new(false));
    let called_clone = Arc::clone(&called);

    let guard = Guard::new("guarded_resource".to_string(), move |_resource| {
        called_clone.store(true, Ordering::SeqCst);
    });

    assert!(
        !called.load(Ordering::SeqCst),
        "callback should not be called yet"
    );

    drop(guard);

    assert!(
        called.load(Ordering::SeqCst),
        "callback should have been called on drop"
    );
}

#[test]
fn guard_into_inner_prevents_callback() {
    let called = Arc::new(AtomicBool::new(false));
    let called_clone = Arc::clone(&called);

    let guard = Guard::new("guarded_resource".to_string(), move |_resource| {
        called_clone.store(true, Ordering::SeqCst);
    });

    let released = guard.into_inner();
    assert_eq!(released, "guarded_resource");

    assert!(
        !called.load(Ordering::SeqCst),
        "callback should NOT be called after into_inner"
    );
}

#[test]
fn guard_deref_provides_access_to_inner() {
    let guard = Guard::new("hello".to_string(), |_| {});
    let inner: &String = &*guard;
    assert_eq!(inner, "hello");
}

#[test]
fn guard_deref_mut_allows_modification() {
    let mut guard = Guard::new("hello".to_string(), |_| {});
    guard.push_str(" world");
    assert_eq!(*guard, "hello world");
}

#[test]
fn guard_callback_receives_the_resource() {
    let received = Arc::new(parking_lot::Mutex::new(String::new()));
    let received_clone = Arc::clone(&received);

    let guard = Guard::new("test_value".to_string(), move |resource| {
        *received_clone.lock() = resource;
    });

    drop(guard);

    assert_eq!(
        *received.lock(),
        "test_value",
        "callback should receive the resource instance"
    );
}
