use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use dyn_clone::{DynClone, clone_trait_object};
use std::fmt;

// ====================== Базовые трейты ======================

/// Унифицированный трейт для управляемых объектов
///
/// Определяет интерфейс для объектов, которыми может управлять пул.
/// Объекты должны поддерживать сброс в начальное состояние, валидацию
/// и операции жизненного цикла.
pub trait ManagedObject: DynClone + Send + Sync + 'static {
    /// Сбрасывает объект в начальное состояние
    ///
    /// Должен восстанавливать объект в состояние, аналогичное
    /// только что созданному, сохраняя при этом выделенные ресурсы.
    fn reset(&mut self);

    /// Возвращает размер объекта в байтах
    ///
    /// Используется для контроля памяти в пуле объектов.
    /// По умолчанию возвращает размер структуры плюс внутренние буферы.
    #[inline]
    fn size_bytes(&self) -> usize {
        std::mem::size_of_val(self)
    }

    /// Проверяет валидность объекта
    ///
    /// Возвращает `true`, если объект в рабочем состоянии
    /// и может быть использован повторно.
    #[inline]
    fn validate(&self) -> bool {
        true
    }

    /// Вызывается при получении объекта из пула
    ///
    /// Подготавливает объект к использованию после периода простоя.
    #[inline]
    fn on_acquire(&mut self) {}

    /// Вызывается при возврате объекта в пул
    ///
    /// По умолчанию просто сбрасывает объект.
    #[inline]
    fn on_release(&mut self) {
        self.reset();
    }

    /// Приоритет объекта (для стратегий вытеснения)
    ///
    /// Возвращает значение от 0 (низкий приоритет) до 255 (высокий приоритет).
    /// Объекты с более низким приоритетом будут вытеснены первыми при нехватке памяти.
    #[inline]
    fn priority(&self) -> u8 {
        128 // Средний приоритет по умолчанию
    }
}

clone_trait_object!(ManagedObject);

/// Трейт для управления жизненным циклом конкретного типа объектов
///
/// Обеспечивает унифицированный интерфейс для создания, уничтожения и переработки
/// объектов определенного типа. Реализации этого трейта содержат бизнес-логику
/// для управления конкретными типами ресурсов.
pub trait TypedLifecycle<T: ManagedObject>: Send + Sync + 'static {
    /// Создает новый объект данного типа
    fn create(&self) -> T;

    /// Уничтожает объект и освобождает связанные ресурсы
    ///
    /// По умолчанию просто позволяет объекту выйти из области видимости,
    /// что вызовет его деструктор.
    #[inline]
    fn destroy(&self, _obj: T) {
        // Объект будет уничтожен автоматически системой владения Rust
        // при выходе из этой функции
    }

    /// Перерабатывает объект для повторного использования
    ///
    /// Восстанавливает объект в работоспособное состояние без необходимости
    /// полного уничтожения и пересоздания. Возвращает ошибку, если объект
    /// не может быть переработан.
    fn recycle(&self, obj: &mut T) -> Result<(), LifecycleError>;
}

// ====================== Система ошибок ======================

/// Ошибки, связанные с управлением жизненным циклом объектов
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum LifecycleError {
    /// Объект не прошел валидацию и не может быть использован
    #[error("Object validation failed")]
    ValidationFailed,

    /// Объект не поддерживает переработку
    #[error("Recycling not allowed for this object")]
    RecyclingNotAllowed,

    /// Ошибка при работе с ресурсами объекта
    #[error("Resource error: {0}")]
    ResourceError(String),

    /// Пользовательская ошибка с описанием
    #[error("Custom error: {0}")]
    Custom(String),
}

// ====================== Реализация пула ======================

/// Пул объектов с типизированным жизненным циклом
///
/// Управляет коллекцией объектов, реализующих трейт `ManagedObject`,
/// обеспечивая их переиспользование и эффективное управление ресурсами.
#[derive(Debug)]
pub struct ObjectPool<T: ManagedObject> {
    objects: Vec<T>,
    lifecycle: Arc<dyn TypedLifecycle<T>>,
    stats: PoolStats,
    config: PoolConfig,
}

/// Конфигурация пула объектов
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Максимальное количество объектов в пуле (0 = неограниченно)
    pub max_size: usize,
    /// Предварительное создание объектов при инициализации пула
    pub preallocate: usize,
    /// Стратегия вытеснения при достижении максимального размера
    pub eviction_strategy: EvictionStrategy,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_size: 0, // Неограниченный размер по умолчанию
            preallocate: 0,
            eviction_strategy: EvictionStrategy::LeastRecentlyUsed,
        }
    }
}

