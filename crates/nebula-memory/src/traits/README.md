# Traits в nebula-memory

Модуль `traits` в `nebula-memory` предоставляет набор универсальных интерфейсов для интеграции и расширения функциональности управления памятью в вашем проекте. Эти трейты спроектированы для обеспечения максимальной гибкости и минимальных зависимостей между компонентами.

## Содержание

- [Обзор](#обзор)
- [MemoryContext](#memorycontext)
- [MemoryIsolation](#memoryisolation)
- [Priority](#priority)
- [ObjectLifecycle](#objectlifecycle)
- [MemoryObserver](#memoryobserver)
- [ObjectFactory](#objectfactory)
- [Интеграция с другими компонентами](#интеграция-с-другими-компонентами)

## Обзор

Модуль `traits` содержит следующие основные трейты:

- `MemoryContext` - контекст памяти для изоляции и управления ресурсами
- `MemoryIsolation` - механизмы изоляции памяти между компонентами системы
- `Priority` - приоритезация операций с памятью
- `ObjectLifecycle` - управление жизненным циклом объектов
- `MemoryObserver` - наблюдение и мониторинг использования памяти
- `ObjectFactory` - создание и управление объектами

## MemoryContext

Трейт `MemoryContext` обеспечивает абстракцию для изоляции памяти между различными компонентами системы. Контексты могут быть организованы в иерархию, где дочерние контексты наследуют ограничения родительских.

### Использование

```rust
use nebula_memory::traits::context::{MemoryContext, SimpleMemoryContext};

// Создание корневого контекста с ограничением памяти
let root_context = SimpleMemoryContext::new(
    "root", // идентификатор
    10,      // приоритет
    Some(1024 * 1024 * 100) // лимит 100 MB
);

// Создание дочернего контекста, наследующего ограничения
let child_context = root_context.create_child_context("child".to_string());

// Проверка возможности выделения памяти
if child_context.can_allocate(1024 * 1024) {
    println!("Можно выделить 1MB памяти");
} else {
    println!("Недостаточно памяти для выделения");
}

// Использование в аллокаторах и других компонентах
fn allocate_memory<C: MemoryContext + ?Sized>(size: usize, context: &C) -> Result<Vec<u8>, String> {
    if context.can_allocate(size) {
        Ok(Vec::with_capacity(size))
    } else {
        Err(format!("Контекст {} не может выделить {} байт", 
            context.identifier(), size))
    }
}
```

## MemoryIsolation

Трейт `MemoryIsolation` обеспечивает абстракцию для изоляции памяти, позволяя ограничивать и контролировать использование памяти различными компонентами системы.

### Использование

```rust
use nebula_memory::traits::isolation::{MemoryIsolation, MemoryAllocation};
use nebula_memory::traits::context::SimpleMemoryContext;

// Реализуем трейт MemoryIsolation для нашего изолятора памяти
struct MyMemoryIsolator;

impl MemoryIsolation for MyMemoryIsolator {
    type Context = SimpleMemoryContext;
    
    fn request_memory(&self, size: usize, context: &Self::Context) -> Result<MemoryAllocation, _> {
        if context.can_allocate(size) {
            // Создаем токен аллокации с обработчиком освобождения
            Ok(MemoryAllocation::new(
                size,
                context.identifier().to_string(),
                move || {
                    // Код, который выполнится при освобождении памяти
                    println!("Освобождено {} байт", size);
                }
            ))
        } else {
            Err(MemoryIsolationError::MemoryLimitExceeded {
                requested: size,
                available: context.memory_limit().unwrap_or(0),
                context: context.identifier().to_string(),
            })
        }
    }
    
    // ... реализация других методов
}

// Использование
let context = SimpleMemoryContext::new("app", 10, Some(1024 * 1024));
let isolator = MyMemoryIsolator;

// Запрос на выделение памяти
match isolator.request_memory(1024, &context) {
    Ok(allocation) => {
        // Используем выделенную память
        println!("Выделено {} байт в контексте {}", 
            allocation.size, allocation.context_id);
        
        // Память будет автоматически освобождена при уничтожении allocation
        // или можно явно вызвать
        allocation.release();
    },
    Err(e) => eprintln!("Ошибка выделения памяти: {}", e),
}
```

## Priority

Трейт `Priority` может быть реализован для любого типа, который имеет понятие приоритета, что позволяет системе управления памятью принимать решения о выделении и освобождении ресурсов.

### Использование

```rust
use nebula_memory::traits::priority::{Priority, NumericPriority, DynamicPriority};
use std::cmp::Ordering;

// Использование готовой реализации NumericPriority
let high_priority = NumericPriority::new(200);
let low_priority = NumericPriority::new(50);

assert_eq!(high_priority.compare(&low_priority), Ordering::Greater);

// Создаем свой тип с приоритетом
struct MemoryBuffer {
    data: Vec<u8>,
    importance: u8,
}

impl Priority for MemoryBuffer {
    fn priority(&self) -> u8 {
        self.importance
    }
    
    fn set_priority(&mut self, priority: u8) {
        self.importance = priority;
    }
}

// Использование динамического приоритета
let mut buffer = MemoryBuffer {
    data: vec![0; 1024],
    importance: 100,
};

// Повышаем приоритет при обработке критических данных
buffer.set_priority(200);

// Сравнение приоритетов разных объектов
fn process_by_priority(buffers: &mut [&mut dyn Priority]) {
    // Сортируем буферы по приоритету (высокий в начале)
    buffers.sort_by(|a, b| b.priority().cmp(&a.priority()));
    
    for buffer in buffers {
        // Обрабатываем буферы в порядке приоритета
        println!("Обработка буфера с приоритетом {}", buffer.priority());
    }
}
```

## ObjectLifecycle

Трейт `ObjectLifecycle` предоставляет интерфейс для управления жизненным циклом объектов, что особенно полезно при реализации пулов объектов и других систем управления памятью.

### Использование

```rust
use nebula_memory::traits::lifecycle::{ObjectLifecycle, Resetable, DefaultLifecycle};

// Создаем структуру, поддерживающую сброс
#[derive(Default, Clone)]
struct DataBuffer {
    data: Vec<u8>,
    is_dirty: bool,
    last_access: std::time::Instant,
}

impl Resetable for DataBuffer {
    fn reset(&mut self) {
        self.data.clear();
        self.is_dirty = false;
        self.last_access = std::time::Instant::now();
    }
    
    fn is_reset(&self) -> bool {
        self.data.is_empty() && !self.is_dirty
    }
}

// Используем готовую реализацию DefaultLifecycle
let lifecycle = DefaultLifecycle::<DataBuffer>::new();

// Создаем объект
let mut buffer = lifecycle.create();
buffer.data.extend_from_slice(&[1, 2, 3, 4]);
buffer.is_dirty = true;

// Сбрасываем объект в начальное состояние
lifecycle.reset(&mut buffer);
assert!(buffer.is_reset());

// Создаем собственный менеджер жизненного цикла
struct CustomLifecycle;

impl ObjectLifecycle for CustomLifecycle {
    type Object = DataBuffer;
    
    fn create(&self) -> Self::Object {
        DataBuffer::default()
    }
    
    fn reset(&self, obj: &mut Self::Object) {
        obj.data.clear();
        obj.is_dirty = false;
    }
    
    fn destroy(&self, obj: Self::Object) {
        // Специальная логика уничтожения, если нужна
        println!("Уничтожение буфера размером {}", obj.data.capacity());
    }
    
    fn validate(&self, obj: &Self::Object) -> bool {
        !obj.is_dirty
    }
    
    fn on_idle(&self, obj: &mut Self::Object) {
        // Сжимаем буфер при простое для экономии памяти
        obj.data.shrink_to_fit();
    }
}
```

## MemoryObserver

Трейт `MemoryObserver` предоставляет интерфейс для наблюдения за использованием памяти в системе, что позволяет реализовать мониторинг, логирование и другие механизмы обратной связи.

### Использование

```rust
use nebula_memory::traits::observer::{MemoryObserver, MemoryEvent, MemoryPressure};
use std::sync::Arc;

// Создаем наблюдателя, который логирует события памяти
#[derive(Clone)]
struct LoggingObserver;

impl MemoryObserver for LoggingObserver {
    fn on_memory_event(&self, event: MemoryEvent) {
        match &event {
            MemoryEvent::Allocation { size, context, .. } => {
                println!("Выделено {} байт в контексте '{}'", size, context);
            },
            MemoryEvent::Deallocation { size, context, .. } => {
                println!("Освобождено {} байт в контексте '{}'", size, context);
            },
            MemoryEvent::PressureChange { pressure, .. } => {
                println!("Изменение давления памяти: {}", pressure);
            },
            _ => println!("Другое событие памяти: {:?}", event),
        }
    }
}

// Система, которая может уведомлять наблюдателей
struct MemorySystem {
    observers: Vec<Arc<dyn MemoryObserver>>,
}

impl MemorySystem {
    fn new() -> Self {
        Self { observers: Vec::new() }
    }
    
    fn add_observer(&mut self, observer: Arc<dyn MemoryObserver>) -> usize {
        let id = self.observers.len();
        self.observers.push(observer);
        id
    }
    
    fn allocate_memory(&self, size: usize, context: &str) {
        // Выделяем память...
        
        // Уведомляем наблюдателей
        for observer in &self.observers {
            observer.on_allocation(size, context.to_string());
        }
    }
    
    fn notify_pressure_change(&self, pressure: MemoryPressure) {
        for observer in &self.observers {
            observer.on_pressure_change(pressure);
        }
    }
}

// Использование
let mut memory_system = MemorySystem::new();
let observer = Arc::new(LoggingObserver);
memory_system.add_observer(observer);

// Вызывает уведомление наблюдателей
memory_system.allocate_memory(1024 * 1024, "main");
memory_system.notify_pressure_change(MemoryPressure::High);
```

## ObjectFactory

Трейт `ObjectFactory` предоставляет абстракцию для создания и инициализации объектов, что особенно полезно при реализации пулов и других компонентов управления памятью.

### Использование

```rust
use nebula_memory::traits::factory::{ObjectFactory, ObjectFactoryResult, SimpleObjectFactory};

// Тип, который мы будем создавать
struct User {
    id: u64,
    name: String,
    active: bool,
}

// Создаем фабрику с функцией-создателем
let user_factory = SimpleObjectFactory::new(|| -> ObjectFactoryResult<User> {
    Ok(User {
        id: 0,
        name: String::new(),
        active: false,
    })
});

// Создаем объект
let mut user = user_factory.create().unwrap();
user.id = 42;
user.name = "Alice".to_string();
user.active = true;

// Создаем пакет объектов
let users = user_factory.create_batch(5).unwrap();
println!("Создано {} пользователей", users.len());

// Создаем фабрику с ограничением на количество объектов
let mut limited_factory = SimpleObjectFactory::new(|| Ok(User {
    id: 0,
    name: String::new(),
    active: false,
}));
limited_factory.set_max_count(Some(10));

// После создания 10 объектов, попытка создать 11-й вызовет ошибку
for _ in 0..10 {
    let _ = limited_factory.create().unwrap();
}
match limited_factory.create() {
    Ok(_) => println!("Успешно создан объект"),
    Err(e) => println!("Ошибка создания: {}", e),
}
```

## Интеграция с другими компонентами

Трейты в `nebula-memory` спроектированы для легкой интеграции с другими компонентами системы. Вот пример интеграции с системой воркфлоу:

```rust
use nebula_memory::traits::context::MemoryContext;
use nebula_memory::traits::isolation::MemoryIsolation;
use nebula_memory::traits::observer::MemoryObserver;

// Реализуем компонент для выполнения задач с учетом ограничений памяти
struct WorkflowExecutor<I: MemoryIsolation> {
    memory_isolator: I,
    observers: Vec<Arc<dyn MemoryObserver>>,
}

impl<I: MemoryIsolation> WorkflowExecutor<I> {
    fn new(memory_isolator: I) -> Self {
        Self {
            memory_isolator,
            observers: Vec::new(),
        }
    }
    
    fn add_observer(&mut self, observer: Arc<dyn MemoryObserver>) {
        self.observers.push(observer);
    }
    
    async fn execute_task<C>(&self, task: Task, context: &C) -> Result<TaskResult, TaskError>
    where
        C: MemoryContext + ?Sized,
        C: Into<I::Context>,
    {
        // Запрашиваем память для выполнения задачи
        let memory = self.memory_isolator.request_memory(
            task.estimated_memory(), 
            context
        )?;
        
        // Уведомляем наблюдателей
        for observer in &self.observers {
            observer.on_allocation(memory.size, context.identifier().to_string());
        }
        
        // Выполняем задачу
        let result = task.execute().await?;
        
        // Память будет автоматически освобождена при выходе из области видимости
        Ok(result)
    }
}
```

Этот пример демонстрирует, как различные трейты из `nebula-memory` могут быть объединены для создания системы с изоляцией памяти, мониторингом и контролем ресурсов.

## Заключение

Трейты в `nebula-memory` предоставляют гибкий и расширяемый интерфейс для интеграции с различными компонентами вашей системы. Они спроектированы для минимизации зависимостей и обеспечения максимальной переиспользуемости кода.

Для получения дополнительной информации, обратитесь к документации каждого конкретного трейта и примерам в директории `examples/`.
