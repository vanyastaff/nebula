//! RAII guard for resource instances

/// RAII guard that wraps a resource instance.
///
/// When the guard is dropped, the on-drop callback is invoked (typically
/// returning the instance to the pool). Use `into_inner()` to take
/// ownership without triggering the callback.
///
/// The second type parameter `F` is the concrete callback type. It defaults
/// to `Box<dyn FnOnce(T) + Send>` so existing `Guard<T>` annotations continue
/// to compile, but pool internals can use the concrete closure type directly
/// to avoid a heap allocation on each acquire.
pub struct Guard<T, F: FnOnce(T) + Send + 'static = Box<dyn FnOnce(T) + Send>> {
    resource: Option<T>,
    on_drop: Option<F>,
}

impl<T, F: FnOnce(T) + Send + 'static> Guard<T, F> {
    /// Create a new guard wrapping `resource` with a drop callback.
    ///
    /// No heap allocation is performed; the callback is stored inline.
    pub fn new(resource: T, on_drop: F) -> Self {
        Self {
            resource: Some(resource),
            on_drop: Some(on_drop),
        }
    }

    /// Take the resource out of the guard, preventing the drop callback.
    #[must_use]
    pub fn into_inner(mut self) -> T {
        self.on_drop.take(); // prevent callback
        self.resource.take().expect("guard used after into_inner")
    }
}

impl<T, F: FnOnce(T) + Send + 'static> std::ops::Deref for Guard<T, F> {
    type Target = T;

    fn deref(&self) -> &T {
        self.resource.as_ref().expect("guard used after into_inner")
    }
}

impl<T, F: FnOnce(T) + Send + 'static> std::ops::DerefMut for Guard<T, F> {
    fn deref_mut(&mut self) -> &mut T {
        self.resource.as_mut().expect("guard used after into_inner")
    }
}

impl<T, F: FnOnce(T) + Send + 'static> Drop for Guard<T, F> {
    fn drop(&mut self) {
        if let (Some(resource), Some(on_drop)) = (self.resource.take(), self.on_drop.take()) {
            on_drop(resource);
        }
    }
}

impl<T: std::fmt::Debug, F: FnOnce(T) + Send + 'static> std::fmt::Debug for Guard<T, F> {
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
