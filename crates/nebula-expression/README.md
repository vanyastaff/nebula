# nebula-expression

Мощный язык выражений для автоматизации workflow в Nebula, совместимый с синтаксисом n8n.

## Возможности

- ✅ **Шаблонный синтаксис** с разделителями `{{ }}`
- ✅ **Доступ к переменным**: `$node`, `$execution`, `$workflow`, `$input`
- ✅ **Pipeline операторы**: `|` для цепочки операций
- ✅ **Встроенные функции**: строковые, математические, работа с массивами
- ✅ **Безопасное выполнение** выражений с использованием nebula-error
- ✅ **Кэширование** для повышения производительности (nebula-memory)
- ✅ **Совместимость с n8n** синтаксисом

## Установка

Добавьте в `Cargo.toml`:

```toml
[dependencies]
nebula-expression = { path = "../nebula-expression" }
```

## Быстрый старт

```rust
use nebula_expression::{ExpressionEngine, EvaluationContext};
use nebula_value::Value;

// Создаем движок выражений
let engine = ExpressionEngine::new();

// Создаем контекст с данными
let mut context = EvaluationContext::new();
context.set_input(Value::Object(
    nebula_value::Object::new()
        .insert("name".to_string(), serde_json::json!("John"))
        .insert("age".to_string(), serde_json::json!(30))
));

// Вычисляем выражение
let result = engine.evaluate("{{ $input.name }}", &context).unwrap();
println!("{}", result); // "John"
```

## Примеры

### Арифметика

```rust
let result = engine.evaluate("{{ 2 + 2 }}", &context).unwrap();
// 4
```

### Строковые операции

```rust
let result = engine.evaluate("{{ \"hello\" + \" world\" }}", &context).unwrap();
// "hello world"
```

### Pipeline операции

```rust
let result = engine.evaluate("{{ \"HELLO\" | lowercase() }}", &context).unwrap();
// "hello"

let result = engine.evaluate("{{ 3.14159 | round(2) }}", &context).unwrap();
// 3.14
```

### Условные выражения

```rust
let result = engine.evaluate(
    "{{ if $input.age >= 18 then \"adult\" else \"minor\" }}",
    &context
).unwrap();
```

### Работа с данными workflow

```rust
context.set_node_data("http", Value::Object(
    nebula_value::Object::new().insert(
        "response".to_string(),
        serde_json::json!({"statusCode": 200})
    )
));

let result = engine.evaluate(
    "{{ $node.http.response.statusCode }}",
    &context
).unwrap();
// 200
```

## Встроенные функции

### Строковые
- `uppercase()`, `lowercase()`, `trim()`
- `split(delimiter)`, `replace(from, to)`, `substring(start, end)`
- `contains(substring)`, `startsWith(prefix)`, `endsWith(suffix)`

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
- `toString()`, `toNumber()`, `toBoolean()`
- `toJson()`, `parseJson()`

### Дата и время
- `now()`, `nowIso()` - текущее время
- `formatDate(date, format)` - форматирование даты
- `parseDate(string)` - парсинг строки в timestamp
- `dateAdd(date, amount, unit)`, `dateSubtract(date, amount, unit)` - арифметика с датами
- `dateDiff(date1, date2, unit)` - разница между датами
- `dateYear()`, `dateMonth()`, `dateDay()`, `dateHour()`, `dateMinute()`, `dateSecond()` - извлечение компонентов
- `dateDayOfWeek()` - день недели (0=воскресенье)

### Утилиты
- `length()` - работает со строками и массивами
- `isNull()`, `isArray()`, `isObject()`, `isString()`, `isNumber()`
- `uuid()`

## MaybeExpression

`MaybeExpression<T>` - это специальный тип для параметров, которые могут быть как конкретными значениями, так и выражениями:

```rust
use nebula_expression::MaybeExpression;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
struct Config {
    // Может быть статическим значением или выражением
    timeout: MaybeExpression<i64>,
    message: MaybeExpression<String>,
}

// JSON с конкретными значениями
let config_json = r#"{
    "timeout": 30,
    "message": "Hello"
}"#;

// JSON с выражениями (автоматически определяется по {{ }})
let dynamic_json = r#"{
    "timeout": "{{ $input.timeout }}",
    "message": "{{ \"Hello, \" + $input.name }}"
}"#;

// Резолвинг значений
let engine = ExpressionEngine::new();
let context = EvaluationContext::new();

let timeout = config.timeout.resolve_as_integer(&engine, &context)?;
let message = config.message.resolve_as_string(&engine, &context)?;
```

## Производительность

- Использует `nebula-memory` для кэширования разобранных выражений
- Высокая производительность благодаря нативному Rust коду
- Минимальное потребление памяти

## Примеры работы с датами

```rust
// Текущее время
let result = engine.evaluate("{{ now() }}", &context)?;

// Форматирование
let result = engine.evaluate(
    "{{ now() | formatDate(\"YYYY-MM-DD HH:mm:ss\") }}",
    &context
)?;

// Добавить 7 дней
let result = engine.evaluate(
    "{{ now() | dateAdd(7, \"days\") | formatDate(\"YYYY-MM-DD\") }}",
    &context
)?;

// Разница между датами
let result = engine.evaluate(
    "{{ dateDiff($input.end, $input.start, \"days\") }}",
    &context
)?;
```

## Запуск примеров

```bash
cargo run --example basic_usage
cargo run --example workflow_data
cargo run --example maybe_expression
cargo run --example datetime_usage
```

## Запуск тестов

```bash
cargo test -p nebula-expression
```

## Лицензия

MIT OR Apache-2.0