/// Стратегия вытеснения объектов из пула
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictionStrategy {
    /// Вытеснять объекты, которые использовались наименее часто
    LeastFrequentlyUsed,
    /// Вытеснять объекты, которые использовались давно
    LeastRecentlyUsed,
    /// Вытеснять объекты с наименьшим приоритетом
    LowestPriority,
}

impl<T: ManagedObject> ObjectPool<T> {
    /// Создает новый пул с указанным жизненным циклом
    #[inline]
    pub fn new(lifecycle: Arc<dyn TypedLifecycle<T>>) -> Self {
        Self::with_config(lifecycle, PoolConfig::default())
    }

    /// Создает новый пул с указанным жизненным циклом и конфигурацией
    pub fn with_config(lifecycle: Arc<dyn TypedLifecycle<T>>, config: PoolConfig) -> Self {
        let mut pool = Self {
            objects: Vec::with_capacity(config.preallocate.max(16)),
            lifecycle,
            stats: PoolStats::default(),
            config,
        };

        // Предварительное создание объектов, если требуется
        if pool.config.preallocate > 0 {
            pool.preallocate();
        }

        pool
    }

    /// Предварительно создает объекты в пуле согласно конфигурации
    pub fn preallocate(&mut self) {
        let to_create = self.config.preallocate.saturating_sub(self.objects.len());
        for _ in 0..to_create {
            let obj = self.lifecycle.create();
            self.objects.push(obj);
            self.stats.created += 1;
        }
    }

    /// Получает объект из пула или создает новый
    pub fn acquire(&mut self) -> PooledObject<T> {
        if let Some(mut obj) = self.objects.pop() {
            obj.on_acquire();
            self.stats.reused += 1;
            PooledObject::new(obj, Arc::clone(&self.lifecycle))
        } else {
            let obj = self.lifecycle.create();
            self.stats.created += 1;
            PooledObject::new(obj, Arc::clone(&self.lifecycle))
        }
    }

    /// Возвращает объект в пул
    pub fn release(&mut self, mut obj: T) {
        obj.on_release();

        if !obj.validate() {
            self.lifecycle.destroy(obj);
            self.stats.destroyed += 1;
            return;
        }

        // Проверяем, не превышаем ли лимит размера пула
        if self.config.max_size > 0 && self.objects.len() >= self.config.max_size {
            // Применяем стратегию вытеснения
            match self.config.eviction_strategy {
                EvictionStrategy::LowestPriority => {
                    // Если новый объект имеет более высокий приоритет, заменяем объект с наименьшим приоритетом
                    if let Some(min_idx) = self.find_lowest_priority_index() {
                        if self.objects[min_idx].priority() < obj.priority() {
                            let evicted = std::mem::replace(&mut self.objects[min_idx], obj);
                            self.lifecycle.destroy(evicted);
                            self.stats.recycled += 1;
                            self.stats.evicted += 1;
                            return;
                        }
                    }
                },
                // Для других стратегий просто уничтожаем новый объект
                _ => {}
            }

            // Если не нашли место или объект не подходит, уничтожаем его
            self.lifecycle.destroy(obj);
            self.stats.destroyed += 1;
        } else {
            // Добавляем объект в пул
            self.objects.push(obj);
            self.stats.recycled += 1;
        }
    }

    /// Находит индекс объекта с наименьшим приоритетом
    fn find_lowest_priority_index(&self) -> Option<usize> {
        if self.objects.is_empty() {
            return None;
        }

        let mut min_idx = 0;
        let mut min_priority = self.objects[0].priority();

        for (i, obj) in self.objects.iter().enumerate().skip(1) {
            let priority = obj.priority();
            if priority < min_priority {
                min_priority = priority;
                min_idx = i;
            }
        }

        Some(min_idx)
    }

    /// Очищает пул, уничтожая все объекты
    pub fn clear(&mut self) {
        let count = self.objects.len();
        while let Some(obj) = self.objects.pop() {
            self.lifecycle.destroy(obj);
        }
        self.stats.destroyed += count;
    }

    /// Изменяет размер пула, удаляя лишние объекты или добавляя новые
    pub fn resize(&mut self, new_size: usize) {
        if new_size < self.objects.len() {
            // Удаляем лишние объекты
            let to_remove = self.objects.len() - new_size;
            for _ in 0..to_remove {
                if let Some(obj) = self.objects.pop() {
                    self.lifecycle.destroy(obj);
                    self.stats.destroyed += 1;
                }
            }
        } else if new_size > self.objects.len() {
            // Добавляем новые объекты
            let to_add = new_size - self.objects.len();
            for _ in 0..to_add {
                let obj = self.lifecycle.create();
                self.objects.push(obj);
                self.stats.created += 1;
            }
        }
    }

