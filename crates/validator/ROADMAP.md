# nebula-validator: Long-Term Roadmap

> **Статус:** Living document
> **Создан:** 2026-02-16
> **Горизонт:** 12-18 месяцев
> **Принцип:** Лучше меньше, но качественно. Каждая фаза — завершённый, production-ready результат.

---

## 1. Видение финального продукта

`nebula-validator` — фреймворк валидации данных для workflow-движка Nebula. Единая точка входа для проверки параметров, входов action'ов и workflow-определений.

### Что это НЕ является

- **Не JSON Schema движок.** Крейт реализует value-level валидаторы, а не спецификацию JSON Schema. `nebula-config` имеет свой `SchemaValidator` для JSON Schema traversal, но делегирует format/constraint проверки сюда.
- **Не ORM/формы.** Фокус — на `serde_json::Value` в контексте workflow pipeline'а.
- **Не i18n фреймворк.** Локализация ошибок — ответственность UI-слоя. Крейт предоставляет error codes и параметры для программной обработки.
- **Не конкурент `validator` (crates.io).** Тот крейт валидирует Rust-структуры через derive. Наш крейт валидирует `serde_json::Value` в runtime workflow-движка.

### Что это является

```
┌─────────────────────────────────────────────────────────┐
│                      nebula-engine                       │
│  Резолвит параметры → collection.validate() → action    │
│  Владеет ExpressionEngine для Custom правил             │
└──────────────────────┬──────────────────────────────────┘
                       │
┌──────────────────────▼────────────────────────────────┐
│                    nebula-action                       │
│  ProcessAction::validate_input() — per-action хук     │
│  ParameterCollection описывает ожидаемые параметры    │
└──────────────────────┬────────────────────────────────┘
                       │
┌──────────────────────▼────────────────────────────────┐
│                   nebula-parameter                     │
│  ParameterCollection::validate()                      │
│    └─ для каждого ParameterDef → rule.validate(&val)  │
│                                                       │
│  ValidationRule::validate(&self, &Value) → Result     │
│    └─ делегирует в nebula-validator                   │
└──────────────────────┬────────────────────────────────┘
                       │ depends on
                       │
┌──────────────────────┤────────────────────────────────┐
│                      │                                │
│  nebula-config       │                                │
│  SchemaValidator     │                                │
│    validate_string() ─┤                               │
│    validate_number() ─┤ делегируют в                  │
│    validate_format() ─┤ nebula-validator              │
│                      │                                │
│  JSON Schema обход — │ остаётся в config              │
│  (type, required,    │                                │
│   $ref, enum, const) │                                │
└──────────────────────┤────────────────────────────────┘
                       │ depends on
┌──────────────────────▼──────────────────────────────────┐
│                  nebula-validator                        │
│                                                         │
│  ┌─────────────┐ ┌──────────────┐ ┌──────────────────┐ │
│  │  Validate    │ │ Combinators  │ │ AsValidatable    │ │
│  │  trait       │ │ And/Or/Not   │ │ (Value → typed)  │ │
│  └──────┬──────┘ └──────┬───────┘ └────────┬─────────┘ │
│         │               │                  │           │
│  ┌──────▼───────────────▼──────────────────▼─────────┐ │
│  │              Validator Categories                  │ │
│  │  - String: length, pattern, format                │ │
│  │  - Numeric: min, max, range, properties           │ │
│  │  - Collection: size, elements                     │ │
│  │  - Network: IP, port, MAC                         │ │
│  │  - Logical: boolean, nullable                     │ │
│  └───────────────────────────────────────────────────┘ │
│                                                         │
│  ┌───────────────┐ ┌────────────┐ ┌─────────────────┐ │
│  │ JsonField     │ │ Cached     │ │ Macros          │ │
│  │ (JSON Pointer)│ │ (moka LRU) │ │ validator!() etc│ │
│  └───────────────┘ └────────────┘ └─────────────────┘ │
│                                                         │
│  ┌───────────┐ ┌──────────┐ ┌─────────┐               │
│  │ Refined   │ │ TypeState│ │ Async   │               │
│  │ (Phase 7) │ │ (Phs. 7) │ │ (Phs. 8)│               │
│  └───────────┘ └──────────┘ └─────────┘               │
└─────────────────────────────────────────────────────────┘
```

**Ключевая проблема, которую решает крейт:**

Сейчас логика валидации дублируется по workspace:

- `nebula-validator` имеет 22K LOC валидаторов, которые никто не использует
- `evaluate_rule()` в parameter — 100 LOC inline match, дублирует min/max/length/pattern
- `SchemaValidator` в config — 240 LOC inline проверок (minLength, maxLength, pattern, minimum, maximum, multipleOf) + 40 LOC format validators (email, url, ipv4, ipv6, uuid, date, datetime, time, hostname) — всё то же самое что в nebula-validator
- Итого: 3 места с одинаковой логикой, разными edge cases, разными сообщениями об ошибках

Целевое состояние: одна валидационная логика в `nebula-validator`, используемая из parameter и config.

```
Сейчас:
  parameter: evaluate_rule() [inline match, 100 LOC]
  config:    validate_string() + validate_number() + validate_string_format() [280 LOC]
  validator: 60+ валидаторов [22K LOC, 0 потребителей]

Целевое:
  parameter: rule.validate(&value) → nebula-validator
  config:    SchemaValidator → nebula-validator для format/constraint checks
  validator: единый source of truth
```

---

## 2. Финальная архитектура (target state)

### 2.1 Структура файлов

