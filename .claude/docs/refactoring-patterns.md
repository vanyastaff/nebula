# Refactoring Patterns для Nebula

## Принцип рефакторинга

> **"продолжаем делать реальный рефакторинг правильный а не просто чтобы ошибка исчезла"**

Применяем проверенные архитектурные паттерны для решения проблем системно.

---

## Pattern 1: Extension Trait Pattern

### Проблема
Работа с `Arc<Mutex<T>>` требует постоянного `.lock().unwrap()`, что загромождает код и усложняет читаемость.

```rust
// Неудобно и многословно
let arena = arena_ref.lock().unwrap();
arena.allocate(value);

let mut arena = arena_ref.lock().unwrap();
arena.reset();
```

### Решение
Создать extension trait с эргономичными методами:

```rust
pub trait ArenaExt<T> {
    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Arena<T>) -> R;

    fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Arena<T>) -> R;
}

impl<T> ArenaExt<T> for Arc<Mutex<Arena<T>>> {
    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Arena<T>) -> R,
    {
        let arena = self.lock().unwrap();
        f(&arena)
    }

    fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Arena<T>) -> R,
    {
        let mut arena = self.lock().unwrap();
        f(&mut arena)
    }
}
```

### Использование

```rust
// До: многословно
let arena = arena_ref.lock().unwrap();
let ptr = arena.allocate(value);
drop(arena);

// После: эргономично
let ptr = arena_ref.with_mut(|arena| arena.allocate(value));

// Чтение
let size = arena_ref.with(|arena| arena.len());
```

### Преимущества
- ✅ Инкапсуляция логики lock/unlock
- ✅ Автоматический drop guard
- ✅ Читаемый и безопасный код
- ✅ Легко тестируется

### Применение
- Shared mutable state (`Arc<Mutex<T>>`, `Arc<RwLock<T>>`)
- Обертки вокруг сложных API
- Упрощение работы с interior mutability

---

## Pattern 2: Type Erasure Wrapper

### Проблема
Trait с associated types не является object-safe и не может использоваться с `dyn`:

```rust
// Не компилируется - trait не object-safe
trait TypedValidator {
    type Input;
    type Output;
    type Error;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error>;
}

// ❌ Error: associated types make trait not object-safe
fn use_validator(validator: &dyn TypedValidator) { }
```

Но нужен dynamic dispatch для коллекций validators или plugin systems.

### Решение
Создать object-safe trait-обертку и adapter:

```rust
// Object-safe trait для type erasure
trait DisplayRuleEvaluator: Send + Sync {
    fn evaluate(&self, ctx: &DisplayContext) -> Result<bool, DisplayError>;
    fn name(&self) -> &str;
}

// Adapter для конкретных типов
struct ValidatorAdapter<V> {
    validator: V,
}

// Реализация для любого TypedValidator
impl<V> DisplayRuleEvaluator for ValidatorAdapter<V>
where
    V: TypedValidator<Input = Value, Output = ()>,
{
    fn evaluate(&self, ctx: &DisplayContext) -> Result<bool, DisplayError> {
        let value = Value::from(ctx);
        self.validator
            .validate(&value)
            .map(|_| true)
            .map_err(|e| DisplayError::ValidationFailed(e))
    }

    fn name(&self) -> &str {
        std::any::type_name::<V>()
    }
}

// Теперь можно использовать dynamic dispatch
pub struct DisplayRule {
    evaluator: Box<dyn DisplayRuleEvaluator>,
}

impl DisplayRule {
    pub fn new<V>(validator: V) -> Self
    where
        V: TypedValidator<Input = Value, Output = ()> + Send + Sync + 'static,
    {
        Self {
            evaluator: Box::new(ValidatorAdapter { validator }),
        }
    }
}
```

### Использование

```rust
// Можем создать коллекцию разных validators
let rules: Vec<DisplayRule> = vec![
    DisplayRule::new(MinLength { min: 5 }),
    DisplayRule::new(MaxLength { max: 100 }),
    DisplayRule::new(Pattern { regex: "..." }),
];

// И использовать их через единый интерфейс
for rule in &rules {
    rule.evaluator.evaluate(&context)?;
}
```

### Преимущества
- ✅ Позволяет dynamic dispatch для typed traits
- ✅ Сохраняет type safety внутри adapter
- ✅ Позволяет гетерогенные коллекции
- ✅ Расширяемость через trait objects

### Применение
- Plugin systems
- Validator chains
- Strategy pattern с разными типами
- Когда нужна коллекция `Vec<Box<dyn Trait>>`

