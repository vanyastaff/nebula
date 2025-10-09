# nebula-error - Следующие шаги

## 🎯 Приоритизированный план работ

### Sprint 3: Завершение качества кода (2-4 часа)

#### 1. Автоматизировать #[must_use] (30 мин)

**Цель:** Добавить #[must_use] ко всем оставшимся 34 методам

**Скрипт:**
```bash
cd crates/nebula-error

# Найти все pub fn возвращающие Self без #[must_use]
rg -n "pub fn \w+.*-> Self" src --type rust | \
  grep -v "#\[must_use\]" > missing_must_use.txt

# Для каждого добавить аннотацию (ручная проверка рекомендуется)
```

**Файлы для проверки:**
- `src/kinds/client.rs`
- `src/kinds/server.rs`
- `src/kinds/system.rs`
- `src/kinds/workflow.rs`

#### 2. Добавить backticks в документацию (45 мин)

**Цель:** Исправить 19 warnings о missing backticks

**Паттерны для поиска:**
```bash
# Найти все слова в CamelCase без backticks в doc comments
rg '//!.*[A-Z][a-zA-Z]+' src --type rust | grep -v '`'

# Найти ErrorKind, NebulaError и т.д. без backticks
rg '//!.*(ErrorKind|NebulaError|RetryStrategy)' src --type rust | grep -v '`'
```

**Примеры исправлений:**
```rust
// БЫЛО:
/// Converts this error to NebulaError

// СТАЛО:
/// Converts this error to `NebulaError`
```

#### 3. Исправить Clone on Copy (15 мин)

**Найти:**
```bash
cd crates/nebula-error
cargo clippy 2>&1 | grep "Clone on Copy"
```

**Исправить:**
```rust
// БЫЛО:
let strategy2 = strategy1.clone();

// СТАЛО:
let strategy2 = strategy1;  // RetryStrategy implements Copy
```

#### 4. Добавить # Errors секции (30 мин)

**Цель:** Добавить документацию об ошибках для 8 функций

**Шаблон:**
```rust
/// Retry operation with given strategy
///
/// # Errors
///
/// Returns `NebulaError` if:
/// - All retry attempts are exhausted
/// - Operation timeout is exceeded
/// - Non-retryable error occurs
pub async fn retry<F>(...) -> Result<T, NebulaError> {
    // ...
}
```

**Файлы:**
- `src/core/retry.rs` - функции retry, retry_with_timeout
- `src/core/conversion.rs` - helper функции

---

### Sprint 4: Архитектурные улучшения (4-6 часов)

#### 1. Feature flags для модульности

**Cargo.toml:**
```toml
[features]
default = ["retry", "context", "serde"]
minimal = []
retry = ["tokio", "rand", "async-trait"]
context = ["chrono"]
serde = ["dep:serde", "dep:serde_json"]
full = ["retry", "context", "serde"]
```

**Условная компиляция:**
```rust
#[cfg(feature = "context")]
pub mod context;

#[cfg(feature = "retry")]
pub mod retry;
```

**Преимущества:**
- Меньший размер бинарников для embedded
- Гибкость для пользователей
- Faster compilation

#### 2. Анализ и оптимизация зависимостей

**Проверить:**
```bash
cd crates/nebula-error
cargo tree --duplicates
cargo tree | grep anyhow  # Используется ли?
```

**Возможные действия:**
- ❓ Удалить `anyhow` если не используется
- ✅ Обновить все зависимости до latest
- ✅ Минимизировать features в зависимостях

**Пример оптимизации:**
```toml
# БЫЛО:
tokio = { version = "1.47", features = ["full"] }

# СТАЛО:
tokio = { version = "1.47", features = ["time", "sync", "macros"], optional = true }
```

#### 3. Benchmarks для критических путей

**Создать:** `benches/error_creation.rs`

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nebula_error::NebulaError;

fn bench_error_creation(c: &mut Criterion) {
    c.bench_function("error_validation", |b| {
        b.iter(|| {
            NebulaError::validation(black_box("test error"))
        });
    });

    c.bench_function("error_with_context", |b| {
        b.iter(|| {
            let err = NebulaError::validation("test");
            err.with_context(ErrorContext::new(black_box("operation")))
        });
    });
}

criterion_group!(benches, bench_error_creation);
criterion_main!(benches);
```

