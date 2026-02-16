//! RAII guard for resource instances

/// RAII guard that wraps a resource instance.
///
/// When the guard is dropped, the on-drop callback is invoked (typically
/// returning the instance to the pool). Use `into_inner()` to take
/// ownership without triggering the callback.
pub struct Guard<T> {
    resource: Option<T>,
    on_drop: Option<Box<dyn FnOnce(T) + Send>>,
}

impl<T> Guard<T> {
    /// Create a new guard wrapping `resource` with a drop callback.
    pub fn new<F>(resource: T, on_drop: F) -> Self
    where
        F: FnOnce(T) + Send + 'static,
    {
        Self {
            resource: Some(resource),
            on_drop: Some(Box::new(on_drop)),
        }
    }

    /// Take the resource out of the guard, preventing the drop callback.
    #[must_use]
    pub fn into_inner(mut self) -> T {
        self.on_drop.take(); // prevent callback
        self.resource.take().expect("guard used after into_inner")
    }
}

impl<T> std::ops::Deref for Guard<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.resource.as_ref().expect("guard used after into_inner")
    }
}

impl<T> std::ops::DerefMut for Guard<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.resource.as_mut().expect("guard used after into_inner")
    }
}

impl<T> Drop for Guard<T> {
    fn drop(&mut self) {
        if let (Some(resource), Some(on_drop)) = (self.resource.take(), self.on_drop.take()) {
            on_drop(resource);
        }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Guard<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guard")
            .field("resource", &self.resource)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn guard_deref() {
        let guard = Guard::new(42u32, |_| {});
        assert_eq!(*guard, 42);
    }

    #[test]
    fn guard_drop_fires_callback() {
        let called = Arc::new(AtomicBool::new(false));
        let called_c = called.clone();
        let guard = Guard::new("hello", move |_| {
            called_c.store(true, Ordering::SeqCst);
        });
        assert!(!called.load(Ordering::SeqCst));
        drop(guard);
        assert!(called.load(Ordering::SeqCst));
    }

    #[test]
    fn guard_into_inner_prevents_callback() {
        let called = Arc::new(AtomicBool::new(false));
        let called_c = called.clone();
        let guard = Guard::new(99u32, move |_| {
            called_c.store(true, Ordering::SeqCst);
        });
        let val = guard.into_inner();
        assert_eq!(val, 99);
        assert!(!called.load(Ordering::SeqCst));
    }

    #[test]
    fn guard_deref_mut() {
        let mut guard = Guard::new(String::from("hello"), |_| {});
        guard.push_str(" world");
        assert_eq!(*guard, "hello world");
    }
}