```
crates/validator/
├── Cargo.toml
├── ROADMAP.md
├── README.md
├── src/
│   ├── lib.rs                     # pub mod foundation/validators/combinators/json/prelude
│   │
│   ├── foundation/                # Фундаментальные трейты и типы (бывший core/)
│   │   ├── mod.rs                 # Реэкспорты, утилиты
│   │   ├── traits.rs              # Validate, AsyncValidate, ValidateExt
│   │   ├── error.rs               # ValidationError, ValidationErrors, ErrorSeverity
│   │   ├── context.rs             # ValidationContext (cross-field)
│   │   ├── metadata.rs            # ValidatorMetadata (introspection)
│   │   ├── refined.rs             # Refined<T,V> compile-time proof [Phase 7]
│   │   ├── state.rs               # Type-state Parameter<T,S> [Phase 7]
│   │   ├── category.rs            # Error category taxonomy
│   │   └── validatable.rs         # AsValidatable trait + serde_json::Value impls
│   │
│   ├── validators/                # 60+ built-in валидаторов
│   │   ├── mod.rs                 # Реэкспорт всех категорий
│   │   ├── string/                # 17 string валидаторов
│   │   │   ├── length.rs          # MinLength, MaxLength, ExactLength, LengthRange
│   │   │   ├── pattern.rs         # Contains, StartsWith, EndsWith, MatchesRegex
│   │   │   ├── content.rs         # Email, Url
│   │   │   ├── uuid.rs            # UUID (RFC 4122)
│   │   │   ├── datetime.rs        # ISO 8601 DateTime + date_only() mode
│   │   │   ├── time.rs            # TimeOnly — HH:MM:SS [NEW]
│   │   │   ├── json.rs            # JSON structure
│   │   │   ├── password.rs        # Password strength
│   │   │   ├── phone.rs           # Phone numbers
│   │   │   ├── credit_card.rs     # Luhn algorithm
│   │   │   ├── iban.rs            # IBAN
│   │   │   ├── semver.rs          # Semantic versioning
│   │   │   ├── slug.rs            # URL slug
│   │   │   ├── hex.rs             # Hex strings
│   │   │   ├── base64.rs          # Base64 encoding
│   │   │   └── mod.rs
│   │   ├── numeric/               # 6 numeric валидаторов
│   │   │   ├── range.rs           # Min, Max, InRange, GreaterThan, LessThan
│   │   │   ├── properties.rs      # Positive, Negative, NonZero, Even, Odd
│   │   │   ├── divisibility.rs    # DivisibleBy, MultipleOf
│   │   │   ├── float.rs           # Finite, NotNan, DecimalPlaces
│   │   │   ├── percentage.rs      # Percentage (0.0..1.0)
│   │   │   └── mod.rs
│   │   ├── collection/            # 4 collection валидатора
│   │   │   ├── size.rs            # MinSize, MaxSize, ExactSize, SizeRange
│   │   │   ├── elements.rs        # All, Any, None, Count, Contains
│   │   │   ├── structure.rs       # HasKey
│   │   │   └── mod.rs
│   │   ├── network/               # 4 network валидатора
│   │   │   ├── ip_address.rs      # IpAddress, Ipv4, Ipv6
│   │   │   ├── hostname.rs        # Hostname (RFC 1123) [NEW]
│   │   │   ├── port.rs            # Port number
│   │   │   ├── mac_address.rs     # MAC address
│   │   │   └── mod.rs
│   │   └── logical/               # 2 logical валидатора
│   │       ├── boolean.rs         # IsTrue, IsFalse
│   │       ├── nullable.rs        # Required, NotNull
│   │       └── mod.rs
│   │
│   ├── combinators/               # 17 комбинаторов
│   │   ├── mod.rs                 # Реэкспорт, algebraic law tests
│   │   ├── and.rs                 # AND (short-circuit)
│   │   ├── or.rs                  # OR (any pass)
│   │   ├── not.rs                 # NOT (logical negation)
│   │   ├── optional.rs            # Optional<T> для Option<_>
│   │   ├── when.rs                # Conditional validation
│   │   ├── unless.rs              # Inverse conditional
│   │   ├── each.rs                # Collection iteration
│   │   ├── lazy.rs                # Deferred init (OnceLock)
│   │   ├── cached.rs              # LRU caching (moka) [#[cfg(feature = "caching")]]
│   │   ├── field.rs               # Field-specific validation
│   │   ├── nested.rs              # Nested object validation
│   │   ├── json_field.rs          # JSON Pointer (RFC 6901) [КЛЮЧЕВОЙ]
│   │   ├── message.rs             # Custom messages и error codes
│   │   ├── map.rs                 # Output mapping
│   │   ├── error.rs               # CombinatorError
│   │   ├── optimizer.rs           # ValidatorChainOptimizer [#[cfg(feature = "optimizer")]]
│   │   └── category.rs            # Category utilities
│   │
│   ├── json.rs                    # [NEW] JSON convenience: json_min_size(), json_max_size()
│   │
│   ├── prelude.rs                 # [NEW] Crate-level prelude: traits + validators + json
│   │
│   ├── macros/
│   │   └── mod.rs                 # validator!, validate!, validator_fn!, etc.
│   │
│   └── testing/                   # [Phase 6] Утилиты для тестирования
│       ├── mod.rs                 # MockValidator, TestHarness
│       └── fixtures.rs            # Готовые фикстуры
│
├── tests/
│   ├── integration_test.rs        # Базовые интеграционные
│   ├── combinator_error_test.rs   # Ошибки комбинаторов
│   ├── json_integration.rs        # JSON валидация
│   ├── validation_context_test.rs # Cross-field
│   ├── refined_test.rs            # Refined types
│   ├── optimizer_test.rs          # Optimizer
│   └── asvalidatable_test.rs      # Value → typed conversions
│
├── benches/
│   ├── mod.rs
│   ├── string_validators.rs       # String benchmark
│   └── combinators.rs             # Combinator benchmark
│
└── examples/
    ├── validator_basic_usage.rs   # Простой валидатор
    ├── combinators.rs             # Комбинаторы
    └── json_validation.rs         # JSON валидация
```

### 2.2 Ключевые трейты (текущее состояние)

```rust
/// Основной трейт валидации.
/// Generic по Input — compile-time type safety.
pub trait Validate {
    /// Тип входных данных (может быть ?Sized для str, [T]).
    type Input: ?Sized;

    /// Проверить input. Возвращает Ok(()) или ValidationError.
    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError>;

    /// Валидирует любой тип, конвертируемый в Self::Input.
    /// КЛЮЧЕВОЙ МЕТОД: позволяет валидировать serde_json::Value напрямую.
    ///
    /// min_length(5).validate_any(&json!("hello"))  // Value → &str автоматически
    /// min(1.0).validate_any(&json!(42))            // Value → f64 автоматически
    fn validate_any<S>(&self, value: &S) -> Result<(), ValidationError>
    where
        S: AsValidatable<Self::Input> + ?Sized;

    /// Метаданные валидатора (имя, сложность, теги).
    fn metadata(&self) -> ValidatorMetadata { ... }
}

/// Trait для конвертации типов перед валидацией.
/// Реализован для serde_json::Value → str, f64, i64, bool, [Value].
/// При type mismatch возвращает ValidationError с code "type_mismatch".
pub trait AsValidatable<T: ?Sized> {
    type Output<'a>: Borrow<T> where Self: 'a;
    fn as_validatable(&self) -> Result<Self::Output<'_>, ValidationError>;
}

/// Extension trait для композиции валидаторов.
pub trait ValidateExt: Validate {
    fn and<V>(self, other: V) -> And<Self, V> { ... }
    fn or<V>(self, other: V) -> Or<Self, V> { ... }
    fn not(self) -> Not<Self> { ... }
    fn when<C>(self, condition: C) -> When<Self, C> { ... }
    fn optional(self) -> Optional<Self> { ... }
    #[cfg(feature = "caching")]
    fn cached(self) -> Cached<Self> { ... }
}

/// Async-версия для I/O-зависимых валидаций.
/// Нет реализаций — отложено до Phase 8.
pub trait AsyncValidate {
    type Input: ?Sized;
    fn validate(&self, input: &Self::Input)
        -> impl Future<Output = Result<(), ValidationError>> + Send;
}
```

**`AsValidatable` реализации для `serde_json::Value` (уже существуют):**

| Value variant | Target type | Conversion |
|---|---|---|
| `Value::String(s)` | `str` | `s.as_str()` |
| `Value::Number(n)` | `f64` | `n.as_f64()` |
| `Value::Number(n)` | `i64` | `n.as_i64()` |
| `Value::Bool(b)` | `bool` | `*b` |
| `Value::Array(arr)` | `[Value]` | `arr.as_slice()` |
| Любой другой тип | — | `Err(ValidationError { code: "type_mismatch", ... })` |

Это позволяет `validate_any(&json_value)` работать без ручного извлечения типов.

### 2.3 Структурированные ошибки

```rust
/// Ошибка валидации с кодами, параметрами, вложенностью.
/// Cow<'static, str> — zero-alloc для статических строк.
pub struct ValidationError {
    /// Код ошибки для программной обработки: "min_length", "required"
    pub code: Cow<'static, str>,

    /// Человекочитаемое сообщение (English)
    pub message: Cow<'static, str>,

    /// Путь к полю: "user.email", "items[0].name"
    pub field: Option<Cow<'static, str>>,

    /// Параметры: [("min", "5"), ("actual", "3")]
    pub params: Vec<(Cow<'static, str>, Cow<'static, str>)>,

    /// Вложенные ошибки для объектов с несколькими полями
    pub nested: Vec<ValidationError>,

    /// Severity: Error, Warning, Info
    pub severity: ErrorSeverity,

    /// Help text
    pub help: Option<Cow<'static, str>>,
}
```