    /// Возвращает количество объектов в пуле
    #[inline]
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Проверяет, пуст ли пул
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Возвращает текущую статистику пула
    #[inline]
    pub fn stats(&self) -> &PoolStats {
        &self.stats
    }

    /// Возвращает конфигурацию пула
    #[inline]
    pub fn config(&self) -> &PoolConfig {
        &self.config
    }
}

impl<T: ManagedObject> Drop for ObjectPool<T> {
    fn drop(&mut self) {
        self.clear();
    }
}

/// Статистика пула
#[derive(Default, Debug)]
pub struct PoolStats {
    created: usize,
    reused: usize,
    recycled: usize,
    destroyed: usize,
    evicted: usize,
}

// ====================== Пример использования ======================

/// Пример объекта - соединение с БД
#[derive(Clone)]
struct DatabaseConnection {
    id: u64,
    is_connected: bool,
    buffer: Vec<u8>,
}

impl ManagedObject for DatabaseConnection {
    fn reset(&mut self) {
        self.buffer.clear();
        self.is_connected = false;
    }

    fn size_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.buffer.capacity()
    }

    fn validate(&self) -> bool {
        self.id != 0
    }

    fn on_acquire(&mut self) {
        if !self.is_connected {
            self.connect();
        }
    }

    fn on_release(&mut self) {
        self.reset();
    }

    fn priority(&self) -> u8 {
        if self.buffer.capacity() > 1024 { 50 } else { 100 }
    }
}

impl DatabaseConnection {
    fn new(id: u64) -> Self {
        Self {
            id,
            is_connected: false,
            buffer: Vec::new(),
        }
    }

    fn connect(&mut self) {
        self.is_connected = true;
    }

    fn execute_query(&mut self, query: &str) {
        self.buffer.extend(query.bytes());
    }
}

/// Реализация жизненного цикла для соединений
struct ConnectionLifecycle {
    next_id: AtomicU64,
}

impl TypedLifecycle<DatabaseConnection> for ConnectionLifecycle {
    fn create(&self) -> DatabaseConnection {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        DatabaseConnection::new(id)
    }

    fn destroy(&self, mut conn: DatabaseConnection) {
        conn.reset();
    }

    fn recycle(&self, conn: &mut DatabaseConnection) -> Result<(), LifecycleError> {
        if conn.validate() {
            conn.reset();
            Ok(())
        } else {
            Err(LifecycleError::ValidationFailed)
        }
    }
}

// ====================== Тесты ======================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_workflow() {
        let lifecycle = Arc::new(ConnectionLifecycle {
            next_id: AtomicU64::new(1),
        });

        let mut pool = ObjectPool::new(lifecycle.clone());

        // Получаем соединение из пула
        let mut conn = pool.acquire();
        conn.get_mut().execute_query("SELECT * FROM users");

        // Возвращаем соединение в пул (в реальном коде через release)
        drop(conn);

        // Проверяем статистику
        let stats = pool.stats();
        assert_eq!(stats.created, 1);
        assert_eq!(stats.reused, 0);
    }
}

// ====================== Расширенное использование ======================

/// Адаптер для интеграции с другими системами
pub struct ForeignObjectAdapter<F, T> {
    create_fn: F,
    _marker: std::marker::PhantomData<T>,
}