**Cargo.toml:**
```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "error_creation"
harness = false
```

**Запуск:**
```bash
cargo bench
# Результаты в target/criterion/report/index.html
```

---

### Sprint 5: CI/CD и автоматизация (2-3 часа)

#### 1. GitHub Actions workflow

**`.github/workflows/ci.yml`:**
```yaml
name: CI

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@stable

      - name: Format check
        run: cargo fmt --check

      - name: Clippy (strict)
        run: |
          cargo clippy --all-features -- \
            -D warnings \
            -W clippy::pedantic \
            -A clippy::module-name-repetitions

      - name: Test
        run: cargo test --all-features

      - name: Doc
        run: cargo doc --no-deps --all-features

  bench:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@stable

      - name: Benchmark
        run: cargo bench --no-run
```

#### 2. Pre-commit hooks

**`.git/hooks/pre-commit`:**
```bash
#!/bin/bash
set -e

echo "🔍 Running pre-commit checks..."

# Format check
cargo fmt --check || {
    echo "❌ Code not formatted. Run: cargo fmt"
    exit 1
}

# Clippy check
cargo clippy --all-features -- -D warnings || {
    echo "❌ Clippy found issues"
    exit 1
}

# Tests
cargo test --all-features --quiet || {
    echo "❌ Tests failed"
    exit 1
}

echo "✅ All checks passed!"
```

#### 3. Cargo.toml lints configuration

```toml
[lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"
unused_must_use = "deny"

[lints.clippy]
all = "warn"
pedantic = "warn"
nursery = "warn"
# Разрешённые исключения
module_name_repetitions = "allow"
similar_names = "allow"
```

---

## 📊 Ожидаемые результаты

### После Sprint 3 (Качество)
- ✅ 0 clippy warnings (--pedantic)
- ✅ 100% #[must_use] coverage
- ✅ Полная документация

### После Sprint 4 (Архитектура)
- ✅ Модульные feature flags
- ✅ Оптимизированные зависимости
- ✅ Benchmarks для мониторинга производительности

### После Sprint 5 (Автоматизация)
- ✅ CI проверяет качество на каждом PR
- ✅ Pre-commit hooks предотвращают плохой код
- ✅ Автоматическая публикация на crates.io

---

## 🎯 Метрики успеха

| Метрика | Текущее | Цель | Sprint |
|:--------|:--------|:-----|:-------|
| Clippy warnings | 114 | 0 | 3 |
| #[must_use] coverage | 40% | 100% | 3 |
| Документация completeness | 70% | 95% | 3 |
| Binary size (minimal) | N/A | <50KB | 4 |
| Feature flexibility | Нет | 4 варианта | 4 |
| Benchmark coverage | 0% | 80% | 4 |
| CI automation | Нет | Полная | 5 |

---

## 💡 Дополнительные идеи

### Улучшения API

1. **Error builder pattern:**
```rust
NebulaError::builder()
    .validation("Invalid email")
    .context("User registration")
    .with_field("email", email)
    .retryable(false)
    .build()
```

2. **Typed metadata:**
```rust
impl ErrorContext {
    pub fn with_typed_metadata<T: Serialize>(
        mut self,
        key: &str,
        value: T
    ) -> Self {
        let json = serde_json::to_string(&value).unwrap();
        self.with_metadata(key, json)
    }
}
```

3. **Error chains:**
```rust
impl NebulaError {
    pub fn chain(self, cause: impl Into<NebulaError>) -> Self {
        // Сохранить цепочку ошибок
    }
}
```

### Интеграции

1. **Tracing integration:**
```rust
#[cfg(feature = "tracing")]
impl NebulaError {
    pub fn trace(&self) {
        tracing::error!(
            error_code = %self.code,
            retryable = self.retryable,
            "{}", self.message
        );
    }
}
```

2. **Metrics integration:**
```rust
#[cfg(feature = "metrics")]
impl NebulaError {
    pub fn record_metric(&self) {
        metrics::counter!(
            "errors_total",
            "code" => self.code.clone(),
            "category" => self.kind.error_category()
        ).increment(1);
    }
}
```

---

**Следующий шаг:** Sprint 3 - Качество кода (2-4 часа)
**Приоритет:** Высокий
**Статус:** Готов к началу ✅