### 2.4 Интеграция с parameter (target state)

Вместо bridge/adapter паттернов — прямая зависимость. Один workspace, одна платформа.

```rust
// crates/parameter/Cargo.toml
[dependencies]
nebula-validator = { path = "../validator" }
```

```rust
// crates/parameter/src/validation.rs
use nebula_validator::prelude::*;

impl ValidationRule {
    /// Валидирует serde_json::Value против этого правила.
    /// validate_any() автоматически извлекает typed value из Value.
    pub fn validate(&self, value: &serde_json::Value) -> Result<(), ParameterError> {
        let result = match self {
            Self::MinLength { length, .. } =>
                min_length(*length).validate_any(value),
            Self::MaxLength { length, .. } =>
                max_length(*length).validate_any(value),
            Self::Pattern { pattern, .. } =>
                matches_regex(pattern)
                    .map_err(|e| ParameterError::invalid_pattern(pattern, e))?
                    .validate_any(value),
            Self::Min { value: min, .. } =>
                min(*min).validate_any(value),
            Self::Max { value: max, .. } =>
                max(*max).validate_any(value),
            Self::MinItems { count, .. } =>
                json_min_size(*count).validate_any(value),
            Self::MaxItems { count, .. } =>
                json_max_size(*count).validate_any(value),
            Self::OneOf { values, .. } => {
                if values.contains(value) { Ok(()) }
                else { Err(ValidationError::new("one_of", "value not in allowed set")) }
            }
            Self::Custom { .. } => Ok(()), // Engine handles via ExpressionEngine
        };

        let message = match self {
            Self::MinLength { message, .. } | Self::MaxLength { message, .. }
            | Self::Pattern { message, .. } | Self::Min { message, .. }
            | Self::Max { message, .. } | Self::OneOf { message, .. }
            | Self::Custom { message, .. } | Self::MinItems { message, .. }
            | Self::MaxItems { message, .. } => message,
        };

        result.map_err(|e| to_param_error(e, message))
    }
}
```

**Ключевое отличие от предыдущей версии:**

- `validate_any(value)` вместо ручного `value.as_str().ok_or(...)? + .validate(s)`
- `AsValidatable<str> for Value` автоматически извлекает `&str` и генерирует `type_mismatch` ошибку
- `json_min_size()` / `json_max_size()` вместо `min_size::<Value>()` с turbofish
- `matches_regex()` возвращает `Result` — обработка невалидного regex

Существующий `evaluate_rule()` в `collection.rs` заменяется на `rule.validate(value)`:

```rust
// crates/parameter/src/collection.rs — из 100 LOC inline → 5 LOC
fn evaluate_rule(
    rule: &ValidationRule,
    value: &Value,
    path: &str,
    errors: &mut Vec<ParameterError>,
) {
    if matches!(rule, ValidationRule::Custom { .. }) {
        return; // Engine handles Custom separately
    }
    if let Err(e) = rule.validate(value) {
        errors.push(e.with_path(path));
    }
}
```

### 2.5 Data Flow (целевое состояние)

```
1. Workflow Definition загружен

2. validate_workflow() — структурная проверка
   └─ Для литеральных значений: rule.validate(&literal)

3. Per-node execution:
   a. ParamResolver резолвит выражения → serde_json::Value
   b. collection.validate(&resolved_values):
      └─ Для каждого ParameterDef:
         - Required check
         - Type check
         - Для каждого ValidationRule: rule.validate(&value)
         - Рекурсия для Object/List
   c. Engine: для Custom правил → ExpressionEngine
   d. При ошибке → node failed
   e. action.validate_input() → per-action хук
   f. action.execute()
```

Никакого glue-кода в engine. Engine вызывает `collection.validate()` — и всё работает.

---

## 3. Roadmap по фазам

### Соглашения

- **Каждая фаза — законченный результат.** После каждой фазы крейт можно использовать.
- **Exit criteria** — что должно быть выполнено, чтобы считать фазу завершённой.
- **Quality gate** — автоматические проверки (CI), которые должны проходить.

---

### Phase 0: Foundation & Restructuring

> **Цель:** Оценить текущий код, убрать мёртвый код, подготовить крейт к интеграции.
> **Длительность:** 2-3 недели
> **Результат:** Чистая кодовая база, feature flags, готовность к использованию из parameter.

#### 0.1 Dead code audit

| Проверить | Действие |
|-----------|----------|
| `Refined<T,V>` в `core/refined.rs` | Если 0 реальных потребителей — gate за `type-state` feature |
| `Parameter<T,S>` в `core/state.rs` | Если 0 реальных потребителей — gate за `type-state` feature |
| `ValidatorStatistics` в `core/metadata.rs` | Если ничего не читает — удалить |
| `RegisteredValidatorMetadata` | Если ничего не читает — удалить |
| `ValidatorChainOptimizer` в `combinators/optimizer.rs` | Если 0 вызовов — gate за `optimizer` feature |
| `Cached` combinator (зависимость `moka`) | Оставить, но gate за `caching` feature |
| `AsValidatable` GAT в `core/validatable.rs` | Оставить — ключевой для интеграции с `serde_json::Value` |

#### 0.2 Rename `core/` → `foundation/`

Модуль `core` потенциально shadowing Rust's `core` crate в glob imports и макросах. Переименовать:

```bash
git mv crates/validator/src/core crates/validator/src/foundation
```

- Обновить все `use crate::core::` → `use crate::foundation::`
- В `lib.rs`: `pub mod foundation;`
- Deprecated re-export для миграции: `#[deprecated] pub use foundation as core;`
- 0 внешних потребителей → deprecated alias убирается сразу или в следующей фазе

#### 0.3 Добавить crate-level prelude

```rust
// crates/validator/src/prelude.rs
pub use crate::foundation::{
    Validate, ValidateExt, AsValidatable,
    ValidationError, ValidationErrors,
};

// String validators (factory functions)
pub use crate::validators::string::{
    min_length, max_length, exact_length, length_range, not_empty,
    contains, starts_with, ends_with,
    alphabetic, alphanumeric, numeric, lowercase, uppercase,
    email, url, matches_regex,
    DateTime, Uuid, Semver, Slug,
};

// Numeric validators (factory functions)
pub use crate::validators::numeric::{
    min, max, in_range, greater_than, less_than,
    divisible_by,
};

// JSON convenience (collection without turbofish)
#[cfg(feature = "serde")]
pub use crate::json::*;
```

**Потребитель пишет одну строку:**

```rust
use nebula_validator::prelude::*;

// И сразу работает:
min_length(5).validate_any(&json!("hello"))?;
min(0.0).validate_any(&json!(42))?;
json_min_size(1).validate_any(&json!([1, 2]))?;
```

#### 0.4 Добавить JSON convenience module

Collection validators generic по `T` — для JSON нужен turbofish `min_size::<serde_json::Value>(3)`. Неудобно. Добавить:

```rust
// crates/validator/src/json.rs
#[cfg(feature = "serde")]
use crate::validators::collection;

pub type JsonMinSize = collection::MinSize<serde_json::Value>;
pub type JsonMaxSize = collection::MaxSize<serde_json::Value>;

pub fn json_min_size(min: usize) -> JsonMinSize { collection::min_size(min) }
pub fn json_max_size(max: usize) -> JsonMaxSize { collection::max_size(max) }
pub fn json_exact_size(size: usize) -> collection::ExactSize<serde_json::Value> {
    collection::exact_size(size)
}
pub fn json_size_range(min: usize, max: usize) -> collection::SizeRange<serde_json::Value> {
    collection::size_range(min, max)
}
```

