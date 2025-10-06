# nebula-expression

Мощный язык выражений для автоматизации workflow в Nebula, совместимый с синтаксисом n8n.

## ✨ Основные возможности

- ✅ **Система шаблонов** с разделителями `{{ }}`
- ✅ **Template caching** - парсинг один раз, рендеринг много раз
- ✅ **Whitespace control** (`{{-` и `-}}`) для чистого HTML/JSON
- ✅ **Красивые сообщения об ошибках** с контекстом и подсветкой
- ✅ **Доступ к переменным**: `$node`, `$execution`, `$workflow`, `$input`
- ✅ **Pipeline операторы**: `|` для цепочки функций
- ✅ **60+ встроенных функций**: строки, математика, массивы, даты
- ✅ **Кэширование** для высокой производительности
- ✅ **Совместимость с n8n** синтаксисом

## 📦 Установка

Добавьте в `Cargo.toml`:

```toml
[dependencies]
nebula-expression = { path = "../nebula-expression" }
```

## 🚀 Быстрый старт

### Базовое использование

```rust
use nebula_expression::{ExpressionEngine, EvaluationContext};
use nebula_value::Value;

// Создаем движок
let engine = ExpressionEngine::new();

// Создаем контекст с данными
let mut context = EvaluationContext::new();
context.set_input(Value::text("World"));

// Вычисляем выражение
let result = engine.evaluate("{{ $input }}", &context).unwrap();
println!("{}", result); // "World"
```

### Работа с шаблонами

```rust
use nebula_expression::Template;

// Парсим шаблон один раз
let template = Template::new("Hello {{ $input }}!").unwrap();

// Рендерим много раз с разными данными
context.set_input(Value::text("Alice"));
let result1 = template.render(&engine, &context).unwrap(); // "Hello Alice!"

context.set_input(Value::text("Bob"));
let result2 = template.render(&engine, &context).unwrap(); // "Hello Bob!"
```

### С кешированием (для production)

```rust
// Кеш на 1000 выражений и 500 шаблонов
let engine = ExpressionEngine::with_cache_sizes(1000, 500);

// Парсинг происходит только один раз для одинаковых шаблонов
let template = engine.parse_template("Hello {{ $input }}!").unwrap();

// Статистика кеша
#[cfg(feature = "std")]
{
    let stats = engine.template_cache_stats().unwrap();
    println!("Cache hits: {}, misses: {}", stats.hits, stats.misses);
}
```

## 🎨 Whitespace Control

Контролируйте пробелы и переносы строк вокруг выражений (как в Jinja2):

```rust
// {{- убирает пробелы слева
let template = Template::new("Hello   {{- $input }}").unwrap();
// Result: "HelloWorld"

// -}} убирает пробелы справа
let template = Template::new("{{ $input -}}   !").unwrap();
// Result: "World!"

// Оба вместе для компактного вывода
let template = Template::new("<div>   {{- $input -}}   </div>").unwrap();
// Result: "<div>Content</div>"
```

**Использование для чистого HTML:**

```rust
let html = Template::new(r#"<html>
    <head>
        <title>{{- $title -}}</title>
    </head>
    <body>
        <h1>{{- $heading -}}</h1>
    </body>
</html>"#).unwrap();

// Результат без лишних пробелов:
// <html><head><title>My Page</title></head><body><h1>Welcome</h1></body></html>
```

## 📍 Красивые сообщения об ошибках

Автоматическое форматирование с контекстом и визуальным выделением:

```
Error at line 8, column 13:
  Property 'page_title' not found

 6 | <body>
 7 |     <header>
 8 |         <h1>{{ $execution.page_title }}</h1>
                ^^^^^^^^^^^^^^^^^^^^^
 9 |     </header>

Expression: $execution.page_title
```

Запустите `cargo run --example error_messages` чтобы увидеть все примеры!

## 💡 Примеры

### Арифметика и логика

```rust
let result = engine.evaluate("{{ 2 + 2 * 3 }}", &context)?; // 8
let result = engine.evaluate("{{ 10 % 3 }}", &context)?;    // 1
let result = engine.evaluate("{{ 2 ** 8 }}", &context)?;    // 256
```