---

## Pattern 3: Scoped Callback (RAII)

### Проблема
Ресурсы (arena allocations, temp files, connections) требуют ручной очистки, что приводит к:
- Утечкам при early return
- Забытым cleanup вызовам
- Сложной обработке ошибок

```rust
// ❌ Легко забыть reset или пропустить при panic
let mut arena = Arena::new();
arena.allocate(value1);
arena.allocate(value2);
// ... код может паниковать ...
arena.reset(); // Может не выполниться!
```

### Решение
RAII guard с callback и lifetime-гарантиями:

```rust
pub struct Guard<'a, T> {
    arena: &'a mut Arena<T>,
}

impl<'a, T> Drop for Guard<'a, T> {
    fn drop(&mut self) {
        // Автоматическая очистка при выходе из scope
        self.arena.reset();
    }
}

impl<T> Arena<T> {
    pub fn scope<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut Guard<T>) -> R,
    {
        let mut guard = Guard { arena: self };
        f(&mut guard) // Cleanup автоматически в drop(guard)
    }
}

// Доступ к arena через guard
impl<'a, T> Guard<'a, T> {
    pub fn allocate(&mut self, value: T) -> ArenaPtr<T> {
        self.arena.allocate(value)
    }

    pub fn get(&self, ptr: ArenaPtr<T>) -> Option<&T> {
        self.arena.get(ptr)
    }
}
```

### Использование

```rust
let mut arena = Arena::new();

// Scope 1: автоматическая очистка
let result = arena.scope(|guard| {
    let ptr1 = guard.allocate("hello");
    let ptr2 = guard.allocate("world");

    // Работаем с данными
    format!("{} {}", guard.get(ptr1)?, guard.get(ptr2)?)
}); // <- arena.reset() вызывается автоматически

// Scope 2: новые данные, предыдущие очищены
arena.scope(|guard| {
    let ptr = guard.allocate("new data");
    // ...
}); // <- снова reset()
```

### Преимущества
- ✅ Гарантированная очистка (даже при panic)
- ✅ Невозможно забыть cleanup
- ✅ Lifetime-safety для allocated данных
- ✅ Явный scope для temporary allocations

### Применение
- Arena allocators
- Temporary files/directories
- Database transactions
- Lock guards
- Resource acquisition (RAII pattern)

---

## Pattern 4: Builder Pattern с Type State

### Проблема
Конфигурация сложных объектов с обязательными и опциональными параметрами:

```rust
// ❌ Легко забыть обязательные параметры
let cache = Cache::new();
cache.set_capacity(100); // Забыли set_policy!
```

### Решение
Type-state builder, который гарантирует корректность на compile-time:

```rust
// Marker types для состояний
struct NeedsPolicy;
struct NeedsCapacity;
struct Ready;

struct CacheBuilder<State> {
    policy: Option<EvictionPolicy>,
    capacity: Option<usize>,
    _state: PhantomData<State>,
}

// Начальное состояние
impl CacheBuilder<NeedsPolicy> {
    pub fn new() -> Self {
        Self {
            policy: None,
            capacity: None,
            _state: PhantomData,
        }
    }

    pub fn with_policy(self, policy: EvictionPolicy) -> CacheBuilder<NeedsCapacity> {
        CacheBuilder {
            policy: Some(policy),
            capacity: self.capacity,
            _state: PhantomData,
        }
    }
}

// Следующее состояние
impl CacheBuilder<NeedsCapacity> {
    pub fn with_capacity(self, capacity: usize) -> CacheBuilder<Ready> {
        CacheBuilder {
            policy: self.policy,
            capacity: Some(capacity),
            _state: PhantomData,
        }
    }
}

// Финальное состояние - можно build
impl CacheBuilder<Ready> {
    pub fn build(self) -> Cache {
        Cache {
            policy: self.policy.unwrap(),
            capacity: self.capacity.unwrap(),
        }
    }
}
```

### Использование

```rust
// ✅ Компилируется - все параметры установлены
let cache = CacheBuilder::new()
    .with_policy(LruPolicy::new())
    .with_capacity(1000)
    .build();

// ❌ Не компилируется - забыли capacity
let cache = CacheBuilder::new()
    .with_policy(LruPolicy::new())
    .build(); // Error: method `build` not found
```

### Преимущества
- ✅ Compile-time проверка корректности
- ✅ Невозможно создать некорректный объект
- ✅ Self-documenting API
- ✅ Fluent interface

### Применение
- Конфигурация сложных объектов
- Гарантии инициализации
- Wizard-style API