#### 0.5 Feature flags

```toml
[features]
default = ["serde"]
serde = []
caching = ["dep:moka"]           # Cached combinator (moka LRU)
optimizer = []                    # ValidatorChainOptimizer, ValidatorStatistics
full = ["caching", "optimizer"]

[dependencies]
moka = { version = "0.12", features = ["sync"], optional = true }
```

**Что гейтится:**

| Feature | Гейтит | Зависимость |
|---|---|---|
| `serde` (default) | `json` module, `AsValidatable<_> for Value`, `json_field` | — |
| `caching` | `cached.rs`, `.cached()` на `ValidateExt` | `moka` |
| `optimizer` | `optimizer.rs`, `ValidatorStatistics` | — |

**Принцип:** `default` + `full`, без fine-grained per-category features. Unused валидаторы элиминируются линкером — cost of inclusion = 0 runtime.

#### 0.6 Dependency cleanup

- `moka` → optional, за `caching` feature
- `base64`, `regex`, `url`, `uuid`, `serde`, `serde_json` — остаются (нужны для валидаторов)

#### 0.7 Проверить AsValidatable реализации

`AsValidatable` для `serde_json::Value` — уже реализованы и протестированы:

- `Value::String` → `&str` ✅ (работает)
- `Value::Number` → `f64` ✅ (работает)
- `Value::Number` → `i64` ✅ (работает)
- `Value::Array` → `&[Value]` ✅ (работает)
- `Value::Bool` → `bool` ✅ (работает)
- Type mismatch → `ValidationError { code: "type_mismatch", params: [("expected", ...), ("actual", ...)] }` ✅

Проверить edge cases:
- `Value::Null` + string validator → type_mismatch (не panic)
- `Value::Number(3.14)` → `i64` → type_mismatch (float без целой части)
- Очень большие числа → overflow handling

#### 0.8 Добавить недостающие валидаторы

**Hostname** (для config format "hostname"):

```rust
// crates/validator/src/validators/network/hostname.rs
/// RFC 1123 hostname validation.
/// Длина 1..=253, labels 1..=63, [a-zA-Z0-9-], без leading/trailing hyphen.
pub struct Hostname;

impl Validate for Hostname {
    type Input = str;
    // ...
}

pub fn hostname() -> Hostname { Hostname }
```

**TimeOnly** (для config format "time"):

```rust
// crates/validator/src/validators/string/time.rs
/// Validates time-only strings: HH:MM:SS, HH:MM:SS.sss, с optional timezone.
pub struct TimeOnly {
    allow_milliseconds: bool,
    require_timezone: bool,
}

impl TimeOnly {
    pub fn new() -> Self { ... }
    pub fn require_timezone(mut self) -> Self { ... }
}

impl Validate for TimeOnly {
    type Input = str;
    // Переиспользует parsing logic из DateTime
}

pub fn time_only() -> TimeOnly { TimeOnly::new() }
```

**DateTime::date_only()** (для config format "date"):

```rust
// Добавить к существующему DateTime
impl DateTime {
    /// Только формат YYYY-MM-DD. Отклоняет строки с time component.
    pub fn date_only() -> Self {
        Self { allow_date_only: true, date_only_mode: true, .. }
    }
}
```

#### 0.9 Обновить lib.rs

```rust
// crates/validator/src/lib.rs (target)
#![allow(clippy::result_large_err)]
#![allow(clippy::type_complexity)]

pub mod foundation;
pub mod combinators;
pub mod validators;

#[cfg(feature = "serde")]
pub mod json;

pub mod prelude;
```

#### Exit criteria

- `core/` переименован в `foundation/`
- `pub mod prelude` экспортирует traits + factory functions + json convenience
- `pub mod json` экспортирует `json_min_size()`, `json_max_size()`, etc.
- `moka` dependency за `caching` feature flag
- `ValidatorChainOptimizer` за `optimizer` feature flag
- `Hostname` валидатор в `validators::network`
- `TimeOnly` валидатор в `validators::string`
- `DateTime::date_only()` builder method
- `cargo check -p nebula-validator` — OK
- `cargo check -p nebula-validator --no-default-features` — OK (moka не линкуется)
- `cargo test -p nebula-validator` — все проходят
- `cargo test -p nebula-validator --all-features` — все проходят
- `cargo clippy -p nebula-validator -- -D warnings` — OK

---

### Phase 1: Parameter Integration

> **Цель:** `ValidationRule` делегирует валидацию в `nebula-validator`. Единая логика.
> **Длительность:** 3-4 недели
> **Результат:** `nebula-parameter` зависит от `nebula-validator`, inline-логика заменена на вызовы валидаторов.

#### 1.1 Добавить зависимость parameter → validator

```toml
# crates/parameter/Cargo.toml
[dependencies]
nebula-validator = { path = "../validator" }
```

Оба крейта на Domain layer — зависимость допустима и естественна.

#### 1.2 ValidationRule::validate() через validate_any

Добавить метод `.validate()` на `ValidationRule`, используя `validate_any()`:

```rust
use nebula_validator::prelude::*;

impl ValidationRule {
    pub fn validate(&self, value: &serde_json::Value) -> Result<(), ParameterError> {
        let result = match self {
            Self::MinLength { length, .. } => min_length(*length).validate_any(value),
            Self::MaxLength { length, .. } => max_length(*length).validate_any(value),
            Self::Pattern { pattern, .. } =>
                matches_regex(pattern)
                    .map_err(|e| ParameterError::invalid_pattern(pattern, e))?
                    .validate_any(value),
            Self::Min { value: v, .. } => min(*v).validate_any(value),
            Self::Max { value: v, .. } => max(*v).validate_any(value),
            Self::MinItems { count, .. } => json_min_size(*count).validate_any(value),
            Self::MaxItems { count, .. } => json_max_size(*count).validate_any(value),
            Self::OneOf { values, .. } if values.contains(value) => Ok(()),
            Self::OneOf { .. } => Err(ValidationError::new("one_of", "value not in allowed set")),
            Self::Custom { .. } => Ok(()), // Engine handles via ExpressionEngine
        };
        result.map_err(|e| to_param_error(e, self.message()))
    }
}
```

**Ключевое:**
- `validate_any(value)` — автоматическое извлечение типа через `AsValidatable`. Нет ручного `value.as_str()`.
- `matches_regex()` возвращает `Result<MatchesRegex, regex::Error>` — обработка невалидного regex
- `json_min_size()` / `json_max_size()` — без turbofish
- `AsValidatable` сам генерирует `type_mismatch` если передать число в string validator
- Exhaustive match — компилятор ловит новые варианты

#### 1.3 Заменить evaluate_rule() в collection.rs

Существующая `evaluate_rule()` (100 LOC inline match) заменяется на:

```rust
fn evaluate_rule(
    rule: &ValidationRule,
    value: &Value,
    path: &str,
    errors: &mut Vec<ParameterError>,
) {
    if matches!(rule, ValidationRule::Custom { .. }) {
        return;
    }
    if let Err(e) = rule.validate(value) {
        errors.push(e.with_path(path));
    }
}
```

Из 100 строк inline-кода → 5 строк делегации.

#### 1.4 Конвертация ошибок

`nebula-validator` возвращает `ValidationError` (rich, structured, с `Cow<'static, str>`). `nebula-parameter` использует `ParameterError::ValidationError { key, reason }`. Конвертация:

