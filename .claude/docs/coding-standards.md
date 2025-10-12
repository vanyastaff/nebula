# Nebula Rust Coding Standards

## Основные принципы

### 1. Архитектурный подход
- **Правильный рефакторинг, не патчи**: "продолжаем делать реальный рефакторинг правильный а не просто чтобы ошибка исчезла"
- Решать проблемы архитектурно и системно
- Не подавлять ошибки, а исправлять их причину
- Применять проверенные паттерны проектирования

### 2. Rust 2024 Edition
- Строгая проверка типов и времени жизни
- Explicit type annotations для сложных generic типов
- Использовать sized типы (`String`, `Vec<T>`) вместо unsized (`str`, `[T]`) в тестах
- Trait bounds должны быть явными и полными

### 3. Системность
- Закрывать issues последовательно и полностью
- Документировать архитектурные решения
- Оставлять код лучше, чем он был

## Архитектурные паттерны

### Extension Trait Pattern
Эргономичная обертка для Arc<Mutex<T>>:

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

**Применение**: Упрощение работы с shared mutable state.

### Type Erasure Wrapper Pattern
Мост между typed validators и dynamic dispatch:

```rust
trait DisplayRuleEvaluator: Send + Sync {
    fn evaluate(&self, ctx: &DisplayContext) -> Result<bool, DisplayError>;
    fn name(&self) -> &str;
}

struct ValidatorAdapter<V: TypedValidator<Input = Value>> {
    validator: V,
}

impl<V: TypedValidator<Input = Value>> DisplayRuleEvaluator for ValidatorAdapter<V> {
    fn evaluate(&self, ctx: &DisplayContext) -> Result<bool, DisplayError> {
        let value = Value::from(ctx);
        self.validator.validate(&value)
            .map(|_| true)
            .map_err(Into::into)
    }

    fn name(&self) -> &str {
        V::NAME
    }
}
```

**Применение**: Когда нужен trait object, но trait имеет associated types (не object-safe).

### Scoped Callback Pattern (RAII)
Lifetime-safe arena allocation с автоматической очисткой:

```rust
pub struct Guard<'a, T> {
    arena: &'a mut Arena<T>,
}

impl<'a, T> Drop for Guard<'a, T> {
    fn drop(&mut self) {
        self.arena.reset();
    }
}

impl<T> Arena<T> {
    pub fn scope<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut Guard<T>) -> R,
    {
        let mut guard = Guard { arena: self };
        f(&mut guard)
    }
}
```

**Применение**: Управление ресурсами с гарантией очистки.

## Тестирование

### Принципы тестирования
- **Компиляция прежде всего**: Все изменения должны компилироваться без ошибок
- **Тесты должны проходить**: Или быть задокументированы как известные проблемы
- **Не пропускать тесты**: Использовать `#[ignore]` только с четким объяснением

### Rust 2024 особенности тестов
```rust
// ❌ НЕПРАВИЛЬНО - unsized type в Optional
struct MinLength {
    min: usize,
}

impl TypedValidator for MinLength {
    type Input = str; // ❌ Приводит к Option<str> - invalid!
    type Output = ();
    type Error = ValidationError;
}

// ✅ ПРАВИЛЬНО - sized type
impl TypedValidator for MinLength {
    type Input = String; // ✅ Option<String> - valid
    type Output = ();
    type Error = ValidationError;
}

// Тест
#[test]
fn test_optional() {
    let validator = Optional::new(MinLength { min: 5 });
    let value = Some("hello".to_string()); // ✅ String, не &str
    assert!(validator.validate(&value).is_ok());
}
```

### Type annotations для complex generics
```rust
// ❌ НЕПРАВИЛЬНО - Rust 2024 не может вывести типы
let validator = named_field("age", MinValue { min: 18 }, get_age);

// ✅ ПРАВИЛЬНО - явные аннотации
let validator: Field<TestUser, u32, _, _> =
    named_field("age", MinValue { min: 18 }, get_age);
```

## Стиль кода

### Документация
- Публичные API должны иметь doc comments
- Сложные алгоритмы требуют объяснения
- Примеры использования в doc tests

### Error handling
- Использовать `Result<T, E>` для recoverable errors
- Паниковать только для programming errors
- Предоставлять контекст в error messages

### Naming conventions
- `snake_case` для функций, переменных, модулей
- `CamelCase` для типов, traits
- `SCREAMING_SNAKE_CASE` для констант
- Префикс `_` для неиспользуемых переменных

## Workflow

### Перед коммитом
```bash
# 1. Проверка компиляции
cargo check --all-features

# 2. Запуск тестов
cargo test --workspace

# 3. Форматирование
cargo fmt --all

# 4. Clippy
cargo clippy --all-features -- -D warnings
```

### Code review checklist
- [ ] Код компилируется без ошибок
- [ ] Тесты проходят
- [ ] Документация обновлена
- [ ] Архитектурное решение правильное (не патч)
- [ ] Issue закрыт с полным описанием решения

## Дополнительные ресурсы

- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [Rust Design Patterns](https://rust-unofficial.github.io/patterns/)
- [Effective Rust](https://www.lurklurk.org/effective-rust/)
