//! Async/await support for nebula-memory
//!
//! This module provides extension traits that allow integrating
//! the memory management system with async/await code.

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{
    boxed::Box,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
#[cfg(feature = "std")]
use std::{
    boxed::Box,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};

use crate::error::MemoryResult;
use crate::extensions::MemoryExtension;

/// A boxed future that can be stored and handled generically
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Create a boxed future from any future
pub fn boxed<'a, F, T>(future: F) -> BoxFuture<'a, T>
where F: Future<Output = T> + Send + 'a {
    Box::pin(future)
}

/// Trait for async allocation strategies
pub trait AsyncAllocator: Send + Sync {
    /// Allocate memory asynchronously
    fn allocate<'a>(&'a self, size: usize, align: usize) -> BoxFuture<'a, MemoryResult<usize>>;

    /// Deallocate memory asynchronously
    fn deallocate<'a>(
        &'a self,
        ptr: usize,
        size: usize,
        align: usize,
    ) -> BoxFuture<'a, MemoryResult<()>>;

    /// Get the current memory usage asynchronously
    fn usage<'a>(&'a self) -> BoxFuture<'a, MemoryResult<usize>>;
}

/// A no-op async allocator that always fails
pub struct NoopAsyncAllocator;

impl AsyncAllocator for NoopAsyncAllocator {
    fn allocate<'a>(&'a self, _size: usize, _align: usize) -> BoxFuture<'a, MemoryResult<usize>> {
        use crate::error::MemoryError;
        boxed(core::future::ready(Err(MemoryError::NotSupported {
            feature: "async allocation",
            context: Some("NoopAsyncAllocator does not support allocation".to_string()),
        })))
    }

    fn deallocate<'a>(
        &'a self,
        _ptr: usize,
        _size: usize,
        _align: usize,
    ) -> BoxFuture<'a, MemoryResult<()>> {
        use crate::error::MemoryError;
        boxed(core::future::ready(Err(MemoryError::NotSupported {
            feature: "async deallocation",
            context: Some("NoopAsyncAllocator does not support deallocation".to_string()),
        })))
    }

    fn usage<'a>(&'a self) -> BoxFuture<'a, MemoryResult<usize>> {
        boxed(core::future::ready(Ok(0)))
    }
}

/// A wrapper for a synchronous allocator that makes it work with async APIs
pub struct AsyncAllocatorWrapper<A> {
    allocator: A,
}

impl<A> AsyncAllocatorWrapper<A> {
    /// Create a new async allocator wrapper
    pub fn new(allocator: A) -> Self {
        Self { allocator }
    }

    /// Get a reference to the inner allocator
    pub fn inner(&self) -> &A {
        &self.allocator
    }
}

// Временная заглушка для демонстрационных целей
// В реальном коде этот трейт должен быть определен в crate::traits
pub trait GenericAllocator {
    fn allocate(&self, size: usize, align: usize) -> MemoryResult<*mut u8>;
    fn deallocate(&self, ptr: *mut u8, size: usize, align: usize) -> MemoryResult<()>;
    fn usage(&self) -> MemoryResult<usize>;
}

impl<A> AsyncAllocator for AsyncAllocatorWrapper<A>
where A: GenericAllocator + Send + Sync
{
    fn allocate<'a>(&'a self, size: usize, align: usize) -> BoxFuture<'a, MemoryResult<usize>> {
        let result = match self.allocator.allocate(size, align) {
            Ok(ptr) => Ok(ptr as usize),
            Err(e) => Err(e),
        };
        boxed(core::future::ready(result))
    }

    fn deallocate<'a>(
        &'a self,
        ptr: usize,
        size: usize,
        align: usize,
    ) -> BoxFuture<'a, MemoryResult<()>> {
        let result = self.allocator.deallocate(ptr as *mut u8, size, align);
        boxed(core::future::ready(result))
    }

    fn usage<'a>(&'a self) -> BoxFuture<'a, MemoryResult<usize>> {
        let result = self.allocator.usage();
        boxed(core::future::ready(result))
    }
}