```rust
fn to_param_error(
    err: nebula_validator::foundation::ValidationError,
    custom_message: &Option<String>,
) -> ParameterError {
    ParameterError::ValidationError {
        key: String::new(), // path устанавливается вызывающим кодом
        reason: custom_message.clone().unwrap_or_else(|| err.message.to_string()),
    }
}
```

Custom message из `ValidationRule` перекрывает сообщение валидатора — как и сейчас.

#### 1.5 Тесты

Все существующие тесты в `collection.rs` (40+ тестов) должны проходить без изменений — это regression suite. Дополнительно:

- Property tests: случайные `serde_json::Value` × `ValidationRule` — не паникует
- Edge cases: `Value::Null` + каждый rule → graceful error
- Невалидный regex в `Pattern` → graceful error (не panic)
- Type mismatch: string value + numeric rule → type_mismatch error

#### Exit criteria

- `nebula-parameter` зависит от `nebula-validator`
- `ValidationRule::validate()` обрабатывает все 9 вариантов
- Exhaustive match — без `_ =>` wildcard
- `Custom` пропускается (returns `Ok(())`)
- Все 40+ существующих тестов в collection.rs проходят
- >= 20 новых тестов на ValidationRule::validate()
- Property test: 1000 итераций без panic
- `cargo test --workspace` — OK (включая parameter + validator)

---

### Phase 2: Config Integration

> **Цель:** `SchemaValidator` делегирует constraint и format проверки в `nebula-validator`.
> **Длительность:** 2-3 недели
> **Результат:** `nebula-config` зависит от `nebula-validator`, inline-валидация заменена на вызовы валидаторов.

#### 2.1 Добавить зависимость config → validator

```toml
# crates/config/Cargo.toml
[dependencies]
nebula-validator = { path = "../validator" }
```

#### 2.2 Заменить validate_string() constraint checks

Текущий код (60 LOC inline minLength/maxLength/pattern) заменяется на:

```rust
fn validate_string(
    &self,
    s: &str,
    schema_obj: &serde_json::Map<String, Value>,
    path: &str,
) -> ConfigResult<()> {
    use nebula_validator::prelude::*;

    // minLength
    if let Some(min_val) = schema_obj.get("minLength").and_then(Value::as_u64) {
        min_length(min_val as usize).validate(s)
            .map_err(|e| to_config_error(e, path))?;
    }

    // maxLength
    if let Some(max_val) = schema_obj.get("maxLength").and_then(Value::as_u64) {
        max_length(max_val as usize).validate(s)
            .map_err(|e| to_config_error(e, path))?;
    }

    // pattern
    if let Some(pattern_str) = schema_obj.get("pattern").and_then(Value::as_str) {
        match matches_regex(pattern_str) {
            Ok(re) => re.validate(s).map_err(|e| to_config_error(e, path))?,
            Err(_) => nebula_log::warn!("Invalid regex pattern in schema: {}", pattern_str),
        }
    }

    // format
    if let Some(format_str) = schema_obj.get("format").and_then(Value::as_str) {
        self.validate_string_format(s, format_str, path)?;
    }

    Ok(())
}
```

#### 2.3 Заменить validate_number() constraint checks

Текущий код (60 LOC inline minimum/maximum/multipleOf) заменяется на:

```rust
fn validate_number(
    &self,
    n: &serde_json::Number,
    schema_obj: &serde_json::Map<String, Value>,
    path: &str,
) -> ConfigResult<()> {
    let value = n.as_f64().unwrap_or(0.0);

    use nebula_validator::prelude::*;

    // minimum (with exclusive support)
    if let Some(min_val) = schema_obj.get("minimum").and_then(Value::as_f64) {
        let exclusive = schema_obj.get("exclusiveMinimum")
            .and_then(|v| v.as_bool()).unwrap_or(false);
        if exclusive {
            greater_than(min_val).validate(&value).map_err(|e| to_config_error(e, path))?;
        } else {
            min(min_val).validate(&value).map_err(|e| to_config_error(e, path))?;
        }
    }

    // maximum (аналогично с max/less_than)
    // multipleOf
    if let Some(div) = schema_obj.get("multipleOf").and_then(Value::as_f64) {
        divisible_by(div).validate(&value).map_err(|e| to_config_error(e, path))?;
    }

    Ok(())
}
```

#### 2.4 Заменить format validators

9 inline format validators (40 LOC) заменяются на вызовы nebula-validator:

```rust
fn validate_string_format(&self, s: &str, format: &str, path: &str) -> ConfigResult<()> {
    use nebula_validator::validators::{string, network};
    use nebula_validator::foundation::Validate;

    let result = match format {
        "email"     => string::email().validate(s),
        "uri"|"url" => string::url().validate(s),
        "ipv4"      => network::Ipv4.validate(s),
        "ipv6"      => network::Ipv6.validate(s),
        "uuid"      => string::Uuid::new().validate(s),
        "date"      => string::DateTime::date_only().validate(s),
        "date-time" => string::DateTime::new().require_time().require_timezone().validate(s),
        "time"      => string::time_only().validate(s),
        "hostname"  => network::hostname().validate(s),
        _ => return Ok(()), // Unknown format, skip
    };

    result.map_err(|e| to_config_error(e, path))
}
```

**Что остаётся в config:** JSON Schema traversal (validate_recursive, validate_type, validate_object, validate_array, validate_ref, can_coerce, enum/const checks). Это ~500 LOC специфичной для JSON Schema логики — её нечем заменить и не нужно.

**Что уходит в validator:** format checks (~40 LOC), string constraints (~30 LOC), number constraints (~60 LOC). Итого ~130 LOC inline кода → вызовы валидаторов.

#### 2.5 Удалить дублирующие зависимости

После интеграции config может удалить:
- `chrono` — format validation (date, datetime, time) уходит в validator
- `uuid` — UUID format check уходит в validator
- `regex` — возможно останется для regex_cache, но кэширование можно убрать (validator кэширует сам)

Зависимости `chrono`, `uuid`, `regex` остаются транзитивно через `nebula-validator`.

#### 2.6 Конвертация ошибок

```rust
fn to_config_error(
    err: nebula_validator::foundation::ValidationError,
    path: &str,
) -> ConfigError {
    ConfigError::validation_error(
        format!("{} at path '{}'", err.message, path),
        Some(path.to_string()),
    )
}
```

#### Exit criteria

- `nebula-config` зависит от `nebula-validator`
- `validate_string_format()` делегирует 9 форматов в validator
- `validate_string()` делегирует minLength/maxLength/pattern в validator
- `validate_number()` делегирует minimum/maximum/multipleOf в validator
- JSON Schema traversal остаётся в config (validate_recursive, validate_object, etc.)
- Все существующие тесты SchemaValidator проходят
- `cargo test -p nebula-config` — OK
- `cargo test --workspace` — OK

---

### Phase 3: Engine Integration

> **Цель:** Подключить валидацию в execution pipeline движка.
> **Длительность:** 4-6 недель
> **Результат:** Resolved parameters валидируются ПЕРЕД вызовом action.

#### 3.1 Validation step в engine

В `crates/engine/src/engine.rs`, после `self.resolver.resolve()` и перед `action.execute()`:

```rust
// Engine execution flow:
let resolved = self.resolver.resolve(&node)?;
collection.validate(&resolved)?;  // <-- просто вызов, никакого glue
action.execute(resolved).await?;
```

`ParameterCollection::validate()` уже существует — engine просто вызывает его. Единственная новая логика — обработка ошибок.

#### 3.2 EngineError::ValidationFailed