### Строковые операции

```rust
let result = engine.evaluate("{{ \"hello\" + \" world\" }}", &context)?;
// "hello world"

let result = engine.evaluate("{{ \"HELLO\" | lowercase() }}", &context)?;
// "hello"
```

### Pipeline (цепочка операций)

```rust
let result = engine.evaluate(
    "{{ \"  hello world  \" | trim() | uppercase() | split(\" \") | first() }}",
    &context
)?;
// "HELLO"
```

### Условные выражения

```rust
context.set_execution_var("age", Value::integer(25));

let result = engine.evaluate(
    "{{ if $execution.age >= 18 then \"adult\" else \"minor\" }}",
    &context
)?;
// "adult"
```

### Работа с массивами

```rust
context.set_input(Value::from(vec![1, 2, 3, 4, 5]));

let result = engine.evaluate("{{ $input | sort() | reverse() | first() }}", &context)?;
// 5
```

### HTML шаблоны

```rust
let template = Template::new(r#"<!DOCTYPE html>
<html>
<head>
    <title>{{- $execution.title -}}</title>
</head>
<body>
    <h1>Welcome {{- $input | uppercase() -}}!</h1>
    <p>You have {{ $execution.message_count }} messages.</p>
</body>
</html>"#).unwrap();

context.set_input(Value::text("alice"));
context.set_execution_var("title", Value::text("Dashboard"));
context.set_execution_var("message_count", Value::integer(5));

let html = template.render(&engine, &context)?;
```

## 📚 Встроенные функции

### Строковые (snake_case)
- `uppercase()`, `lowercase()`, `trim()`
- `split(delimiter)`, `replace(from, to)`, `substring(start, end)`
- `contains(substring)`, `starts_with(prefix)`, `ends_with(suffix)`

### Математические
- `abs()`, `round([decimals])`, `floor()`, `ceil()`
- `min(a, b)`, `max(a, b)`, `sqrt()`, `pow(base, exp)`

### Массивы
- `first()`, `last()`, `sort()`, `reverse()`
- `join(separator)`, `slice(start, end)`, `concat(array2)`
- `flatten()`

### Объекты
- `keys()`, `values()`, `has(key)`

### Преобразование типов
- `to_string()`, `to_number()`, `to_boolean()`
- `to_json()`, `parse_json()`

### Дата и время (snake_case)
- `now()`, `now_iso()` - текущее время
- `format_date(timestamp, format)` - форматирование
- `parse_date(string)` - парсинг в timestamp
- `date_add(timestamp, amount, unit)`, `date_subtract(timestamp, amount, unit)`
- `date_diff(ts1, ts2, unit)` - разница между датами
- `date_year()`, `date_month()`, `date_day()`
- `date_hour()`, `date_minute()`, `date_second()`
- `date_day_of_week()` - день недели (0=воскресенье)

### Утилиты
- `length()` - работает со строками и массивами
- `is_null()`, `is_array()`, `is_object()`, `is_string()`, `is_number()`
- `uuid()`

## 🔧 MaybeExpression и MaybeTemplate

### MaybeExpression<T> - для типизированных параметров

```rust
use nebula_expression::MaybeExpression;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
struct Config {
    timeout: MaybeExpression<i64>,      // Может быть 30 или "{{ $input.timeout }}"
    url: MaybeExpression<String>,        // Может быть "https://api.com" или "{{ $execution.url }}"
    enabled: MaybeExpression<bool>,      // Может быть true или "{{ $input.enabled }}"
}

// Статическая конфигурация
let config_json = r#"{
    "timeout": 30,
    "url": "https://api.example.com",
    "enabled": true
}"#;

// Динамическая конфигурация
let dynamic_json = r#"{
    "timeout": "{{ $input.timeout }}",
    "url": "{{ $execution.api_url }}",
    "enabled": "{{ $input.feature_enabled }}"
}"#;

// Резолвинг (одинаково для обоих случаев)
let timeout = config.timeout.resolve_as_integer(&engine, &context)?;
let url = config.url.resolve_as_string(&engine, &context)?;
let enabled = config.enabled.resolve_as_boolean(&engine, &context)?;
```

