//! Tests for ResourceGuard drop callback behavior

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nebula_resource::LifecycleState;
use nebula_resource::core::resource::{
    ResourceGuard, ResourceId, ResourceInstanceMetadata, TypedResourceInstance,
};

fn make_instance() -> TypedResourceInstance<String> {
    let metadata = ResourceInstanceMetadata {
        instance_id: uuid::Uuid::new_v4(),
        resource_id: ResourceId::new("test", "1.0"),
        state: LifecycleState::Ready,
        context: nebula_resource::ResourceContext::new(
            "wf".to_string(),
            "wf".to_string(),
            "ex".to_string(),
            "test".to_string(),
        ),
        created_at: chrono::Utc::now(),
        last_accessed_at: None,
        tags: std::collections::HashMap::new(),
    };
    TypedResourceInstance::new(Arc::new("guarded_resource".to_string()), metadata)
}

#[test]
fn guard_drop_calls_callback() {
    let called = Arc::new(AtomicBool::new(false));
    let called_clone = Arc::clone(&called);

    let instance = make_instance();
    let guard = ResourceGuard::new(instance, move |_resource| {
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
fn guard_release_prevents_callback() {
    let called = Arc::new(AtomicBool::new(false));
    let called_clone = Arc::clone(&called);

    let instance = make_instance();
    let guard = ResourceGuard::new(instance, move |_resource| {
        called_clone.store(true, Ordering::SeqCst);
    });

    // Release takes ownership of the resource, preventing the drop callback
    let released = guard.release();
    assert!(released.is_some(), "release should return the resource");

    // guard is now consumed by release(), drop happens but resource was taken
    // The callback should NOT have been called
    assert!(
        !called.load(Ordering::SeqCst),
        "callback should NOT be called after release"
    );
}

#[test]
fn guard_deref_provides_access_to_inner() {
    let instance = make_instance();
    let guard = ResourceGuard::new(instance, |_| {});

    // Deref should provide access to the inner String
    let inner: &String = &*guard;
    assert_eq!(inner, "guarded_resource");
}

#[test]
fn guard_as_ref_returns_some_before_release() {
    let instance = make_instance();
    let guard = ResourceGuard::new(instance, |_| {});

    assert!(guard.as_ref().is_some());
    assert_eq!(guard.as_ref().unwrap(), "guarded_resource");
}

#[test]
fn guard_metadata_returns_some_before_release() {
    let instance = make_instance();
    let guard = ResourceGuard::new(instance, |_| {});

    let meta = guard.metadata();
    assert!(meta.is_some());
    assert_eq!(meta.unwrap().resource_id.name, "test");
}

#[test]
fn guard_callback_receives_the_resource() {
    let received_id = Arc::new(parking_lot::Mutex::new(String::new()));
    let received_id_clone = Arc::clone(&received_id);

    let instance = make_instance();
    let guard = ResourceGuard::new(instance, move |resource| {
        *received_id_clone.lock() = resource.as_ref().clone();
    });

    drop(guard);

    assert_eq!(
        *received_id.lock(),
        "guarded_resource",
        "callback should receive the resource instance"
    );
}