/// Trait for async memory pools
pub trait AsyncPool<T>: Send + Sync {
    /// Get an object from the pool asynchronously
    fn acquire<'a>(&'a self) -> BoxFuture<'a, MemoryResult<T>>;

    /// Return an object to the pool asynchronously
    fn release<'a>(&'a self, value: T) -> BoxFuture<'a, MemoryResult<()>>;

    /// Get the number of available objects in the pool asynchronously
    fn available<'a>(&'a self) -> BoxFuture<'a, MemoryResult<usize>>;

    /// Get the total capacity of the pool asynchronously
    fn capacity<'a>(&'a self) -> BoxFuture<'a, MemoryResult<usize>>;
}

/// A wrapper for a synchronous pool that makes it work with async APIs
pub struct AsyncPoolWrapper<P, T> {
    pool: P,
    _phantom: core::marker::PhantomData<T>,
}

impl<P, T> AsyncPoolWrapper<P, T> {
    /// Create a new async pool wrapper
    pub fn new(pool: P) -> Self {
        Self { pool, _phantom: core::marker::PhantomData }
    }

    /// Get a reference to the inner pool
    pub fn inner(&self) -> &P {
        &self.pool
    }
}

// Временная заглушка для демонстрационных целей
// В реальном коде этот трейт должен быть определен в crate::traits
pub trait GenericPool<T> {
    fn acquire(&self) -> MemoryResult<T>;
    fn release(&self, value: T) -> MemoryResult<()>;
    fn available(&self) -> usize;
    fn capacity(&self) -> usize;
}

impl<P, T> AsyncPool<T> for AsyncPoolWrapper<P, T>
where
    P: GenericPool<T> + Send + Sync,
    T: Send + Sync + 'static,
{
    fn acquire<'a>(&'a self) -> BoxFuture<'a, MemoryResult<T>> {
        let result = self.pool.acquire();
        boxed(core::future::ready(result))
    }

    fn release<'a>(&'a self, value: T) -> BoxFuture<'a, MemoryResult<()>> {
        let result = self.pool.release(value);
        boxed(core::future::ready(result))
    }

    fn available<'a>(&'a self) -> BoxFuture<'a, MemoryResult<usize>> {
        let result = Ok(self.pool.available());
        boxed(core::future::ready(result))
    }

    fn capacity<'a>(&'a self) -> BoxFuture<'a, MemoryResult<usize>> {
        let result = Ok(self.pool.capacity());
        boxed(core::future::ready(result))
    }
}

/// Async memory extension
pub struct AsyncExtension {
    /// Whether async support is enabled
    enabled: bool,
    /// Name of the executor being used
    executor_name: String,
}

impl AsyncExtension {
    /// Create a new async extension
    pub fn new(executor_name: impl Into<String>) -> Self {
        Self { enabled: true, executor_name: executor_name.into() }
    }

    /// Check if async support is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get the name of the executor being used
    pub fn executor_name(&self) -> &str {
        &self.executor_name
    }

    /// Enable async support
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disable async support
    pub fn disable(&mut self) {
        self.enabled = false;
    }
}

impl Default for AsyncExtension {
    fn default() -> Self {
        Self { enabled: true, executor_name: "unknown".to_string() }
    }
}

impl MemoryExtension for AsyncExtension {
    fn name(&self) -> &str {
        "async"
    }

    fn version(&self) -> &str {
        "1.0.0"
    }

    fn category(&self) -> &str {
        "async"
    }

