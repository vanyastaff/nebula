//! RAII guard for resource instances

/// RAII guard that wraps a resource instance.
///
/// When the guard is dropped, the on-drop callback is invoked (typically
/// returning the instance to the pool). Use [`into_inner`](Self::into_inner)
/// to take ownership without triggering the callback, [`detach`](Self::detach)
/// to extract the instance while returning the pool semaphore permit, or
/// [`leak`](Self::leak) to extract the instance without any pool notification.
///
/// The second type parameter `F` is the concrete callback type. It defaults
/// to `Box<dyn FnOnce(T) + Send>` so existing `Guard<T>` annotations continue
/// to compile, but pool internals can use the concrete closure type directly
/// to avoid a heap allocation on each acquire.
pub struct Guard<T, F: FnOnce(T, bool) + Send + 'static = Box<dyn FnOnce(T, bool) + Send>> {
    resource: Option<T>,
    on_drop: Option<F>,
    tainted: bool,
    /// Optional callback invoked when [`detach`](Self::detach) is called.
    ///
    /// Signals the originating pool to return the semaphore permit so it can
    /// create a replacement instance.  `None` for guards not backed by a pool
    /// (e.g. test sentinels).
    on_detach: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl<T, F: FnOnce(T, bool) + Send + 'static> Guard<T, F> {
    /// Create a new guard wrapping `resource` with a drop callback.
    ///
    /// No heap allocation is performed; the callback is stored inline.
    pub fn new(resource: T, on_drop: F) -> Self {
        Self {
            resource: Some(resource),
            on_drop: Some(on_drop),
            tainted: false,
            on_detach: None,
        }
    }

    /// Attach a permit-return callback used by [`detach`](Self::detach).
    ///
    /// Called by the pool after construction to wire in the semaphore-return
    /// logic without reifying it into the main on-drop closure.
    pub(crate) fn set_detach_callback(&mut self, f: impl FnOnce() + Send + 'static) {
        self.on_detach = Some(Box::new(f));
    }

    /// Mark this instance as tainted so pool return skips recycle.
    pub fn taint(&mut self) {
        self.tainted = true;
    }

    /// Returns true if this guard was marked tainted.
    #[must_use]
    pub fn is_tainted(&self) -> bool {
        self.tainted
    }

    /// Take the resource out of the guard, preventing the drop callback.
    #[must_use]
    pub fn into_inner(mut self) -> T {
        self.on_drop.take(); // prevent callback
        self.on_detach.take();
        self.resource.take().expect("guard used after into_inner")
    }

    /// Extract the instance and return the semaphore permit to the pool.
    ///
    /// Unlike [`into_inner`](Self::into_inner), this signals the pool that the
    /// permit has been relinquished so it can create a replacement instance.
    /// The instance itself is owned by the caller indefinitely — the pool no
    /// longer tracks it.
    ///
    /// Use this for long-lived captures such as `TriggerAction`, PostgreSQL
    /// `LISTEN` connections, or SSH port-forwarding sessions where the resource
    /// must outlive the standard acquire/release cycle.
    ///
    /// If the guard was not created by a pool (no `on_detach` callback), this
    /// is equivalent to [`into_inner`](Self::into_inner).
    #[must_use]
    pub fn detach(mut self) -> T {
        self.on_drop.take(); // cancel normal return path
        if let Some(f) = self.on_detach.take() {
            f(); // return semaphore permit → pool can spawn a replacement
        }
        self.resource.take().expect("guard used after detach")
    }

    /// Extract the instance without notifying the pool.
    ///
    /// Both the recycle callback **and** the permit-return callback are
    /// suppressed. The pool's logical size shrinks by one, and no replacement
    /// instance is created. Use when the instance transitions to a mode that is
    /// fundamentally incompatible with pooling (e.g. handing ownership to an
    /// external library that manages its own lifetime).
    #[must_use]
    pub fn leak(mut self) -> T {
        self.on_drop.take();
        self.on_detach.take();
        self.resource.take().expect("guard used after leak")
    }
}