impl<F, T> ForeignObjectAdapter<F, T>
where
    F: Fn() -> T + Send + Sync + 'static,
    T: ManagedObject,
{
    pub fn new(create_fn: F) -> Self {
        Self {
            create_fn,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<F, T> TypedLifecycle<T> for ForeignObjectAdapter<F, T>
where
    F: Fn() -> T + Send + Sync + 'static,
    T: ManagedObject,
{
    fn create(&self) -> T {
        (self.create_fn)()
    }

    fn destroy(&self, _obj: T) {
        // Деструктор по умолчанию
    }

    fn recycle(&self, obj: &mut T) -> Result<(), LifecycleError> {
        obj.reset();
        Ok(())
    }
}

/// Обертка для объекта из пула
///
/// Обеспечивает безопасный доступ к объекту и автоматический возврат
/// объекта в пул при выходе из области видимости.
pub struct PooledObject<T: ManagedObject> {
    obj: Option<T>,
    lifecycle: Arc<dyn TypedLifecycle<T>>,
    return_to_pool: Option<Arc<parking_lot::Mutex<ObjectPool<T>>>>,
}

impl<T: ManagedObject> PooledObject<T> {
    /// Создает новую обертку для объекта
    ///
    /// Внутренний метод, вызывается из `ObjectPool::acquire`
    fn new(obj: T, lifecycle: Arc<dyn TypedLifecycle<T>>) -> Self {
        Self {
            obj: Some(obj),
            lifecycle,
            return_to_pool: None,
        }
    }

    /// Создает обертку, которая будет автоматически возвращать объект в пул
    ///
    /// При выходе из области видимости объект будет возвращен в указанный пул.
    pub(crate) fn with_auto_return(
        obj: T,
        lifecycle: Arc<dyn TypedLifecycle<T>>,
        pool: Arc<parking_lot::Mutex<ObjectPool<T>>>
    ) -> Self {
        Self {
            obj: Some(obj),
            lifecycle,
            return_to_pool: Some(pool),
        }
    }

    /// Получает неизменяемую ссылку на объект
    ///
    /// # Panics
    ///
    /// Паникует, если объект уже был взят из обертки.
    pub fn get(&self) -> &T {
        self.obj.as_ref().expect("Object already taken")
    }

    /// Получает изменяемую ссылку на объект
    ///
    /// # Panics
    ///
    /// Паникует, если объект уже был взят из обертки.
    pub fn get_mut(&mut self) -> &mut T {
        self.obj.as_mut().expect("Object already taken")
    }

    /// Извлекает объект из обертки, предотвращая автоматический возврат в пул
    ///
    /// # Returns
    ///
    /// Возвращает `None`, если объект уже был извлечен.
    pub fn take(&mut self) -> Option<T> {
        self.obj.take()
    }

    /// Возвращает объект в пул, потребляя обертку
    pub fn return_to_pool(mut self) -> Result<(), LifecycleError> {
        if let Some(obj) = self.obj.take() {
            if let Some(pool) = &self.return_to_pool {
                let mut pool_guard = pool.lock();
                pool_guard.release(obj);
                Ok(())
            } else {
                // Если нет пула для возврата, просто уничтожаем объект
                self.lifecycle.destroy(obj);
                Err(LifecycleError::Custom("No pool to return object to".to_string()))
            }
        } else {
            Err(LifecycleError::Custom("Object already taken".to_string()))
        }
    }

    /// Применяет функцию к объекту и возвращает результат
    ///
    /// Позволяет безопасно работать с объектом без проверок на None.
    ///
    /// # Panics
    ///
    /// Паникует, если объект уже был взят из обертки.
    pub fn map<U, F>(&self, f: F) -> U
    where
        F: FnOnce(&T) -> U,
    {
        f(self.get())
    }

    /// Применяет функцию к изменяемому объекту и возвращает результат
    ///
    /// Позволяет безопасно работать с объектом без проверок на None.
    ///
    /// # Panics
    ///
    /// Паникует, если объект уже был взят из обертки.
    pub fn map_mut<U, F>(&mut self, f: F) -> U
    where
        F: FnOnce(&mut T) -> U,
    {
        f(self.get_mut())
    }
}

impl<T: ManagedObject> Drop for PooledObject<T> {
    fn drop(&mut self) {
        if let Some(obj) = self.obj.take() {
            if let Some(pool) = &self.return_to_pool {
                // Автоматически возвращаем объект в пул
                let mut pool_guard = pool.lock();
                pool_guard.release(obj);
            } else {
                // Если нет пула, просто уничтожаем объект
                self.lifecycle.destroy(obj);
            }
        }
    }
}

/// Функции для безопасного создания и использования пулов
pub mod pool_utils {
    use super::*;

    /// Создает пул объектов с автоматическим возвратом
    ///
    /// Возвращает Arc<Mutex<ObjectPool<T>>>, который можно использовать
    /// для получения объектов с автоматическим возвратом в пул.
    pub fn create_shared_pool<T: ManagedObject>(
        lifecycle: Arc<dyn TypedLifecycle<T>>,
        config: PoolConfig,
    ) -> Arc<parking_lot::Mutex<ObjectPool<T>>> {
        Arc::new(parking_lot::Mutex::new(ObjectPool::with_config(lifecycle, config)))
    }

    /// Получает объект из пула с автоматическим возвратом
    ///
    /// Когда возвращенный `PooledObject` выходит из области видимости,
    /// объект автоматически возвращается в пул.
    pub fn acquire_from_shared<T: ManagedObject>(
        pool: &Arc<parking_lot::Mutex<ObjectPool<T>>>,
    ) -> PooledObject<T> {
        let mut pool_guard = pool.lock();
        let lifecycle = Arc::clone(&pool_guard.lifecycle);
        let mut obj = pool_guard.acquire();

        // Настраиваем автоматический возврат в пул
        let return_pool = Arc::clone(pool);
        obj.return_to_pool = Some(return_pool);

        obj
    }
}
