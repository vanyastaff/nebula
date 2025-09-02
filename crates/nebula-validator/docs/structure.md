# 📁 Структура файлов nebula-validator

```
nebula-validator/
├── Cargo.toml
├── README.md
├── LICENSE
├── CHANGELOG.md
├── .gitignore
├── rustfmt.toml
├── clippy.toml
│
├── src/
│   ├── lib.rs                     # Главный файл с re-exports
│   │
│   ├── core/                      # Core функциональность
│   │   ├── mod.rs
│   │   ├── validity.rs            # Valid/Invalid типы
│   │   ├── validated.rs           # Validated<T> enum и методы
│   │   ├── proof.rs              # ValidationProof система
│   │   └── error.rs              # Базовые типы ошибок
│   │
│   ├── types/                     # Основные типы
│   │   ├── mod.rs
│   │   ├── result.rs             # ValidationResult
│   │   ├── error.rs              # ValidationError, ErrorCode
│   │   ├── metadata.rs           # ValidatorMetadata, ValidationMetadata
│   │   ├── complexity.rs         # ValidationComplexity
│   │   ├── config.rs             # ValidationConfig
│   │   └── id.rs                 # ValidatorId и другие идентификаторы
│   │
│   ├── traits/                    # Основные trait'ы
│   │   ├── mod.rs
│   │   ├── validatable.rs        # Основной trait Validatable
│   │   ├── validator.rs          # Validator trait
│   │   ├── async_validator.rs    # AsyncValidator trait
│   │   ├── combinators.rs        # ValidatableExt с комбинаторами
│   │   ├── context_aware.rs      # ContextAwareValidator
│   │   └── state_aware.rs        # StateAwareValidator
│   │
│   ├── validators/                # Конкретные валидаторы
│   │   ├── mod.rs
│   │   │
│   │   ├── basic/                # Базовые валидаторы
│   │   │   ├── mod.rs
│   │   │   ├── always.rs         # AlwaysValid, AlwaysInvalid
│   │   │   ├── predicate.rs      # Predicate валидатор
│   │   │   ├── required.rs       # Required, Optional
│   │   │   ├── null.rs           # NotNull, IsNull
│   │   │   └── type_check.rs     # Проверка типов
│   │   │
│   │   ├── logical/               # Логические комбинаторы
│   │   │   ├── mod.rs
│   │   │   ├── and.rs            # AND валидатор
│   │   │   ├── or.rs             # OR валидатор  
│   │   │   ├── not.rs            # NOT валидатор
│   │   │   ├── xor.rs            # XOR валидатор
│   │   │   ├── all.rs            # All валидатор (для массивов)
│   │   │   └── any.rs            # Any валидатор (для массивов)
│   │   │
│   │   ├── conditional/           # Условные валидаторы
│   │   │   ├── mod.rs
│   │   │   ├── when.rs           # When валидатор
│   │   │   ├── when_chain.rs     # WhenChain (switch/case)
│   │   │   ├── conditions.rs     # Condition trait и реализации
│   │   │   ├── required_if.rs    # RequiredIf валидатор
│   │   │   ├── forbidden_if.rs   # ForbiddenIf валидатор
│   │   │   └── depends_on.rs     # DependsOn валидатор
│   │   │
│   │   ├── string/                # Строковые валидаторы
│   │   │   ├── mod.rs
│   │   │   ├── length.rs         # MinLength, MaxLength, Length
│   │   │   ├── pattern.rs        # Pattern (regex)
│   │   │   ├── format.rs         # Email, Url, UUID, IP
│   │   │   ├── contains.rs       # Contains, StartsWith, EndsWith
│   │   │   └── case.rs           # Uppercase, Lowercase, CamelCase
│   │   │
│   │   ├── numeric/               # Числовые валидаторы
│   │   │   ├── mod.rs
│   │   │   ├── range.rs          # Min, Max, Between
│   │   │   ├── comparison.rs     # GreaterThan, LessThan, Equal
│   │   │   ├── divisible.rs      # DivisibleBy, Even, Odd
│   │   │   ├── precision.rs      # DecimalPlaces, SignificantFigures
│   │   │   └── special.rs        # Positive, Negative, Zero, NonZero
│   │   │
│   │   ├── collection/            # Валидаторы коллекций
│   │   │   ├── mod.rs
│   │   │   ├── array.rs          # ArrayValidator
│   │   │   ├── object.rs         # ObjectValidator
│   │   │   ├── unique.rs         # Unique элементы
│   │   │   ├── sorted.rs         # Проверка сортировки
│   │   │   └── size.rs           # MinSize, MaxSize
│   │   │
│   │   ├── advanced/              # Продвинутые валидаторы
│   │   │   ├── mod.rs
│   │   │   ├── lazy.rs           # Lazy валидатор
│   │   │   ├── deferred.rs       # Deferred валидатор
│   │   │   ├── memoized.rs       # Memoized валидатор
│   │   │   ├── throttled.rs      # Throttled валидатор
│   │   │   ├── retry.rs          # Retry валидатор
│   │   │   └── timeout.rs        # Timeout валидатор
│   │   │
│   │   └── custom/                # Кастомные валидаторы
│   │       ├── mod.rs
│   │       ├── function.rs       # FunctionValidator
│   │       ├── closure.rs        # Closure-based валидатор
│   │       └── external.rs       # External service валидатор
│   │
│   ├── transform/                  # Система трансформаций
│   │   ├── mod.rs
│   │   ├── traits.rs             # Transformer trait
│   │   ├── chain.rs              # TransformChain
│   │   ├── implementations/      # Конкретные трансформеры
│   │   │   ├── mod.rs
│   │   │   ├── string.rs         # StringNormalizer, Trim, etc
│   │   │   ├── numeric.rs        # NumberRounder, Clamp, etc
│   │   │   ├── data.rs           # DataMasker, Sanitizer
│   │   │   ├── format.rs         # FormatConverter
│   │   │   └── codec.rs          # Encoder, Decoder
│   │   └── validator.rs          # TransformingValidator
│   │
│   ├── pipeline/                   # Pipeline система
│   │   ├── mod.rs
│   │   ├── builder.rs            # PipelineBuilder
│   │   ├── stage.rs              # PipelineStage
│   │   ├── executor.rs           # Выполнение pipeline
│   │   ├── result.rs             # PipelineResult
│   │   └── metrics.rs            # PipelineMetrics
│   │
│   ├── rules/                      # Rule engine
│   │   ├── mod.rs
│   │   ├── engine.rs             # RuleEngine
│   │   ├── rule.rs               # Rule trait
│   │   ├── constraint.rs         # Constraint implementation
│   │   ├── context.rs            # RuleContext
│   │   ├── executor.rs           # RuleExecutor
│   │   └── result.rs             # RuleResult
│   │
│   ├── context/                    # Контекст валидации
│   │   ├── mod.rs
│   │   ├── validation_context.rs # ValidationContext
│   │   ├── state.rs              # ValidationState
│   │   ├── strategy.rs           # ValidationStrategy
│   │   └── mode.rs               # ValidationMode
│   │
│   ├── cache/                      # Кэширование
│   │   ├── mod.rs
│   │   ├── memory.rs             # In-memory cache
│   │   ├── lru.rs               # LRU cache
│   │   ├── ttl.rs               # TTL-based cache
│   │   ├── builder.rs           # CacheBuilder
│   │   └── stats.rs             # CacheStats
│   │
│   ├── metrics/                    # Метрики и мониторинг
│   │   ├── mod.rs
│   │   ├── registry.rs          # MetricsRegistry
│   │   ├── collector.rs         # MetricsCollector
│   │   ├── histogram.rs         # Histogram implementation
│   │   ├── counter.rs           # Counter implementation
│   │   └── gauge.rs             # Gauge implementation
│   │
│   ├── registry/                   # Реестр валидаторов
│   │   ├── mod.rs
│   │   ├── validator_registry.rs # ValidatorRegistry
│   │   ├── builder.rs           # RegistryBuilder
│   │   ├── discovery.rs         # Автоматическое обнаружение
│   │   └── stats.rs             # RegistryStats
│   │
│   ├── builder/                    # Builder API
│   │   ├── mod.rs
│   │   ├── validator_builder.rs # Основной builder
│   │   ├── string_builder.rs    # Builder для строк
│   │   ├── numeric_builder.rs   # Builder для чисел
│   │   ├── object_builder.rs    # Builder для объектов
│   │   └── array_builder.rs     # Builder для массивов
│   │
│   ├── prelude/                    # Удобный re-export
│   │   └── mod.rs               # pub use всего необходимого
│   │
│   └── utils/                      # Утилиты
│       ├── mod.rs
│       ├── hash.rs              # Хэширование для кэша
│       ├── json.rs              # JSON утилиты
│       └── async_utils.rs       # Async helpers
│
├── tests/                          # Интеграционные тесты
│   ├── common/
│   │   └── mod.rs               # Общие helper'ы для тестов
│   ├── basic_validators.rs
│   ├── logical_validators.rs
│   ├── conditional_validators.rs
│   ├── string_validators.rs
│   ├── numeric_validators.rs
│   ├── collection_validators.rs
│   ├── pipeline.rs
│   ├── rules.rs
│   ├── transform.rs
│   └── cache.rs
│
├── benches/                        # Бенчмарки
│   ├── validators.rs
│   ├── pipeline.rs
│   └── cache.rs
│
├── examples/                       # Примеры использования
│   ├── basic.rs                 # Базовый пример
│   ├── form_validation.rs       # Валидация формы
│   ├── api_validation.rs        # Валидация API
│   ├── pipeline.rs              # Использование pipeline
│   ├── rules.rs                 # Rule engine
│   ├── custom_validator.rs      # Создание кастомного валидатора
│   └── advanced.rs              # Продвинутые сценарии
│
└── docs/                           # Документация
    ├── architecture.md           # Архитектура
    ├── getting_started.md        # Быстрый старт
    ├── validators.md             # Список валидаторов
    ├── combinators.md            # Комбинаторы
    ├── pipeline.md               # Pipeline система
    ├── rules.md                  # Rule engine
    ├── caching.md                # Кэширование
    └── performance.md            # Оптимизация производительности
```

