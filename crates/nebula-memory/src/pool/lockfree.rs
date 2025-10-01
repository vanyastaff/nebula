//! Lock-free object pool implementation

#[cfg(not(feature = "std"))]
use alloc::boxed::Box;
#[cfg(not(feature = "std"))]
use alloc::sync::Arc;
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
#[cfg(feature = "std")]
use std::sync::Arc;

use super::{NoOpCallbacks, PoolCallbacks, PoolConfig, PoolStats, Poolable};
use crate::core::error::{MemoryError, MemoryResult};

/// Lock-free object pool using atomic operations
///
/// This implementation uses a lock-free stack (Treiber stack) for
/// high-performance concurrent access without mutex overhead.
///
/// # Example
/// ```
/// use std::sync::Arc;
/// use std::thread;
///
/// use nebula_memory::pool::LockFreePool;
///
/// let pool = Arc::new(LockFreePool::new(100, || Vec::<u8>::with_capacity(1024)));
///
/// // Spawn many threads
/// let handles: Vec<_> = (0..100)
///     .map(|_| {
///         let pool = pool.clone();
///         thread::spawn(move || {
///             for _ in 0..1000 {
///                 let buffer = pool.get().unwrap();
///                 // Use buffer...
///             }
///         })
///     })
///     .collect();
/// ```
pub struct LockFreePool<T: Poolable> {
    head: AtomicPtr<Node<T>>,
    factory: Arc<dyn Fn() -> T + Send + Sync>,
    config: PoolConfig,
    callbacks: Arc<dyn PoolCallbacks<T>>,
    size: AtomicUsize,
    #[cfg(feature = "stats")]
    stats: Arc<PoolStats>,
}

struct Node<T> {
    value: ManuallyDrop<T>,
    next: *mut Node<T>,
}