    fn tags(&self) -> Vec<&str> {
        vec!["async", "concurrency"]
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

/// Helper to get the current global async extension
pub fn global_async() -> Option<Arc<AsyncExtension>> {
    use crate::extensions::GlobalExtensions;

    if let Some(ext) = GlobalExtensions::get("async") {
        if let Some(async_ext) = ext.as_any().downcast_ref::<AsyncExtension>() {
            // Создаем новый экземпляр с теми же параметрами
            return Some(Arc::new(AsyncExtension {
                enabled: async_ext.enabled,
                executor_name: async_ext.executor_name.clone(),
            }));
        }
    }
    None
}

/// Initialize the global async extension
pub fn init_global_async(executor_name: impl Into<String>) -> MemoryResult<()> {
    use crate::extensions::GlobalExtensions;

    let extension = AsyncExtension::new(executor_name);
    GlobalExtensions::register(extension)
}

/// Check if async support is enabled globally
pub fn is_async_enabled() -> bool {
    global_async().map(|ext| ext.is_enabled()).unwrap_or(false)
}

/// A task that can be spawned on an async runtime
pub trait Task: Future + Send + 'static {
    /// Get the name of the task
    fn name(&self) -> &str;
}

/// A simple task implementation
pub struct SimpleTask<F, T> {
    name: &'static str,
    future: F,
    _phantom: core::marker::PhantomData<T>,
}

impl<F, T> SimpleTask<F, T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    /// Create a new simple task
    pub fn new(name: &'static str, future: F) -> Self {
        Self { name, future, _phantom: core::marker::PhantomData }
    }
}

impl<F, T> Task for SimpleTask<F, T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    fn name(&self) -> &str {
        self.name
    }
}

impl<F, T> Future for SimpleTask<F, T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: Pin projection to poll inner future.
        // - self is Pin<&mut Self>, guaranteed not to move
        // - get_unchecked_mut accesses self.future field
        // - future field is !Unpin (Box<dyn Future>)
        // - Pin::new_unchecked safe because self's pin guarantee transfers to field
        // - Not moving future out, only polling in place
        unsafe {
            let future = &mut self.as_mut().get_unchecked_mut().future;
            Pin::new_unchecked(future).poll(cx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper function to run a future to completion synchronously
    // This is for testing only - in a real application you'd use a proper async
    // runtime
    pub fn block_on<F: Future>(future: F) -> F::Output {
        use core::task::{RawWaker, RawWakerVTable, Waker};

        // Create a simple waker that does nothing
        fn dummy_raw_waker() -> RawWaker {
            fn no_op(_: *const ()) {}
            fn clone(_: *const ()) -> RawWaker {
                dummy_raw_waker()
            }

            let vtable = &RawWakerVTable::new(clone, no_op, no_op, no_op);
            RawWaker::new(core::ptr::null(), vtable)
        }

        // SAFETY: Creating Waker from RawWaker in test helper.
        // - dummy_raw_waker() creates valid RawWaker with proper vtable
        // - vtable functions (clone, no_op) are valid for null data pointer
        // - Waker is only used within this function scope (no leaks)
        // - This is test code (block_on helper for tests)
        let waker = unsafe { Waker::from_raw(dummy_raw_waker()) };
        let mut cx = Context::from_waker(&waker);

        // Pin the future
        let mut future = Pin::from(Box::new(future));

        // Poll the future until it's ready
        loop {
            match future.as_mut().poll(&mut cx) {
                Poll::Ready(output) => return output,
                Poll::Pending => {},
            }
        }
    }

    #[test]
    fn test_async_wrapper() {
        struct TestAllocator;

        impl GenericAllocator for TestAllocator {
            fn allocate(&self, _size: usize, _align: usize) -> MemoryResult<*mut u8> {
                // This is just a test, don't actually allocate
                Ok(core::ptr::null_mut())
            }

            fn deallocate(&self, _ptr: *mut u8, _size: usize, _align: usize) -> MemoryResult<()> {
                Ok(())
            }

            fn usage(&self) -> MemoryResult<usize> {
                Ok(0)
            }
        }

        let allocator = TestAllocator;
        let async_allocator = AsyncAllocatorWrapper::new(allocator);

        let result = block_on(async_allocator.allocate(1024, 8));
        assert!(result.is_ok());

        // Используем корректный тип (usize) для указателя
        let ptr = 0usize; // нулевой указатель как usize
        let result = block_on(async_allocator.deallocate(ptr, 1024, 8));
        assert!(result.is_ok());

        let result = block_on(async_allocator.usage());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_simple_task() {
        async fn test_future() -> u32 {
            42
        }

        let task = SimpleTask::new("test_task", test_future());
        assert_eq!(task.name(), "test_task");

        let result = block_on(task);
        assert_eq!(result, 42);
    }
}