---

## Pattern 5: Newtype Pattern для Type Safety

### Проблема
Примитивные типы легко перепутать:

```rust
// ❌ Легко перепутать параметры
fn transfer(from: u64, to: u64, amount: u64) { }

transfer(123, 456, 789); // Что есть что?
transfer(789, 123, 456); // Ошибка логики, но компилируется
```

### Решение
Newtype wrappers для domain-specific типов:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccountId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Amount(u64);

impl AccountId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

impl Amount {
    pub fn new(value: u64) -> Self {
        assert!(value > 0, "Amount must be positive");
        Self(value)
    }

    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self)
    }
}

fn transfer(from: AccountId, to: AccountId, amount: Amount) {
    // Type-safe!
}
```

### Использование

```rust
let from = AccountId::new(123);
let to = AccountId::new(456);
let amount = Amount::new(789);

// ✅ Type-safe
transfer(from, to, amount);

// ❌ Не компилируется
transfer(amount, from, to); // Error: type mismatch
transfer(123, 456, 789);    // Error: expected AccountId, found u64
```

### Преимущества
- ✅ Невозможно перепутать параметры
- ✅ Domain-specific validations
- ✅ Self-documenting типы
- ✅ Zero-cost abstraction

### Применение
- IDs, handles, indices
- Measurements (meters, seconds, bytes)
- Domain primitives
- Validated strings (Email, Url)

---

## Pattern 6: Visitor Pattern для AST

### Проблема
Обход и трансформация AST/tree структур требует много boilerplate:

```rust
// ❌ Нужно писать рекурсивный обход вручную для каждой операции
fn count_nodes(expr: &Expr) -> usize {
    match expr {
        Expr::Binary(left, _, right) => 1 + count_nodes(left) + count_nodes(right),
        Expr::Unary(_, inner) => 1 + count_nodes(inner),
        Expr::Literal(_) => 1,
    }
}
```

### Решение
Visitor trait для разделения алгоритма обхода и операции:

```rust
trait ExprVisitor {
    type Output;

    fn visit_binary(&mut self, left: &Expr, op: BinOp, right: &Expr) -> Self::Output;
    fn visit_unary(&mut self, op: UnOp, expr: &Expr) -> Self::Output;
    fn visit_literal(&mut self, value: &Value) -> Self::Output;
}

impl Expr {
    fn accept<V: ExprVisitor>(&self, visitor: &mut V) -> V::Output {
        match self {
            Expr::Binary(left, op, right) => visitor.visit_binary(left, *op, right),
            Expr::Unary(op, expr) => visitor.visit_unary(*op, expr),
            Expr::Literal(value) => visitor.visit_literal(value),
        }
    }
}

// Конкретный visitor
struct NodeCounter {
    count: usize,
}

impl ExprVisitor for NodeCounter {
    type Output = ();

    fn visit_binary(&mut self, left: &Expr, _op: BinOp, right: &Expr) -> Self::Output {
        self.count += 1;
        left.accept(self);
        right.accept(self);
    }

    fn visit_unary(&mut self, _op: UnOp, expr: &Expr) -> Self::Output {
        self.count += 1;
        expr.accept(self);
    }

    fn visit_literal(&mut self, _value: &Value) -> Self::Output {
        self.count += 1;
    }
}
```

### Использование

```rust
let expr = parse("(a + b) * c");

let mut counter = NodeCounter { count: 0 };
expr.accept(&mut counter);
println!("Nodes: {}", counter.count);

let mut optimizer = ConstantFolder::new();
let optimized = expr.accept(&mut optimizer);
```

### Преимущества
- ✅ Separation of concerns
- ✅ Легко добавлять новые операции
- ✅ Переиспользование логики обхода
- ✅ Type-safe traversal

### Применение
- AST traversal и transformation
- Serialization/deserialization
- Code generation
- Query optimization

---

## Когда применять паттерны

| Проблема | Паттерн | Complexity |
|----------|---------|------------|
| Эргономика Arc<Mutex<T>> | Extension Trait | Low |
| Trait не object-safe | Type Erasure | Medium |
| Ресурсы требуют cleanup | Scoped Callback | Low |
| Сложная инициализация | Type-State Builder | Medium |
| Перепутывание примитивов | Newtype | Low |
| AST traversal | Visitor | High |

## Дополнительные ресурсы

- [Rust Design Patterns](https://rust-unofficial.github.io/patterns/)
- [API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [Effective Rust](https://www.lurklurk.org/effective-rust/)