impl<T: Poolable> LockFreePool<T> {
    /// Create new lock-free pool
    pub fn new<F>(capacity: usize, factory: F) -> Self
    where F: Fn() -> T + Send + Sync + 'static {
        Self::with_config(PoolConfig { initial_capacity: capacity, ..Default::default() }, factory)
    }

    /// Create pool with custom configuration
    pub fn with_config<F>(config: PoolConfig, factory: F) -> Self
    where F: Fn() -> T + Send + Sync + 'static {
        // Сначала сохраним значение, которое нам понадобится после перемещения config
        let initial_capacity = config.initial_capacity;
        let pre_warm = config.pre_warm;

        let pool = Self {
            head: AtomicPtr::new(ptr::null_mut()),
            factory: Arc::new(factory),
            config,
            callbacks: Arc::new(NoOpCallbacks),
            size: AtomicUsize::new(0),
            #[cfg(feature = "stats")]
            stats: Arc::new(PoolStats::default()),
        };

        // Pre-warm pool if configured
        if pre_warm {
            for _ in 0..initial_capacity {
                let obj = (pool.factory)();
                #[cfg(feature = "stats")]
                pool.stats.record_creation();
                pool.push_node(obj);
            }
        }

        pool
    }

    /// Push object onto the lock-free stack
    fn push_node(&self, obj: T) {
        let node =
            Box::into_raw(Box::new(Node { value: ManuallyDrop::new(obj), next: ptr::null_mut() }));

        loop {
            let head = self.head.load(Ordering::Acquire);
            unsafe {
                (*node).next = head;
            }

            match self.head.compare_exchange_weak(head, node, Ordering::Release, Ordering::Acquire)
            {
                Ok(_) => {
                    self.size.fetch_add(1, Ordering::Relaxed);
                    break;
                },
                Err(_) => continue,
            }
        }
    }

    /// Pop object from the lock-free stack
    fn pop_node(&self) -> Option<T> {
        loop {
            let head = self.head.load(Ordering::Acquire);
            if head.is_null() {
                return None;
            }

            let next = unsafe { (*head).next };

            match self.head.compare_exchange_weak(head, next, Ordering::Release, Ordering::Acquire)
            {
                Ok(_) => {
                    self.size.fetch_sub(1, Ordering::Relaxed);
                    let node = unsafe { Box::from_raw(head) };
                    return Some(ManuallyDrop::into_inner(node.value));
                },
                Err(_) => continue,
            }
        }
    }

    /// Get object from pool
    pub fn get(&self) -> MemoryResult<LockFreePooledValue<T>> {
        #[cfg(feature = "stats")]
        self.stats.record_get();

        // Try to pop from stack
        if let Some(mut obj) = self.pop_node() {
            #[cfg(feature = "stats")]
            self.stats.record_hit();

            obj.reset();
            self.callbacks.on_checkout(&obj);

            return Ok(LockFreePooledValue { value: ManuallyDrop::new(obj), pool: self });
        }

        // Stack is empty, create new object
        #[cfg(feature = "stats")]
        self.stats.record_miss();

        // Check capacity
        if let Some(max) = self.config.max_capacity {
            #[cfg(feature = "stats")]
            let created = self.stats.total_created();
            #[cfg(not(feature = "stats"))]
            let created = self.size.load(Ordering::Relaxed);

            if created >= max {
                return Err(MemoryError::pool_exhausted());
            }
        }

        let obj = (self.factory)();
        self.callbacks.on_create(&obj);

        #[cfg(feature = "stats")]
        self.stats.record_creation();

        Ok(LockFreePooledValue { value: ManuallyDrop::new(obj), pool: self })
    }

    /// Try to get object without creating new one
    pub fn try_get(&self) -> Option<LockFreePooledValue<T>> {
        #[cfg(feature = "stats")]
        self.stats.record_get();

        self.pop_node().map(|mut obj| {
            #[cfg(feature = "stats")]
            self.stats.record_hit();

            obj.reset();
            self.callbacks.on_checkout(&obj);

            LockFreePooledValue { value: ManuallyDrop::new(obj), pool: self }
        })
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
                self.stats.record_destruction();
                return;
            }
        }

        // Check pool size limit
        if let Some(max) = self.config.max_capacity {
            if self.size.load(Ordering::Relaxed) >= max {
                self.callbacks.on_destroy(&obj);
                #[cfg(feature = "stats")]
                self.stats.record_destruction();
                return;
            }
        }

        obj.reset();
        self.push_node(obj);

        // Проверяем, нужна ли оптимизация памяти после возврата объекта
        #[cfg(feature = "adaptive")]
        {
            // Для lock-free пула мы не можем оптимизировать прямо здесь,
            // так как это заблокирует пул. Вместо этого проверяем пороговое значение
            // и запускаем оптимизацию только когда она действительно нужна.
            let current_size = self.size.load(Ordering::Relaxed);
            if let Some(max) = self.config.max_capacity {
                let usage_percent = (current_size * 100) / max;

                // Если превысили пороговое значение и на 5% выше - запускаем оптимизацию
                // Добавляем 5%, чтобы не запускать оптимизацию слишком часто
                if usage_percent >= self.config.pressure_threshold as usize + 5 {
                    // Запускаем оптимизацию в отдельном потоке, чтобы не блокировать текущий
                    #[cfg(feature = "std")]
                    {
                        use std::thread;
                        let pool_clone = self.clone();
                        thread::spawn(move || {
                            pool_clone.optimize_memory();
                        });
                    }

                    // В no_std среде просто вызываем напрямую, так как нет потоков
                    #[cfg(not(feature = "std"))]
                    {
                        self.optimize_memory();
                    }
                }
            }
        }
    }

    /// Get number of available objects
    pub fn available(&self) -> usize {
        self.size.load(Ordering::Relaxed)
    }

    /// Clear all objects (not thread-safe with concurrent operations)
    pub fn clear(&self) {
        while self.pop_node().is_some() {
            #[cfg(feature = "stats")]
            self.stats.record_destruction();
        }

        #[cfg(feature = "stats")]
        self.stats.record_clear();
    }

    /// Get pool statistics
    #[cfg(feature = "stats")]
    pub fn stats(&self) -> &PoolStats {
        &self.stats
    }

    /// Optimize memory usage by compressing objects
    ///
    /// This method attempts to reduce memory usage of all pooled objects
    /// by calling their `compress()` method when memory pressure is high.
    ///
    /// Note: This method temporarily locks the pool during optimization.
    ///
    /// Returns the amount of memory saved in bytes.
    #[cfg(feature = "adaptive")]
    pub fn optimize_memory(&self) -> usize {
        // Для оптимизации памяти в lock-free пуле нужно временно заблокировать доступ
        // Извлекаем все объекты из пула
        let mut nodes = Vec::new();
        let mut total_saved = 0;

        // Забираем все узлы из стека
        loop {
            let mut head = self.head.load(Ordering::Acquire);
            if head.is_null() {
                break;
            }

            // Пытаемся забрать весь стек сразу
            if self
                .head
                .compare_exchange(head, ptr::null_mut(), Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                // Успешно забрали весь стек
                let mut current = head;

                // Разбираем стек на отдельные объекты
                while !current.is_null() {
                    let node = unsafe { Box::from_raw(current) };
                    let next = node.next;
                    nodes.push(node);
                    current = next;
                }

                break;
            }
        }

        // Обновляем счетчик размера
        self.size.store(0, Ordering::Relaxed);

        // Оптимизируем все извлеченные объекты
        for node in &mut nodes {
            let before_size = unsafe { (*node.value).memory_usage() };

            #[cfg(feature = "adaptive")]
            let success = unsafe { (*node.value).compress() };
            #[cfg(not(feature = "adaptive"))]
            let success = false;

            let after_size = unsafe { (*node.value).memory_usage() };

            #[cfg(feature = "stats")]
            self.stats.record_compression_attempt(before_size, after_size, success);

            if before_size > after_size {
                total_saved += before_size - after_size;
            }
        }

        // Возвращаем объекты обратно в пул
        for mut node in nodes {
            let mut head = self.head.load(Ordering::Relaxed);
            loop {
                node.next = head;
                match self.head.compare_exchange(
                    head,
                    &mut *node,
                    Ordering::Release,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        // Успешно вернули узел в пул
                        self.size.fetch_add(1, Ordering::Relaxed);
                        // Предотвращаем drop для узла (теперь он в пуле)
                        core::mem::forget(node);
                        break;
                    },
                    Err(new_head) => {
                        // Кто-то изменил head, пробуем снова
                        head = new_head;
                    },
                }
            }
        }

        #[cfg(feature = "stats")]
        self.update_memory_stats();

        total_saved
    }

    /// Check if pool should optimize memory based on pressure threshold
    ///
    /// Returns true if optimization should be performed.
    #[cfg(feature = "adaptive")]
    pub fn should_optimize_memory(&self) -> bool {
        let current_size = self.size.load(Ordering::Relaxed);
        if current_size == 0 {
            return false;
        }

        // Проверяем порог заполнения пула
        if let Some(max) = self.config.max_capacity {
            let usage_percent = (current_size * 100) / max;
            usage_percent >= self.config.pressure_threshold as usize
        } else {
            // Для неограниченных пулов используем эвристику на основе текущего размера
            current_size > 1000
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

impl<T: Poolable> Drop for LockFreePool<T> {
    fn drop(&mut self) {
        // Clean up all nodes
        while let Some(obj) = self.pop_node() {
            self.callbacks.on_destroy(&obj);
        }
    }
}

/// RAII wrapper for lock-free pooled values
pub struct LockFreePooledValue<'a, T: Poolable> {
    value: ManuallyDrop<T>,
    pool: &'a LockFreePool<T>,
}

impl<'a, T: Poolable> LockFreePooledValue<'a, T> {
    /// Detach value from pool
    pub fn detach(mut self) -> T {
        let value = unsafe { ManuallyDrop::take(&mut self.value) };
        core::mem::forget(self);
        value
    }
}

impl<'a, T: Poolable> Deref for LockFreePooledValue<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<'a, T: Poolable> DerefMut for LockFreePooledValue<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<'a, T: Poolable> Drop for LockFreePooledValue<'a, T> {
    fn drop(&mut self) {
        let obj = unsafe { ManuallyDrop::take(&mut self.value) };
        self.pool.return_object(obj);
    }
}

impl<T: Poolable> Clone for LockFreePool<T> {
    fn clone(&self) -> Self {
        Self {
            head: AtomicPtr::new(self.head.load(Ordering::Relaxed)),
            factory: self.factory.clone(),
            config: self.config.clone(),
            callbacks: self.callbacks.clone(),
            size: AtomicUsize::new(self.size.load(Ordering::Relaxed)),
            #[cfg(feature = "stats")]
            stats: self.stats.clone(),
        }
    }
}

#[cfg(feature = "stats")]
impl<T: Poolable> LockFreePool<T> {
    fn update_memory_stats(&self) {
        // Approximate memory usage - can't iterate the stack safely
        let size = self.size.load(Ordering::Relaxed);
        let est_memory = size * core::mem::size_of::<Node<T>>() + size * core::mem::size_of::<T>();
        self.stats.update_memory(est_memory);
    }
}

// Safety: LockFreePool is already thread-safe and can be shared between threads
unsafe impl<T: Poolable + Send> Send for LockFreePool<T> {}
unsafe impl<T: Poolable + Send> Sync for LockFreePool<T> {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::*;

    #[test]
    fn test_lockfree_pool() {
        let pool = Arc::new(LockFreePool::new(10, || Vec::<u8>::with_capacity(1024)));
        let pool2 = pool.clone();

        let handle = thread::spawn(move || {
            for _ in 0..100 {
                let mut vec = pool2.get().unwrap();
                vec.extend_from_slice(&[1, 2, 3, 4, 5]);
            }
        });

        handle.join().unwrap();
        assert!(pool.available() > 0);
    }

    #[test]
    fn test_concurrent_access() {
        let pool = Arc::new(LockFreePool::new(5, || Vec::<u8>::with_capacity(1024)));

        let handles: Vec<_> = (0..20)
            .map(|i| {
                let pool = pool.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        let mut buffer = pool.get().unwrap();
                        buffer.push(i as u8);
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

        let pool = Arc::new(LockFreePool::with_config(config, || {
            let mut obj = CompressibleObject { data: Vec::with_capacity(1000), compressed: false };
            obj.data.extend_from_slice(&[1, 2, 3, 4, 5]);
            obj
        }));

        // Наполняем пул объектами
        for _ in 0..6 {
            let obj = pool.get().unwrap();
            drop(obj); // Объект вернется в пул
        }

        // Проверяем, что оптимизация должна быть выполнена
        assert!(pool.should_optimize_memory());

        // Вручную запускаем оптимизацию
        let saved = pool.optimize_memory();
        assert!(saved > 0);

        // Проверяем, что объекты были сжаты
        let obj = pool.get().unwrap();
        assert!(obj.compressed);
        assert!(obj.data.capacity() <= 100);
    }

    #[cfg(all(feature = "adaptive", feature = "std"))]
    #[test]
    fn test_automatic_optimization() {
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

        // Создаем пул с адаптивной конфигурацией и низким порогом
        let mut config = PoolConfig::bounded(10);
        #[cfg(feature = "adaptive")]
        {
            // Устанавливаем низкий порог, чтобы автоматическая оптимизация
            // гарантированно сработала
            config.pressure_threshold = 40;
        }

        let pool = Arc::new(LockFreePool::with_config(config, || {
            let mut obj = CompressibleObject { data: Vec::with_capacity(1000), compressed: false };
            obj.data.extend_from_slice(&[1, 2, 3, 4, 5]);
            obj
        }));

        // Наполняем пул объектами до превышения порога с запасом
        // (более 45%, чтобы учесть буфер +5% в реализации)
        for _ in 0..8 {
            let obj = pool.get().unwrap();
            drop(obj); // Объект вернется в пул
        }

        // Ждем немного, чтобы фоновый поток успел выполнить оптимизацию
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Проверяем, что объекты были сжаты
        let mut compressed_objects = 0;
        for _ in 0..pool.available() {
            if let Some(obj) = pool.try_get() {
                if obj.compressed && obj.data.capacity() <= 100 {
                    compressed_objects += 1;
                }
            }
        }

        // Хотя бы некоторые объекты должны быть сжаты
        assert!(compressed_objects > 0);
    }
}