```rust
// В crates/engine/src/error.rs
pub enum EngineError {
    // ... существующие варианты ...

    /// Resolved параметры не прошли валидацию.
    #[error("Parameter validation failed for node '{node_id}': {errors:?}")]
    ValidationFailed {
        node_id: String,
        errors: Vec<ParameterError>,
    },
}
```

#### 3.3 Custom expression evaluation

Engine — единственный компонент с `ExpressionEngine`. После `collection.validate()`:

```rust
// Для Custom правил — engine обрабатывает отдельно
for rule in custom_rules {
    if let ValidationRule::Custom { expression, message } = rule {
        let result = expression_engine.evaluate(expression, context).await?;
        if result != serde_json::Value::Bool(true) {
            errors.push(custom_validation_error(expression, message));
        }
    }
}
```

#### 3.4 Action-level validation

`ProcessAction::validate_input()` вызывается ПОСЛЕ parameter validation:

```
collection.validate() → Custom expression evaluation → action.validate_input() → execute()
```

#### 3.5 Integration tests

- Node с `MinLength(10)`, input = 3 chars → node failed с validation error
- Node с `Min(1.0)`, value = 0 → fails
- Node с валидными значениями → проходит к execution
- Node с `Custom { expression }` → expression evaluated
- Multiple validation errors собираются

#### Exit criteria

- Engine валидирует параметры перед execution
- `EngineError::ValidationFailed` содержит node_id и все ошибки
- `Custom` правила evaluateд через ExpressionEngine
- validate_input() вызывается после parameter validation
- Все существующие engine tests проходят
- >= 10 новых integration tests

---

### Phase 4: Workflow-Level Validation

> **Цель:** Ловить ошибки на этапе сохранения workflow, а не execution.
> **Длительность:** 3-4 недели
> **Результат:** `validate_workflow()` проверяет литеральные значения параметров.

#### 4.1 Static validation для ParamValue::Literal

В `crates/workflow/src/validate.rs`, расширить `validate_workflow()`:

1. Для каждой ноды с `ParamValue::Literal(value)`:
   - Получить `ParameterDef` из action registry
   - Если есть validation rules → `rule.validate(&value)` для каждого
   - Ошибки добавляются в `WorkflowError` с node name + parameter key
2. `ParamValue::Expression`, `ParamValue::Template`, `ParamValue::Reference` — пропускаются (невозможно проверить статически)

#### 4.2 Type checking для литералов

Помимо validation rules, проверять совместимость типов:
- `ParameterKind::Number` + `Value::String` → `WorkflowError::TypeMismatch`
- `ParameterKind::Text` + `Value::Number` → предупреждение (auto-coercion возможна)
- `ParameterKind::Select` + значение не из `SelectOption::value` → ошибка

#### 4.3 UI-friendly ошибки

- Каждая ошибка включает: node_name, parameter_key, human-readable reason
- Формат совместим с будущим UI-отображением (inline per-parameter errors)

#### Exit criteria

- `validate_workflow()` находит невалидные литералы
- Type mismatches обнаруживаются
- Expression/Template/Reference — пропускаются
- Ошибки содержат node + parameter path
- >= 15 новых тестов

---

### Phase 5: Rich Validation Rules

> **Цель:** Расширить набор правил ValidationRule, используя мощность nebula-validator.
> **Длительность:** 3-4 недели
> **Результат:** Action developers могут использовать 20+ типов правил вместо 9.

#### 5.1 Новые варианты ValidationRule

Сейчас ValidationRule имеет 9 вариантов. nebula-validator имеет 60+ валидаторов. Добавить наиболее востребованные:

```rust
pub enum ValidationRule {
    // Существующие (9)
    MinLength { .. }, MaxLength { .. }, Pattern { .. },
    Min { .. }, Max { .. }, OneOf { .. }, Custom { .. },
    MinItems { .. }, MaxItems { .. },

    // Новые — строки
    Email { message: Option<String> },
    Url { message: Option<String> },
    Uuid { message: Option<String> },
    NotEmpty { message: Option<String> },

    // Новые — числа
    InRange { min: f64, max: f64, message: Option<String> },
    Positive { message: Option<String> },
    Integer { message: Option<String> },

    // Новые — коллекции
    UniqueItems { message: Option<String> },
    ItemsInRange { min: usize, max: usize, message: Option<String> },
}
```

Каждый новый вариант добавляется по мере потребности action developers. Не добавлять впрок.

#### 5.2 Расширить ValidationRule::validate()

Каждый новый вариант делегирует в соответствующий валидатор:

```rust
Self::Email { message } => {
    let s = require_str(value)?;
    nebula_validator::validators::string::Email.validate(s)
        .map_err(|e| validation_error(e, message))
}
Self::InRange { min, max, message } => {
    let n = require_f64(value)?;
    nebula_validator::validators::numeric::in_range(*min, *max)
        .validate(&n)
        .map_err(|e| validation_error(e, message))
}
```

#### 5.3 Serde backward compatibility

Новые варианты добавляются с `#[serde(tag = "rule")]` — старые workflows с 9 оригинальными правилами десериализуются без изменений.

#### Exit criteria

- Минимум 5 новых вариантов `ValidationRule`
- Каждый делегирует в nebula-validator
- Serde round-trip для всех новых вариантов
- Backward compatibility: старые JSON десериализуются
- >= 20 тестов на новые правила

---

### Phase 6: Developer Experience

> **Цель:** Удобная разработка action'ов с валидацией.
> **Длительность:** 4-6 недель
> **Результат:** Тестирование, примеры, документация.

#### 6.1 Testing utilities

В `nebula-validator/src/testing/mod.rs`:

```rust
/// Mock валидатор для тестов — всегда Ok или всегда Err.
pub struct MockValidator { always_pass: bool }

/// Валидатор с failure injection.
pub struct FailOnNth { fail_on: usize, count: AtomicUsize }
```

В `nebula-parameter` — test helpers для action developers:

```rust
/// Test harness для action validation.
pub struct ValidationTestHarness {
    collection: ParameterCollection,
}

impl ValidationTestHarness {
    pub fn new() -> Self { ... }
    pub fn with_param(mut self, def: ParameterDef) -> Self { ... }
    pub fn assert_valid(&self, values: serde_json::Value) { ... }
    pub fn assert_invalid(&self, values: serde_json::Value, expected_codes: &[&str]) { ... }
}
```

#### 6.2 Error formatting

- `ParameterError` → user-friendly display
- `Vec<ParameterError>` → summary string с количеством ошибок и первыми 3
- Structured logging через `tracing::warn!` при validation failure

#### 6.3 Documentation

- `crates/validator/README.md` — обновить с примерами использования через parameter
- `crates/validator/docs/Validators.md` — таблица всех валидаторов с примерами
- `crates/parameter/docs/Validation.md` — как работает валидация параметров

#### 6.4 Examples

- `examples/json_validation.rs` — валидация serde_json::Value
- `examples/custom_validator.rs` — создание своего валидатора
- `examples/action_validation.rs` — ParameterCollection + validation

#### Exit criteria

- `MockValidator` и `FailOnNth` доступны из validator
- `ValidationTestHarness` позволяет тестировать action'ы
- Error formatting для user и log
- Документация соответствует коду 1:1
- Примеры компилируются и запускаются

---

### Phase 7: Advanced Type Safety

> **Цель:** Type-level guarantees для кода, использующего типизированные структуры.
> **Длительность:** 4-6 недель
> **Результат:** `Refined<T,V>` и type-state работают для typed action inputs.

#### 7.1 Refined types в продакшне

`Refined<T, V>` уже реализован. Задача — сделать его полезным:

```rust
type Port = Refined<u16, InRange<u16>>;
type Email = Refined<String, EmailValidator>;

struct HttpConfig {
    host: String,
    port: Port,
    admin_email: Email,
}
```

- Добавить `TryFrom<serde_json::Value>` для `Refined<T, V>` where T: Deserialize
- Интеграция с `ParameterDef` — auto-derive refined types из parameter schemas

#### 7.2 Type-state для action inputs

`Parameter<T, Unvalidated>` → `Parameter<T, Validated<V>>` уже реализован:

```rust
fn execute(
    &self,
    input: Parameter<HttpConfig, Validated<HttpConfigValidator>>,
) -> Result<ActionOutput, ActionError> {
    let config = input.into_inner(); // Гарантированно валидный
}
```

Полезно ТОЛЬКО когда action'ы перейдут на типизированные inputs. Если inputs остаются `serde_json::Value` — откладывается.

#### 7.3 Validator derivation

```rust
#[derive(Validate)]
struct HttpConfig {
    #[validate(min_length = 1)]
    host: String,

    #[validate(range(1, 65535))]
    port: u16,

    #[validate(email)]
    admin_email: String,
}
```

- Отдельный крейт `nebula-validator-derive` (proc macros)
- Генерирует `impl Validate for HttpConfig`

#### Exit criteria

- `Refined<T,V>` можно создать из `serde_json::Value`
- Type-state работает для typed action inputs
- `#[derive(Validate)]` генерирует корректный код
- >= 10 тестов на derive macros

---

### Phase 8: Async & Enterprise

> **Цель:** Поддержка async-валидации и enterprise use cases.
> **Длительность:** 6-8 недель
> **Результат:** Async валидация, batch validation.

#### 8.1 AsyncValidate implementations

Когда нужно: remote validation (API key проверка, uniqueness check в БД).

```rust
pub trait AsyncValidate {
    type Input: ?Sized;
    fn validate(&self, input: &Self::Input)
        -> impl Future<Output = Result<(), ValidationError>> + Send;
}
```

- `AsyncValidateExt` с `.and_async()`, `.or_async()`
- Timeout: configurable, default 5s
- Wire в engine: если action объявляет async validation, запускать с timeout

#### 8.2 Batch validation

Для bulk operations (import 10,000 records):

```rust
pub fn validate_batch(
    collection: &ParameterCollection,
    items: &[serde_json::Value],
) -> Vec<(usize, Vec<ParameterError>)>
```

- Возвращает (index, errors) для каждого невалидного значения
- Параллелизация через `rayon` (optional feature)

#### 8.3 Validation reports

Structured report для monitoring:

```rust
pub struct ValidationReport {
    pub total_checked: usize,
    pub passed: usize,
    pub failed: usize,
    pub errors_by_code: HashMap<String, usize>,
    pub duration: Duration,
}
```

#### Exit criteria

- AsyncValidate работает end-to-end с timeout
- Batch validation обрабатывает 10K records
- ValidationReport собирается из execution

---

## 4. Timeline (ориентировочный)

```
2026 Q1 (Feb-Apr):    Phase 0  — Foundation & Restructuring
2026 Q2 (Apr-May):    Phase 1  — Parameter Integration
2026 Q2 (May-Jun):    Phase 2  — Config Integration
2026 Q2-Q3 (Jun-Aug): Phase 3  — Engine Integration
2026 Q3 (Aug-Sep):    Phase 4  — Workflow-Level Validation
2026 Q3-Q4 (Sep-Nov): Phase 5  — Rich Validation Rules
2026 Q4 (Nov-Dec):    Phase 6  — Developer Experience
2027 Q1 (Jan-Feb):    Phase 7  — Advanced Type Safety
2027 Q1-Q2 (Feb-Apr): Phase 8  — Async & Enterprise
```

---

## 5. Architectural Decision Records

### ADR-1: Parameter зависит от validator напрямую

**Решение: `nebula-parameter` зависит от `nebula-validator`.**

Оба крейта на Domain layer. `ParameterCollection::validate()` уже существует и содержит inline-логику валидации (100 LOC `evaluate_rule()`). Эта логика дублирует валидаторы из `nebula-validator`. Прямая зависимость:

1. **Устраняет дублирование.** 100 LOC inline match → 5 LOC делегации.
2. **Единая логика.** Один bug fix — одно место. Не два match-блока с одинаковой семантикой.
3. **Естественная зависимость.** Parameter определяет правила, validator их реализует — это не coupling, это cohesion.
4. **Расширяемость.** Новые ValidationRule варианты (Email, Url, InRange) сразу получают реализацию из 60+ готовых валидаторов.

| Альтернатива | За | Против | Вердикт |
|---|---|---|---|
| A: Inline логика (текущее) | 0 зависимостей | Дублирование, 100 LOC, нет edge case coverage | Отклонено |
| B: Bridge в engine | Decoupling | Glue-код, третий участник, parameter по-прежнему дублирует | Отклонено |
| C: Adapter crate | Clean separation | Новый крейт для 50 LOC, overengineering | Отклонено |
| **D: parameter → validator** | **0 glue, 0 дублирования, natural cohesion** | **+1 dep для parameter** | **Принято** |

### ADR-2: ValidationRule.validate() — метод на enum

**Решение: `.validate()` — метод на `ValidationRule`, не free function и не factory.**

`ValidationRule` — это enum с 9 вариантами. Каждый вариант знает, какой валидатор вызвать. Метод на enum — simplest correct solution:

```rust
rule.validate(&value)?;
```

- **Discoverable.** IDE подсказывает `.validate()` сразу.
- **Exhaustive.** Компилятор ловит новые варианты.
- **Нет промежуточных типов.** Нет `ValueValidator` enum, нет `Box<dyn Validate>`, нет factory.
- **Естественный Rust.** Enum с методами — идиоматический паттерн.

### ADR-3: Config делегирует constraint/format checks в validator

**Решение: `nebula-config` зависит от `nebula-validator` для format и constraint проверок.**

`SchemaValidator` имеет два слоя логики:

1. **JSON Schema traversal** (~500 LOC): `validate_recursive`, `validate_type`, `validate_object`, `validate_array`, `validate_ref`, `can_coerce`, `enum`/`const` checks. Это специфичная для JSON Schema логика — остаётся в config.
2. **Value-level constraints** (~130 LOC): `validate_string()` (minLength/maxLength/pattern), `validate_number()` (minimum/maximum/multipleOf), `validate_string_format()` (email, url, ipv4, ipv6, uuid, date, datetime, time, hostname). Это ровно те же проверки, что реализует nebula-validator.

Config (System layer) → Validator (Domain layer) — допустимо, т.к. validator — утилитарная библиотека без бизнес-логики. Аналогично: config уже зависит от `regex`, `chrono`, `uuid` — nebula-validator заменяет эти прямые зависимости.

| Альтернатива | За | Против | Вердикт |
|---|---|---|---|
| A: Inline (текущее) | 0 зависимостей | 130 LOC дублирования, примитивный email check (`contains('@')`) | Отклонено |
| B: Только parameter → validator | Минимум изменений | Config продолжает дублировать | Отклонено |
| **C: config + parameter → validator** | **Единая логика, лучшие format validators** | **+1 dep для config** | **Принято** |

### ADR-4: Custom rules — engine обрабатывает отдельно

**Решение: `ValidationRule::Custom` возвращает `Ok(())` из `.validate()`.**