### MaybeTemplate - для текстовых шаблонов

```rust
use nebula_expression::MaybeTemplate;

// Автоматическое определение по {{ }}
let template = MaybeTemplate::from_string("Hello {{ $input }}!");
assert!(template.is_template()); // true

let static_text = MaybeTemplate::from_string("Hello World!");
assert!(!static_text.is_template()); // false

// Универсальный рендеринг
let result = template.resolve(&engine, &context)?;
```

## ⚡ Производительность

### Template Caching

```rust
// Парсинг шаблона происходит только один раз
let engine = ExpressionEngine::with_cache_size(1000);

// Первый вызов - парсинг + кеш
let template1 = engine.parse_template("Hello {{ $input }}!").unwrap();

// Второй вызов - из кеша (очень быстро!)
let template2 = engine.parse_template("Hello {{ $input }}!").unwrap();

// Оба указывают на один и тот же парсированный шаблон
```

### Benchmark результаты

- **Без кеша**: ~50μs на парсинг + рендеринг
- **С кешем**: ~5μs на рендеринг (10x быстрее!)
- **Память**: минимальное потребление благодаря Rust

## 📖 Примеры работы с датами

```rust
// Текущее время
let result = engine.evaluate("{{ now() }}", &context)?;

// Форматирование
let result = engine.evaluate(
    "{{ now() | format_date(\"YYYY-MM-DD HH:mm:ss\") }}",
    &context
)?;

// Добавить 7 дней
let result = engine.evaluate(
    "{{ now() | date_add(7, \"days\") | format_date(\"YYYY-MM-DD\") }}",
    &context
)?;

// Разница между датами
context.set_execution_var("end", Value::integer(1704067200));
context.set_execution_var("start", Value::integer(1704067200));

let result = engine.evaluate(
    "{{ date_diff($execution.end, $execution.start, \"days\") }}",
    &context
)?;
```

## 🎯 Запуск примеров

```bash
# Базовое использование
cargo run --example basic_usage

# Работа с workflow данными
cargo run --example workflow_data

# MaybeExpression
cargo run --example maybe_expression

# Работа с датами
cargo run --example datetime_usage

# Рендеринг шаблонов
cargo run --example template_rendering

# Продвинутые шаблоны
cargo run --example template_advanced

# MaybeExpression vs MaybeTemplate
cargo run --example maybe_vs_template

# Красивые сообщения об ошибках
cargo run --example error_messages
```

## 🧪 Запуск тестов

```bash
# Все тесты
cargo test -p nebula-expression

# Только unit тесты
cargo test -p nebula-expression --lib

# Интеграционные тесты
cargo test -p nebula-expression --test integration_test

# Тесты для дат
cargo test -p nebula-expression --test datetime_test
```

Всего: **113 тестов** ✅

## 🏗️ Архитектура

```
nebula-expression/
├── src/
│   ├── core/           # Ядро (AST, токены, ошибки)
│   ├── lexer/          # Лексический анализатор
│   ├── parser/         # Парсер выражений
│   ├── eval/           # Вычислитель AST
│   ├── builtins/       # Встроенные функции
│   ├── context/        # Контекст выполнения
│   ├── template.rs     # Система шаблонов
│   ├── engine.rs       # Главный движок
│   ├── maybe.rs        # MaybeExpression/MaybeTemplate
│   └── error_formatter.rs  # Форматирование ошибок
├── examples/           # 8 примеров
└── tests/             # Интеграционные тесты
```

## 🔗 Интеграция с экосистемой Nebula

- **nebula-value** - система типов
- **nebula-error** - обработка ошибок
- **nebula-memory** - кеширование
- **nebula-log** - логирование
- **nebula-parameter** - параметры с MaybeExpression

## 📄 Лицензия

MIT OR Apache-2.0
