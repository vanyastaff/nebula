//! Thread-safe object pool implementation

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};
#[cfg(feature = "std")]
use std::sync::{Arc, Condvar, Mutex};
#[cfg(feature = "std")]
use std::time::Duration;

#[cfg(not(feature = "std"))]
use spin::{Condvar, Mutex};

#[cfg(feature = "stats")]
use super::PoolStats;
use super::{NoOpCallbacks, PoolCallbacks, PoolConfig, Poolable};
use crate::error::{MemoryError, MemoryResult};

/// Thread-safe object pool using mutex
///
/// # Example
/// ```
/// use std::sync::Arc;
/// use std::thread;
///
/// use nebula_memory::pool::ThreadSafePool;
///
/// let pool = Arc::new(ThreadSafePool::new(100, || Vec::<u8>::with_capacity(1024)));
///
/// let handles: Vec<_> = (0..10)
///     .map(|_| {
///         let pool = pool.clone();
///         thread::spawn(move || {
///             let mut buffer = pool.get().unwrap();
///             buffer.extend_from_slice(b"Hello");
///         })
///     })
///     .collect();
///
/// for h in handles {
///     h.join().unwrap();
/// }
/// ```
pub struct ThreadSafePool<T: Poolable> {
    inner: Mutex<PoolInner<T>>,
    factory: Arc<dyn Fn() -> T + Send + Sync>,
    config: PoolConfig,
    callbacks: Arc<dyn PoolCallbacks<T>>,
    #[cfg(feature = "std")]
    not_empty: Condvar,
    #[cfg(feature = "stats")]
    stats: Arc<PoolStats>,
}

struct PoolInner<T> {
    objects: Vec<T>,
    shutdown: bool,
}