## 📝 Содержимое ключевых файлов

### `src/lib.rs`
```rust
//! Nebula Validator - Production-ready validation framework

#![warn(missing_docs)]
#![deny(unsafe_code)]

// Core modules
pub mod core;
pub mod types;
pub mod traits;
pub mod validators;
pub mod transform;
pub mod pipeline;
pub mod rules;
pub mod context;
pub mod cache;
pub mod metrics;
pub mod registry;
pub mod builder;
pub mod utils;

// Prelude for convenient imports
pub mod prelude;

// Re-export core types
pub use core::{Valid, Invalid, Validated, ValidationProof};
pub use types::{
    ValidationResult, ValidationError, ErrorCode,
    ValidatorMetadata, ValidationComplexity,
};
pub use traits::{Validatable, Validator, AsyncValidator};

// Re-export common validators
pub use validators::{
    basic::{AlwaysValid, AlwaysInvalid, Predicate},
    logical::{And, Or, Not, Xor},
    conditional::{When, WhenChain, RequiredIf},
};

// Version info
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
```

### `src/prelude/mod.rs`
```rust
//! Convenient imports for nebula-validator

pub use crate::core::{Valid, Invalid, Validated, ValidationProof};
pub use crate::types::*;
pub use crate::traits::*;

// Common validators
pub use crate::validators::{
    basic::*,
    logical::*,
    conditional::*,
    string::*,
    numeric::*,
};

// Builders
pub use crate::builder::{ValidatorBuilder, StringValidatorBuilder};

// Pipeline
pub use crate::pipeline::{ValidationPipeline, PipelineBuilder};

// Rules
pub use crate::rules::{RuleEngine, Constraint};

// Transform
pub use crate::transform::{Transformer, TransformChain};

// Context
pub use crate::context::ValidationContext;

// Common imports
pub use serde_json::{json, Value};
pub use async_trait::async_trait;
```

