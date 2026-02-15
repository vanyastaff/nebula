//! Tests for ResourceGuard drop callback behavior

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nebula_resource::LifecycleState;
use nebula_resource::core::{
    context::ResourceContext,
    resource::{ResourceGuard, ResourceId, ResourceInstanceMetadata, TypedResourceInstance},
};

fn make_instance() -> TypedResourceInstance<String> {
    let metadata = ResourceInstanceMetadata {
        instance_id: uuid::Uuid::new_v4(),
        resource_id: ResourceId::new("guard-test", "1.0"),
        state: LifecycleState::Ready,
        context: ResourceContext::new(
            "wf".to_string(),
            "wf-name".to_string(),
            "ex".to_string(),
            "dev".to_string(),
        ),
        created_at: chrono::Utc::now(),
        last_accessed_at: None,
        tags: std::collections::HashMap::new(),
    };
    TypedResourceInstance::new(Arc::new("guarded-value".to_string()), metadata)
}

#[test]
fn drop_fires_callback() {
    let called = Arc::new(AtomicBool::new(false));
    let called_clone = Arc::clone(&called);

    let guard = ResourceGuard::new(make_instance(), move |_instance| {
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
fn release_prevents_callback() {
    let called = Arc::new(AtomicBool::new(false));
    let called_clone = Arc::clone(&called);

    let guard = ResourceGuard::new(make_instance(), move |_instance| {
        called_clone.store(true, Ordering::SeqCst);
    });

    // release() takes the resource out, preventing the drop callback
    let released = guard.release();
    assert!(released.is_some(), "release should return the instance");

    // guard is now dropped (moved into release), but callback should NOT fire
    assert!(
        !called.load(Ordering::SeqCst),
        "callback should NOT fire after release()"
    );
}

#[test]
fn deref_accesses_inner_value() {
    let guard = ResourceGuard::new(make_instance(), |_| {});

    // Deref should give us the inner String
    let value: &String = &*guard;
    assert_eq!(value, "guarded-value");
}

#[test]
fn as_ref_returns_inner() {
    let guard = ResourceGuard::new(make_instance(), |_| {});

    let inner = guard.as_ref();
    assert!(inner.is_some());
    assert_eq!(*inner.unwrap(), "guarded-value");
}

#[test]
fn metadata_is_accessible() {
    let guard = ResourceGuard::new(make_instance(), |_| {});

    let meta = guard.metadata();
    assert!(meta.is_some());
    assert_eq!(meta.unwrap().resource_id.name, "guard-test");
}