impl<T: Poolable> ThreadSafePool<T> {
    /// Create new thread-safe pool
    pub fn new<F>(capacity: usize, factory: F) -> Self
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        Self::with_config(
            PoolConfig {
                initial_capacity: capacity,
                ..Default::default()
            },
            factory,
        )
    }

    /// Create pool with custom configuration
    pub fn with_config<F>(config: PoolConfig, factory: F) -> Self
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        let mut objects = Vec::with_capacity(config.initial_capacity);

        #[cfg(feature = "stats")]
        let stats = Arc::new(PoolStats::default());

        // Pre-warm pool if configured
        if config.pre_warm {
            for _ in 0..config.initial_capacity {
                let obj = factory();
                #[cfg(feature = "stats")]
                stats.record_creation();
                objects.push(obj);
            }
        }

        Self {
            inner: Mutex::new(PoolInner {
                objects,
                shutdown: false,
            }),
            factory: Arc::new(factory),
            config,
            callbacks: Arc::new(NoOpCallbacks),
            #[cfg(feature = "std")]
            not_empty: Condvar::new(),
            #[cfg(feature = "stats")]
            stats,
        }
    }

    /// Set callbacks for pool events
    #[must_use = "builder methods must be chained or built"]
    pub fn with_callbacks<C: PoolCallbacks<T> + 'static>(mut self, callbacks: C) -> Self {
        self.callbacks = Arc::new(callbacks);
        self
    }

    /// Get object from pool
    pub fn get(&self) -> MemoryResult<ThreadSafePooledValue<'_, T>> {
        self.get_timeout(None)
    }

    /// Try to get object without blocking
    pub fn try_get(&self) -> Option<ThreadSafePooledValue<'_, T>> {
        #[cfg(feature = "stats")]
        self.stats.record_get();

        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(), // Восстанавливаемся после отравления
        };

        if inner.shutdown {
            return None;
        }

        inner.objects.pop().map(|mut obj| {
            #[cfg(feature = "stats")]
            self.stats.record_hit();

            drop(inner); // Release lock early

            obj.reset();
            self.callbacks.on_checkout(&obj);

            ThreadSafePooledValue {
                value: ManuallyDrop::new(obj),
                pool: self,
            }
        })
    }

    /// Get object with timeout
    #[cfg(feature = "std")]
    pub fn get_timeout(&self, timeout: Option<Duration>) -> MemoryResult<ThreadSafePooledValue<'_, T>> {
        #[cfg(feature = "stats")]
        self.stats.record_get();

        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(), // Восстанавливаемся после отравления
        };

        // Fast path - object available
        if let Some(mut obj) = inner.objects.pop() {
            #[cfg(feature = "stats")]
            self.stats.record_hit();

            drop(inner); // Release lock early

            obj.reset();
            self.callbacks.on_checkout(&obj);

            return Ok(ThreadSafePooledValue {
                value: ManuallyDrop::new(obj),
                pool: self,
            });
        }

        // Check if we can create new object
        if let Some(max) = self.config.max_capacity {
            #[cfg(feature = "stats")]
            let created = self.stats.total_created();
            #[cfg(not(feature = "stats"))]
            let created = 0;

            if created >= max {
                // Need to wait for object to be returned
                if let Some(timeout) = timeout {
                    let start = std::time::Instant::now();
                    while inner.objects.is_empty() && !inner.shutdown {
                        let remaining = timeout.saturating_sub(start.elapsed());
                        if remaining.is_zero() {
                            return Err(MemoryError::pool_exhausted("pool", 0));
                        }

                        // Корректная обработка результата wait_timeout
                        let wait_result = self.not_empty.wait_timeout(inner, remaining);
                        inner = match wait_result {
                            Ok((guard, _)) => guard,
                            Err(poison) => poison.into_inner().0, // Берем первый элемент из кортежа
                        };
                    }

                    if inner.shutdown {
                        return Err(MemoryError::pool_exhausted("pool", 0));
                    }

                    if let Some(mut obj) = inner.objects.pop() {
                        #[cfg(feature = "stats")]
                        self.stats.record_hit();

                        drop(inner);

                        obj.reset();
                        self.callbacks.on_checkout(&obj);

                        return Ok(ThreadSafePooledValue {
                            value: ManuallyDrop::new(obj),
                            pool: self,
                        });
                    }
                }

                return Err(MemoryError::pool_exhausted("pool", 0));
            }
        }

        // Create new object
        drop(inner); // Release lock for creation

        #[cfg(feature = "stats")]
        self.stats.record_miss();

        let obj = (self.factory)();
        self.callbacks.on_create(&obj);

        #[cfg(feature = "stats")]
        {
            self.stats.record_creation();
            self.update_memory_stats();
        }

        Ok(ThreadSafePooledValue {
            value: ManuallyDrop::new(obj),
            pool: self,
        })
    }

    /// Get object with timeout (no-std version)
    #[cfg(not(feature = "std"))]
    pub fn get_timeout(
        &self,
        _timeout: Option<Duration>,
    ) -> MemoryResult<ThreadSafePooledValue<T>> {
        // Without std, we can't do timed waits
        self.try_get().ok_or(MemoryError::pool_exhausted("pool", 0))
    }

    /// Return object to pool
    fn return_object(&self, mut obj: T) {
        #[cfg(feature = "stats")]
        self.stats.record_return();

        self.callbacks.on_checkin(&obj);

        // Validate object if configured
        if self.config.validate_on_return {
            if !obj.validate() || !obj.is_reusable() {
                self.callbacks.on_destroy(&obj);
                #[cfg(feature = "stats")]
                {
                    self.stats.record_destruction();
                    self.update_memory_stats();
                }
                return;
            }
        }

        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        };

        if inner.shutdown {
            self.callbacks.on_destroy(&obj);
            return;
        }

        // Check pool size limit
        if let Some(max) = self.config.max_capacity {
            if inner.objects.len() >= max {
                drop(inner);
                self.callbacks.on_destroy(&obj);
                #[cfg(feature = "stats")]
                {
                    self.stats.record_destruction();
                    self.update_memory_stats();
                }
                return;
            }
        }

        obj.reset();
        inner.objects.push(obj);

        #[cfg(feature = "std")]
        self.not_empty.notify_one();

        #[cfg(feature = "stats")]
        self.update_memory_stats();

        // Проверяем, нужна ли оптимизация памяти, и если да, проводим её
        #[cfg(feature = "adaptive")]
        {
            // Проверяем порог заполнения пула
            let should_optimize = if let Some(max) = self.config.max_capacity {
                let usage_percent = (inner.objects.len() * 100) / max;
                usage_percent >= self.config.pressure_threshold as usize
            } else {
                // Для неограниченных пулов используем эвристику на основе текущего размера
                inner.objects.len() > 1000 && inner.objects.capacity() > inner.objects.len() * 2
            };

            // Если нужно оптимизировать, делаем это прямо в текущем лок-гварде
            if should_optimize && !inner.shutdown {
                for obj in inner.objects.iter_mut() {
                    let before_size = obj.memory_usage();
                    let success = obj.compress();
                    let after_size = obj.memory_usage();

                    #[cfg(feature = "stats")]
                    self.stats
                        .record_compression_attempt(before_size, after_size, success);
                }

                #[cfg(feature = "stats")]
                if !inner.objects.is_empty() {
                    let total_memory = inner.objects.iter().map(|o| o.memory_usage()).sum();
                    self.stats.update_memory(total_memory);
                }
            }
        }
    }

    /// Clear all pooled objects
    pub fn clear(&self) {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        };

        for obj in inner.objects.drain(..) {
            self.callbacks.on_destroy(&obj);
        }

        #[cfg(feature = "stats")]
        {
            self.stats.record_clear();
            self.update_memory_stats();
        }
    }

    /// Shutdown pool
    pub fn shutdown(&self) {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        };
        inner.shutdown = true;

        for obj in inner.objects.drain(..) {
            self.callbacks.on_destroy(&obj);
        }

        #[cfg(feature = "std")]
        self.not_empty.notify_all();
    }

    /// Get number of available objects
    pub fn available(&self) -> usize {
        match self.inner.lock() {
            Ok(guard) => guard.objects.len(),
            Err(poison) => poison.into_inner().objects.len(),
        }
    }

    /// Get pool statistics
    #[cfg(feature = "stats")]
    pub fn stats(&self) -> &PoolStats {
        &self.stats
    }

    /// Update memory statistics
    #[cfg(feature = "stats")]
    fn update_memory_stats(&self) {
        let inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        };
        let memory = inner
            .objects
            .iter()
            .map(|obj| obj.memory_usage())
            .sum::<usize>()
            + inner.objects.capacity() * core::mem::size_of::<T>();

        self.stats.update_memory(memory);
    }

    /// Optimize memory usage by compressing objects
    ///
    /// This method attempts to reduce memory usage of all pooled objects
    /// by calling their `compress()` method when memory pressure is high.
    ///
    /// Returns the amount of memory saved in bytes.
    #[cfg(feature = "adaptive")]
    pub fn optimize_memory(&self) -> usize {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        };
        if inner.shutdown {
            return 0;
        }

        let mut total_saved = 0;

        for obj in inner.objects.iter_mut() {
            let before_size = obj.memory_usage();
            let success = obj.compress();
            let after_size = obj.memory_usage();

            #[cfg(feature = "stats")]
            self.stats
                .record_compression_attempt(before_size, after_size, success);

            if before_size > after_size {
                total_saved += before_size - after_size;
            }
        }

        #[cfg(feature = "stats")]
        if !inner.objects.is_empty() {
            let total_memory = inner.objects.iter().map(|o| o.memory_usage()).sum();
            self.stats.update_memory(total_memory);
        }

        total_saved
    }

    /// Check if pool should optimize memory based on pressure threshold
    ///
    /// Returns true if optimization should be performed.
    #[cfg(feature = "adaptive")]
    pub fn should_optimize_memory(&self) -> bool {
        let inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        };
        if inner.shutdown || inner.objects.is_empty() {
            return false;
        }

        // Check against pressure threshold from config
        if let Some(max) = self.config.max_capacity {
            let usage_percent = (inner.objects.len() * 100) / max;
            usage_percent >= self.config.pressure_threshold as usize
        } else {
            // For unbounded pools, use a heuristic based on current size
            inner.objects.len() > 1000 && inner.objects.capacity() > inner.objects.len() * 2
        }
    }

    /// Try to optimize memory if needed
    ///
    /// Returns amount of memory saved, or 0 if optimization wasn't needed.
    #[cfg(feature = "adaptive")]
    pub fn try_optimize_memory(&self) -> usize {
        if self.should_optimize_memory() {
            self.optimize_memory()
        } else {
            0
        }
    }
}

