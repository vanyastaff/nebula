# Nebula-Error: Refactoring Plan

## 🎯 Цель: Удобный, чистый, производительный код

### Текущие проблемы

1. ❌ **500+ строк дублированного кода** в `error.rs`
2. ❌ **Смешанная структура** - V1, V2, макросы в одной куче
3. ❌ **Избыточные `impl Into<String>`** - можно упростить до `&str` + `String`
4. ❌ **Неясная организация** - что устарело, что актуально?

---

## ✅ Предлагаемая структура

```
nebula-error/
├── src/
│   ├── lib.rs           # Чистые re-exports, prelude
│   │
│   ├── v1/              # СТАБИЛЬНЫЙ API (текущий)
│   │   ├── mod.rs       
│   │   ├── error.rs     # NebulaError
│   │   ├── kinds.rs     # ErrorKind (11 variants)
│   │   ├── context.rs   # ErrorContext
│   │   └── result.rs    # Result extensions
│   │
│   ├── v2/              # ОПТИМИЗИРОВАННЫЙ API (новый)
│   │   ├── mod.rs
│   │   ├── error.rs     # NebulaErrorV2 (48 bytes)
│   │   ├── kinds.rs     # ErrorKindV2 (4 categories)
│   │   ├── context.rs   # ErrorContextV2 (integer IDs)
│   │   └── bitflags.rs  # ErrorFlags
│   │
│   ├── common/          # ОБЩИЙ КОД
│   │   ├── mod.rs
│   │   ├── macros.rs    # Макросы (validation_error!, etc)
│   │   ├── retry.rs     # RetryStrategy (оба API)
│   │   ├── traits.rs    # Общие traits
│   │   └── conversion.rs # Конверсии из std/3rd party
│   │
│   └── utils/           # УТИЛИТЫ
│       ├── mod.rs
│       └── size_analysis.rs  # Профилирование
│
├── benches/
│   ├── v1_benchmarks.rs      # Бенчмарки V1
│   ├── v2_benchmarks.rs      # Бенчмарки V2
│   └── comparison.rs         # V1 vs V2
│
├── docs/
│   ├── MIGRATION_GUIDE.md    # V1 → V2
│   ├── BEST_PRACTICES.md     # Паттерны использования
│   └── ARCHITECTURE.md       # Архитектура
│
└── examples/
    ├── basic_usage.rs
    ├── with_retry.rs
    ├── custom_errors.rs
    └── migration_v1_to_v2.rs
```

---

## 🔨 Конкретные улучшения

### 1. Упрощение `impl Into<String>`

**Было:**
```rust
pub fn validation(message: impl Into<String>) -> Self
pub fn not_found(resource_type: impl Into<String>, resource_id: impl Into<String>) -> Self
```

**Станет:**
```rust
// Перегрузка для удобства
pub fn validation(message: &str) -> Self { ... }
pub fn validation_owned(message: String) -> Self { ... }

// Или используем From trait
impl From<&str> for NebulaError {
    fn from(s: &str) -> Self {
        Self::validation(s)
    }
}
```

### 2. Удаление дублирования через макрос

**Было: 500 строк**
```rust
pub fn validation(...) -> Self { Self::new(...) }
pub fn not_found(...) -> Self { Self::new(...) }
pub fn permission_denied(...) -> Self { Self::new(...) }
// ... ещё 60 функций
```

**Станет: 100 строк**
```rust
define_constructors! {
    client => {
        validation(message: &str),
        not_found(resource_type: &str, resource_id: &str),
        permission_denied(operation: &str, resource: &str),
        authentication(reason: &str),
    },
    server => {
        internal(message: &str),
        service_unavailable(service: &str, reason: &str),
    },
    system => {
        timeout(operation: &str, duration: Duration),
        network(message: &str),
        database(message: &str),
    },
}
```

### 3. Понятные имена модулей

**Было:**
- `src/core/` - что это core? V1 или V2?
- `src/kinds/` - какие kinds? Для V1 или V2?
- `src/optimized.rs` - это что, экспериментально?

**Станет:**
- `src/v1/` - стабильный API
- `src/v2/` - новый optimized API  
- `src/common/` - общие компоненты
- `src/utils/` - вспомогательные инструменты

### 4. Чистый публичный API

**lib.rs станет:**
```rust
// Стабильный V1 API (по умолчанию)
pub use v1::{NebulaError, Result, ErrorKind, ErrorContext};

// Оптимизированный V2 API
pub mod v2 {
    pub use crate::v2::*;
}

// Общие компоненты
pub use common::{
    RetryStrategy, Retryable, retry,
    validation_error, internal_error, ensure,
};

// Prelude для удобства
pub mod prelude {
    pub use crate::v1::*;
    pub use crate::common::macros::*;
}
```

---

## 📊 План выполнения

### Шаг 1: Реорганизация (1 день)
- [ ] Создать `src/v1/` и перенести текущий код
- [ ] Создать `src/v2/` и перенести optimized
- [ ] Создать `src/common/` для макросов и retry
- [ ] Обновить `lib.rs` с чистыми exports

### Шаг 2: Упрощение (1 день)
- [ ] Заменить `impl Into<String>` на `&str` где возможно
- [ ] Удалить unused код
- [ ] Применить макрос для конструкторов
- [ ] Исправить clippy warnings

### Шаг 3: Документация (0.5 дня)
- [ ] Обновить README с новой структурой
- [ ] Добавить примеры в `examples/`
- [ ] Написать migration guide
- [ ] Обновить doc comments

### Шаг 4: Валидация (0.5 дня)
- [ ] Запустить все тесты
- [ ] Проверить benchmarks
- [ ] Cargo clippy without warnings
- [ ] Cargo doc без ошибок

---

## 🎨 Примеры улучшенного API

### Упрощенные конструкторы

```rust
// Простой случай - статическая строка
let err = NebulaError::validation("Invalid email");

// Динамический случай
let err = NebulaError::validation_fmt(format!("Invalid {}", field));

// Или через макрос (ещё проще)
let err = validation_error!("Invalid email");
let err = validation_error!("Invalid {}", field);  // Auto-format
```

### Понятная структура импортов

```rust
// V1 (стабильный)
use nebula_error::{NebulaError, Result};

// V2 (optimized)
use nebula_error::v2::{NebulaErrorV2, Result};

// Общие макросы
use nebula_error::prelude::*;
```

### Чистая документация

```rust
/// Create a validation error
///
/// # Examples
///
/// ```rust
/// use nebula_error::NebulaError;
///
/// let err = NebulaError::validation("Invalid email format");
/// assert!(!err.is_retryable());
/// ```
///
/// # Performance
///
/// This uses `Cow<'static, str>` internally, so static strings
/// have zero allocations.
pub fn validation(message: &str) -> Self {
    // Simplified implementation
}
```

---

## 🚀 Хотите начать рефакторинг?

Я могу:
1. **Быстрая чистка** - удалить мёртвый код, исправить warnings (~30 мин)
2. **Средний рефакторинг** - реорганизовать в v1/v2/common (~2 часа)
3. **Полный рефакторинг** - всё выше + макросы + примеры (~4 часа)

Что предпочитаете?