### `src/validators/mod.rs`
```rust
//! Validator implementations

pub mod basic;
pub mod logical;
pub mod conditional;
pub mod string;
pub mod numeric;
pub mod collection;
pub mod advanced;
pub mod custom;

// Re-export all validators
pub use basic::*;
pub use logical::*;
pub use conditional::*;
pub use string::*;
pub use numeric::*;
pub use collection::*;
pub use advanced::*;
pub use custom::*;
```

### `Cargo.toml`
```toml
[package]
name = "nebula-validator"
version = "0.1.0"
edition = "2021"
authors = ["Your Name <email@example.com>"]
description = "Production-ready validation framework for Nebula"
repository = "https://github.com/yourusername/nebula"
license = "MIT OR Apache-2.0"
keywords = ["validation", "validator", "nebula", "async"]
categories = ["data-structures", "asynchronous"]

[dependencies]
# Core
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
async-trait = "0.1"
thiserror = "2.0"

# Async runtime
tokio = { version = "1.43", features = ["full"] }
futures = "0.3"

# Data structures
dashmap = "6.1"
indexmap = "2.7"

# Time
chrono = { version = "0.4", features = ["serde"] }

# Validation
regex = "1.11"
once_cell = "1.20"

# Utilities
tracing = "0.1"
anyhow = "1.0"
base64 = "0.22"
uuid = { version = "1.11", features = ["v4", "serde"] }

# Optional dependencies
redis = { version = "0.27", optional = true }
sqlx = { version = "0.8", optional = true }

[dev-dependencies]
tokio-test = "0.4"
proptest = "1.6"
criterion = "0.6"
pretty_assertions = "1.4"

[features]
default = ["full"]
full = ["redis-cache", "database", "metrics"]
redis-cache = ["redis"]
database = ["sqlx"]
metrics = []

[[bench]]
name = "validators"
harness = false

[[example]]
name = "basic"

[[example]]
name = "form_validation"
```

## 🎯 Организационные принципы

1. **Модульность** - Каждый модуль отвечает за одну область
2. **Переиспользование** - Общий код в `utils` и `common`
3. **Тестируемость** - Каждый модуль имеет unit тесты, плюс интеграционные
4. **Документация** - Каждый публичный API задокументирован
5. **Примеры** - Реальные use cases в `examples/`

## 📦 Дополнительные файлы

### `.gitignore`
```gitignore
/target
/Cargo.lock
*.swp
*.swo
.DS_Store
.idea/
.vscode/
*.iml
```

### `rustfmt.toml`
```toml
edition = "2021"
max_width = 100
use_small_heuristics = "Max"
imports_granularity = "Crate"
group_imports = "StdExternalCrate"
```

### `clippy.toml`
```toml
warn-on-all-wildcard-imports = true
allow-expect-in-tests = true
allow-unwrap-in-tests = true
allow-dbg-in-tests = true
```

Эта структура обеспечивает:
- **Четкую организацию** - легко найти нужный код
- **Масштабируемость** - легко добавлять новые валидаторы
- **Поддерживаемость** - модульная архитектура
- **Тестируемость** - отдельные тесты для каждого компонента
- **Документированность** - примеры и документация