/// RAII wrapper for thread-safe pooled values
pub struct ThreadSafePooledValue<'a, T: Poolable> {
    value: ManuallyDrop<T>,
    pool: &'a ThreadSafePool<T>,
}

impl<'a, T: Poolable> ThreadSafePooledValue<'a, T> {
    /// Detach value from pool
    pub fn detach(mut self) -> T {
        let value = unsafe { ManuallyDrop::take(&mut self.value) };
        core::mem::forget(self);
        value
    }
}

impl<'a, T: Poolable> Deref for ThreadSafePooledValue<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<'a, T: Poolable> DerefMut for ThreadSafePooledValue<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<'a, T: Poolable> Drop for ThreadSafePooledValue<'a, T> {
    fn drop(&mut self) {
        let obj = unsafe { ManuallyDrop::take(&mut self.value) };
        self.pool.return_object(obj);
    }
}

// ThreadSafePool is Send + Sync if T is Send
unsafe impl<T: Poolable + Send> Send for ThreadSafePool<T> {}
unsafe impl<T: Poolable + Send> Sync for ThreadSafePool<T> {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::*;

    #[derive(Debug)]
    struct TestObject {
        value: i32,
    }

    impl Poolable for TestObject {
        fn reset(&mut self) {
            self.value = 0;
        }
    }