impl<T, F: FnOnce(T, bool) + Send + 'static> std::ops::Deref for Guard<T, F> {
    type Target = T;

    fn deref(&self) -> &T {
        self.resource.as_ref().expect("guard used after into_inner")
    }
}

impl<T, F: FnOnce(T, bool) + Send + 'static> std::ops::DerefMut for Guard<T, F> {
    fn deref_mut(&mut self) -> &mut T {
        self.resource.as_mut().expect("guard used after into_inner")
    }
}

impl<T, F: FnOnce(T, bool) + Send + 'static> Drop for Guard<T, F> {
    fn drop(&mut self) {
        // on_detach is only set if detach() was NOT called (detach() takes it).
        // Drop it silently — the permit was either already returned by detach()
        // or the guard is being recycled via on_drop.
        let _ = self.on_detach.take();
        if let (Some(resource), Some(on_drop)) = (self.resource.take(), self.on_drop.take()) {
            on_drop(resource, self.tainted);
        }
    }
}

impl<T: std::fmt::Debug, F: FnOnce(T, bool) + Send + 'static> std::fmt::Debug for Guard<T, F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guard")
            .field("resource", &self.resource)
            .field("tainted", &self.tainted)
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
        let guard = Guard::new(42u32, |_, _| {});
        assert_eq!(*guard, 42);
    }

    #[test]
    fn guard_drop_fires_callback() {
        let called = Arc::new(AtomicBool::new(false));
        let called_c = called.clone();
        let guard = Guard::new("hello", move |_, _| {
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
        let guard = Guard::new(99u32, move |_, _| {
            called_c.store(true, Ordering::SeqCst);
        });
        let val = guard.into_inner();
        assert_eq!(val, 99);
        assert!(!called.load(Ordering::SeqCst));
    }

    #[test]
    fn guard_deref_mut() {
        let mut guard = Guard::new(String::from("hello"), |_, _| {});
        guard.push_str(" world");
        assert_eq!(*guard, "hello world");
    }

    #[test]
    fn guard_taint_marks_flag() {
        let mut guard = Guard::new(7u32, |_, _| {});
        assert!(!guard.is_tainted());
        guard.taint();
        assert!(guard.is_tainted());
    }

    #[test]
    fn guard_detach_fires_on_detach_not_on_drop() {
        let drop_called = Arc::new(AtomicBool::new(false));
        let detach_called = Arc::new(AtomicBool::new(false));

        let drop_c = drop_called.clone();
        let detach_c = detach_called.clone();

        let mut guard = Guard::new(42u32, move |_, _| {
            drop_c.store(true, Ordering::SeqCst);
        });
        guard.set_detach_callback(move || {
            detach_c.store(true, Ordering::SeqCst);
        });

        let val = guard.detach();
        assert_eq!(val, 42);
        assert!(detach_called.load(Ordering::SeqCst), "on_detach must fire");
        assert!(!drop_called.load(Ordering::SeqCst), "on_drop must NOT fire");
    }

    #[test]
    fn guard_leak_fires_neither_callback() {
        let drop_called = Arc::new(AtomicBool::new(false));
        let detach_called = Arc::new(AtomicBool::new(false));

        let drop_c = drop_called.clone();
        let detach_c = detach_called.clone();

        let mut guard = Guard::new(99u32, move |_, _| {
            drop_c.store(true, Ordering::SeqCst);
        });
        guard.set_detach_callback(move || {
            detach_c.store(true, Ordering::SeqCst);
        });

        let val = guard.leak();
        assert_eq!(val, 99);
        assert!(!drop_called.load(Ordering::SeqCst), "on_drop must NOT fire");
        assert!(
            !detach_called.load(Ordering::SeqCst),
            "on_detach must NOT fire"
        );
    }
}