- `Custom` содержит expression (`{{ $value > $json.min_age }}`), для evaluation нужен ExpressionEngine.
- ExpressionEngine живёт в engine (Application layer).
- Parameter (Domain) не зависит от engine — поэтому Custom пропускается.
- Engine обрабатывает Custom правила отдельно, после `collection.validate()`.

### ADR-5: Feature flags — default + full

**Решение: Два уровня, без fine-grained per-category.**

- Fine-grained features → комбинаторный CI matrix (2^N combinations).
- Неиспользуемые валидаторы элиминируются линкером — runtime cost = 0.
- Зависимости крейта (regex, url, uuid, base64) уже используются транзитивно.
- `moka` — единственная "тяжёлая" зависимость → optional за `caching` feature.
- `ValidatorChainOptimizer` → optional за `optimizer` feature.

### ADR-6: AsyncValidate — defer

**Решение: 0 реализаций, 0 потребителей. Trait остаётся, инвестиции — Phase 8.**

- Текущие use cases — чистая синхронная логика.
- Async нужен для remote validation (API key check, DB uniqueness) — нет use cases до Phase 8.
- Trait signature (`-> impl Future + Send`) корректна — будет работать когда придёт время.

### ADR-7: validate_any — основной API для serde_json::Value

**Решение: Потребители используют `validate_any(&value)`, а не `validate(&extracted)`.**

- `validate_any()` уже реализован на `Validate` trait
- `AsValidatable<str/f64/i64/bool/[Value]> for serde_json::Value` — уже реализованы
- Автоматическое извлечение типа + генерация `type_mismatch` ошибки
- Потребителю не нужны хелперы `require_str()`, `require_f64()`, `type_mismatch()`

```rust
// Вместо:
let s = value.as_str().ok_or_else(|| type_mismatch("string", value))?;
min_length(5).validate(s)

// Просто:
min_length(5).validate_any(value)
```

### ADR-8: Rename core → foundation

**Решение: Модуль `core/` переименовывается в `foundation/`.**

- `nebula_validator::core` потенциально shadowing `core::` в glob imports и макросах
- 0 внешних потребителей → безболезненное переименование
- `foundation` точнее описывает содержимое (фундаментальные трейты, ошибки)
- Deprecated re-export `pub use foundation as core` для внутренней миграции

### ADR-9: JSON convenience module для collection validators

**Решение: `pub mod json` с pre-specialized функциями `json_min_size()`, `json_max_size()`, etc.**

- Collection validators generic по `T`: `MinSize<T>` с `Input = [T]`
- Для JSON нужен turbofish: `min_size::<serde_json::Value>(3)` — уродливо
- `json_min_size(3)` — type alias на `MinSize<serde_json::Value>`, zero-cost
- В prelude для удобства потребителей

---

## 6. Crate Dependency Impact

```
Текущее состояние:
  nebula-validator → (isolated, 0 dependents)
  nebula-parameter: inline evaluate_rule() дублирует логику
  nebula-config: inline format/constraint checks дублируют логику

После Phase 1:
  nebula-parameter ──→ nebula-validator (NEW: delegates validation)
  evaluate_rule() → rule.validate() → nebula-validator

После Phase 2:
  nebula-config ──→ nebula-validator (NEW: delegates format/constraint checks)
  validate_string/number/format() → nebula-validator
  Config может убрать прямые зависимости: chrono, uuid (остаются транзитивно)

После Phase 3:
  nebula-engine ──→ nebula-parameter (уже есть)
  Engine: collection.validate(&resolved) — просто вызов, без glue

После Phase 4:
  nebula-workflow ──→ nebula-parameter (уже есть)
  validate_workflow(): rule.validate(&literal) для статических значений

Layer compliance:
  ✅ Parameter (Domain) → Validator (Domain) — ОК (same layer)
  ✅ Config (System) → Validator (Domain) — ОК (validator = утилитарная библиотека)
  ✅ Engine (Application) → Parameter (Domain) → Validator (Domain) — ОК (вниз)
  ✅ Workflow (Domain) → Parameter (Domain) → Validator (Domain) — ОК (same layer)
```

**Ключевой принцип:** Validator — библиотека валидаторов. Parameter и Config — потребители, которые делегируют проверки в validator. Engine — orchestrator, который вызывает `collection.validate()`. Каждый делает своё дело.

---

## 7. Quality Gates (CI pipeline, каждый PR)

```bash
# Обязательные
cargo fmt --all -- --check
cargo clippy -p nebula-validator -- -D warnings
cargo check -p nebula-validator --all-features
cargo test -p nebula-validator
cargo doc --no-deps -p nebula-validator

# После Phase 1
cargo test -p nebula-parameter                    # Parameter integration
cargo clippy -p nebula-parameter -- -D warnings

# После Phase 2
cargo test -p nebula-config                       # Config integration
cargo clippy -p nebula-config -- -D warnings

# После Phase 3
cargo test -p nebula-engine                       # Engine integration
cargo bench -p nebula-validator                   # Regression check
```

---

## 8. Метрики зрелости

| Метрика | Phase 0 | Phase 1 | Phase 2 | Phase 3 | Phase 5 | Phase 8 |
|---------|---------|---------|---------|---------|---------|---------|
| LOC (validator src/) | ~20,000 | ~20,500 | ~20,500 | ~20,500 | ~21,000 | ~22,000 |
| LOC (parameter collection.rs) | ~330 | ~250 | ~250 | ~250 | ~260 | ~260 |
| LOC (config schema.rs) | ~860 | ~860 | ~730 | ~730 | ~730 | ~730 |
| Крейты-потребители | 0 | 1 (param) | 2 (param, config) | 2 (+engine) | 2 (+wf) | 2+ |
| ValidationRule variants | 9 | 9 | 9 | 9 | 15+ | 15+ |
| Тесты (validator) | ~100 | ~120 | ~130 | ~140 | ~170 | ~210 |
| Тесты (parameter validation) | ~40 | ~60 | ~60 | ~60 | ~80 | ~80 |
| Тесты (config validation) | ~30 | ~30 | ~40 | ~40 | ~40 | ~40 |
| Property тесты | 0 | 4 | 6 | 8 | 10 | 14 |
| Бенчмарки | 2 | 2 | 3 | 3 | 4 | 5 |
| Feature flags | 1 | 3 | 3 | 3 | 3 | 4 |
| Docs (matching code) | 1 | 2 | 2 | 2 | 3 | 3 |
| AsyncValidate impls | 0 | 0 | 0 | 0 | 0 | 3+ |

---

## 9. Принципы (на весь путь)

1. **Каждая фаза — shippable.** Не бывает "промежуточных" фаз, где крейт сломан.
2. **serde_json::Value — главный тип данных.** Всё в Nebula pipeline проходит через `Value`. Валидаторы работают с `Value` first, typed structs — second.
3. **Одна платформа — один code path.** Один workspace, одна команда. Логика валидации живёт в одном месте, не дублируется.
4. **ValidationRule.validate() — единственная точка входа.** Parameter определяет правила. Validator реализует логику. Engine вызывает `collection.validate()`. Никакого glue-кода.
5. **Custom правила — ответственность engine.** ValidationRule::Custom пропускается в validate(), engine вызывает ExpressionEngine.
6. **Config делегирует value-level checks.** JSON Schema traversal остаётся в config, format/constraint проверки уходят в validator.
7. **Тесты первее кода.** Property tests для Value conversions, regression suite для collection.validate().
8. **Документация = код.** Если код изменился, документация обновляется в том же PR.
9. **No premature abstraction.** AsyncValidate, Refined types, Cached — defer до реального use case.
10. **Boring is good.** `rule.validate(&value)` — просто вызов метода. Не closure, не trait object, не macro magic.