    #[test]
    fn test_thread_safe_pool() {
        let pool = Arc::new(ThreadSafePool::new(10, || TestObject { value: 42 }));
        let pool2 = pool.clone();

        let handle = thread::spawn(move || {
            let mut obj = pool2.get().unwrap();
            obj.value = 100;
        });

        handle.join().unwrap();

        // Object should be returned
        assert!(pool.available() > 0);
    }

    #[test]
    fn test_concurrent_access() {
        let pool = Arc::new(ThreadSafePool::new(5, || vec![0u8; 1024]));

        let handles: Vec<_> = (0..20)
            .map(|i| {
                let pool = pool.clone();
                thread::spawn(move || {
                    for _ in 0..10 {
                        let mut buffer = pool.get().unwrap();
                        buffer[0] = i as u8;
                        thread::yield_now();
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    #[cfg(feature = "adaptive")]
    #[test]
    fn test_memory_optimization() {
        // Создаем тестовый тип, поддерживающий сжатие
        struct CompressibleObject {
            data: Vec<u8>,
            compressed: bool,
        }

        impl Poolable for CompressibleObject {
            fn reset(&mut self) {
                self.data.clear();
                self.compressed = false;
            }

            fn memory_usage(&self) -> usize {
                self.data.capacity() + core::mem::size_of::<Self>()
            }

            #[cfg(feature = "adaptive")]
            fn compress(&mut self) -> bool {
                if !self.compressed && self.data.capacity() > 100 {
                    // Симулируем сжатие, уменьшая ёмкость
                    let mut new_data = Vec::with_capacity(100);
                    new_data.extend_from_slice(&self.data);
                    self.data = new_data;
                    self.compressed = true;
                    true
                } else {
                    false
                }
            }
        }

        // Создаем пул с адаптивной конфигурацией
        let mut config = PoolConfig::bounded(10);
        #[cfg(feature = "adaptive")]
        {
            config.pressure_threshold = 50; // 50% заполнения вызывает
            // оптимизацию
        }

        let pool = Arc::new(ThreadSafePool::with_config(config, || {
            let mut obj = CompressibleObject {
                data: Vec::with_capacity(1000),
                compressed: false,
            };
            obj.data.extend_from_slice(&[1, 2, 3, 4, 5]);
            obj
        }));

        // Создаем множество потоков, которые будут работать с пулом
        let handles: Vec<_> = (0..5)
            .map(|_| {
                let pool = pool.clone();
                thread::spawn(move || {
                    for _ in 0..10 {
                        let mut obj = pool.get().unwrap();
                        obj.data.push(10);
                        thread::yield_now();
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        // После выполнения всех потоков, пул должен быть заполнен выше порогового
        // значения и автоматическая оптимизация должна была сработать
        let obj = pool.get().unwrap();
        assert!(obj.compressed);
        assert!(obj.data.capacity() <= 100);
    }

    #[cfg(feature = "adaptive")]
    #[test]
    fn test_manual_optimization() {
        // Создаем тестовый тип, поддерживающий сжатие
        struct CompressibleObject {
            data: Vec<u8>,
            compressed: bool,
        }

        impl Poolable for CompressibleObject {
            fn reset(&mut self) {
                self.data.clear();
                self.compressed = false;
            }

            fn memory_usage(&self) -> usize {
                self.data.capacity() + core::mem::size_of::<Self>()
            }

            #[cfg(feature = "adaptive")]
            fn compress(&mut self) -> bool {
                if !self.compressed && self.data.capacity() > 100 {
                    // Симулируем сжатие, уменьшая ёмкость
                    let mut new_data = Vec::with_capacity(100);
                    new_data.extend_from_slice(&self.data);
                    self.data = new_data;
                    self.compressed = true;
                    true
                } else {
                    false
                }
            }
        }

        // Создаем пул с высоким порогом, чтобы автоматическая оптимизация не
        // срабатывала
        let mut config = PoolConfig::bounded(20);
        #[cfg(feature = "adaptive")]
        {
            config.pressure_threshold = 90; // Высокий порог
        }

        let pool = Arc::new(ThreadSafePool::with_config(config, || {
            let mut obj = CompressibleObject {
                data: Vec::with_capacity(1000),
                compressed: false,
            };
            obj.data.extend_from_slice(&[1, 2, 3, 4, 5]);
            obj
        }));

        // Наполняем пул объектами
        for _ in 0..10 {
            let obj = pool.get().unwrap();
            drop(obj);
        }

        // Вручную вызываем оптимизацию
        let saved = pool.optimize_memory();
        assert!(saved > 0);

        // Проверяем, что объекты в пуле были сжаты
        let obj = pool.get().unwrap();
        assert!(obj.compressed);
        assert!(obj.data.capacity() <= 100);
    }
}
