# Полная документация bon-rs: Compile-Time проверяемые билдеры для Rust

**bon** — это Rust-крейт для генерации билдеров с compile-time проверками для структур и функций, используя паттерн typestate. В отличие от альтернатив с runtime-валидацией, bon ловит отсутствующие обязательные поля и дублирующиеся вызовы сеттеров на этапе компиляции — никаких паник, никаких `unwrap()`, только ошибки типов, которые направляют разработчика к корректному коду.

**Используется в production:** `crates.io` backend, `tantivy`, `apache-avro`, `google-cloud-auth`, `comrak`, `ractor`.

## Установка

```toml
[dependencies]
bon = "3.8"
```

Поддержка `no_std`: используйте `default-features = false`.

---

## Основы работы с билдерами

### Function Builders — билдеры для функций

Атрибут `#[builder]` превращает функции с позиционными параметрами в функции с именованными параметрами через билдер-интерфейс.

```rust
use bon::builder;

#[builder]
fn greet(name: &str, level: Option<u32>) -> String {
    let level = level.unwrap_or(0);
    format!("Hello {name}! Your level is {level}")
}

let greeting = greet()
    .name("Bon")
    .level(24)      // опционально, можем пропустить
    .call();        // финальная функция

assert_eq!(greeting, "Hello Bon! Your level is 24");
```

**Поддерживается любой синтаксис функций:**
- `async fn` — автоматически возвращает `Future`
- Возврат `Result` — билдер становится fallible
- Generic функции с параметрами типов
- `impl Trait` в параметрах и возвращаемом типе
- `unsafe fn`

### Struct Builders — билдеры для структур

`#[derive(Builder)]` генерирует эквивалентную функциональность для структур:

```rust
use bon::Builder;

#[derive(Builder)]
struct User {
    name: String,
    is_admin: bool,
    level: Option<u32>,  // автоматически опциональное поле
}

let user = User::builder()
    .name("Bon".to_owned())
    .level(24)
    .is_admin(true)      // сеттеры можно вызывать в любом порядке
    .build();
```

### Method Builders — билдеры для методов

Ассоциированные методы требуют атрибута `#[bon]` на `impl` блоке.

**Методы с именем `new`** генерируют `builder()`/`build()`:

```rust
use bon::bon;

struct User {
    id: u32,
    name: String,
}

#[bon]
impl User {
    #[builder]
    fn new(id: u32, name: String) -> Self {
        Self { id, name }
    }
}

let user = User::builder()
    .id(1)
    .name("Bon".to_owned())
    .build();
```

**Другие методы** генерируют `{method_name}()`/`call()`:

```rust
#[bon]
impl Greeter {
    #[builder]
    fn greet(&self, target: &str, prefix: Option<&str>) -> String {
        let prefix = prefix.unwrap_or("INFO");
        format!("[{prefix}] {} says hello to {target}", self.name)
    }
}

let greeting = greeter
    .greet()
    .target("the world")
    .call();
```

**Поддерживаются методы с и без `self`.**

---

## Опциональные члены (Optional Members)

### Option<T> — автоматическая опциональность

Поля типа `Option<T>` **автоматически** становятся опциональными — билдер не требует их установки, используя `None` по умолчанию.

```rust
#[derive(Builder)]
struct Example {
    level: Option<u32>
}

// Можно вызывать без указания `level`
Example::builder().build();
```

**Отключить автоматику:** используйте `#[builder(required)]`.

### Пара сеттеров для опциональных полей

Для каждого опционального члена bon генерирует **два сеттера**:

| Имя | Вход | Описание |
|-----|------|----------|
| `{member}` | `T` | Принимает non-None значение |
| `maybe_{member}` | `Option<T>` | Принимает `Option` напрямую |

```rust
impl<S> ExampleBuilder<S> {
    fn level(self, value: u32) -> ExampleBuilder<SetLevel<S>> {
        self.maybe_level(Some(value))  // Да, вот так просто!
    }

    fn maybe_level(self, value: Option<u32>) -> ExampleBuilder<SetLevel<S>> {
        /* ... */
    }
}
```

**Примеры использования:**

```rust
// Передаем non-None через обычный сеттер
Example::builder().level(42).build();

// Передаем Option напрямую через maybe_ сеттер
let value = if condition { Some(42) } else { None };
Example::builder().maybe_level(value).build();
```

### #[builder(default)] — дефолтные значения

Для non-Option типов используйте `#[builder(default)]`:

```rust
#[derive(Builder)]
struct Example {
    // Использует Default trait
    #[builder(default)]
    a: u32,

    // Пользовательское дефолтное значение
    #[builder(default = 4)]
    b: u32,
}

let result = Example::builder().build();
assert_eq!(result.a, 0);  // Default::default()
assert_eq!(result.b, 4);  // Указанное значение
```

**Переключение между `Option<T>` и `#[builder(default)]` — полностью совместимо!**

### Вычисляемые дефолты (Computed Defaults)

Можно ссылаться на ранее объявленные члены в дефолтных выражениях:

```rust
#[derive(Builder)]
struct Computed {
    x1: u32,
    
    #[builder(default = 2 * x1)]    // ссылается на x1
    x2: u32,
    
    #[builder(default = x2 + x1)]   // ссылается на оба
    x3: u32,
}

let result = Computed::builder().x1(3).build();
assert_eq!((result.x1, result.x2, result.x3), (3, 6, 9));
```

**Важно:** Члены инициализируются в порядке объявления. Доступны только члены, объявленные выше.

---

## Into Conversions — устранение boilerplate

### Проблема

Без конвертаций передача string literals в `String` поля требует `.to_owned()` или `.into()`:

```rust
struct User { name: String }

impl User {
    fn new(name: String) -> Self {
        Self { name }
    }
}

let user = User::new("Bon".to_owned());  // Boilerplate!
```

### Решение: #[builder(into)]

Атрибут `#[builder(into)]` генерирует сеттеры, принимающие `impl Into<T>`:

```rust
#[derive(Builder)]
struct Example {
    #[builder(into)]
    name: String,
    
    #[builder(into)]
    description: Option<String>,
}

Example::builder()
    .name("Bon")                    // &str → String автоматически
    .description("Awesome crate")   // &str → String
    .build();
```

### Применение к множеству типов: on(..., into)

```rust
use std::path::PathBuf;

#[derive(Builder)]
#[builder(on(String, into))]  // Все String поля получают Into
struct Project {
    name: String,
    description: String,
    
    #[builder(into)]           // Индивидуальное переопределение
    path: PathBuf,
}

Project::builder()
    .name("Bon")
    .description("Awesome")
    .path("/path/to/bon")      // &str → PathBuf
    .build();
```

**Множественные паттерны:**
```rust
#[builder(on(String, into), on(Box<_>, into))]
```

**Отключить для конкретного поля:**
```rust
#[builder(into = false)]
```

### Какие типы НЕ получают автоматический Into

**Примитивные типы исключены по умолчанию**, потому что `impl Into` ломает type inference для числовых литералов:

```rust
fn half(value: impl Into<u32>) { /* */ }
half(10);  // ERROR: не может определить тип для литерала
```

**Также исключены:**
- Типы с явным `impl Trait` в сигнатуре
- Generic параметры из сигнатуры функции
- Tuple, array, reference, function pointer типы

---

## Custom Conversions — кастомная логика с #[builder(with)]

Для конвертаций, выходящих за рамки `Into`, атрибут `with` принимает closure, определяющее кастомную логику сеттера.

### Базовый пример

```rust
struct Point { x: u32, y: u32 }

#[derive(Builder)]
struct Example {
    #[builder(with = |x: u32, y: u32| Point { x, y })]
    point: Point,
}

Example::builder()
    .point(2, 3)    // два аргумента вместо Point значения
    .build();
```

### Fallible setters — сеттеры с Result

Возврат `Result` из closure создает **fallible сеттер**:

```rust
#[derive(Builder)]
struct Parsed {
    #[builder(with = |s: &str| -> Result<_, std::num::ParseIntError> { 
        s.parse() 
    })]
    value: u32,
}

Parsed::builder()
    .value("42")?   // сеттер возвращает Result
    .build();
```

### Shortcut для коллекций

```rust
#[builder(with = FromIterator::from_iter)]
```

Делает сеттер коллекции принимающим `impl IntoIterator`, скрывая конкретный тип коллекции.

### Преимущества перед Typestate API

Для простых case используйте `#[builder(with)]`. Для сложной логики или кастомных финальных функций используйте прямые impl блоки с Typestate API (см. ниже).

---

## Positional Members — позиционные параметры

Иногда не нужна вся гибкость именованных параметров. Можно сделать некоторые члены **позиционными параметрами** в стартовой или финальной функции.

### Позиционные параметры в стартовой функции

```rust
#[derive(Builder)]
#[builder(start_fn = with_coordinates)]  // Переименовываем
struct Treasure {
    #[builder(start_fn)]  // Делаем позиционным
    x: u32,
    #[builder(start_fn)]
    y: u32,
    label: Option<String>,
}

let treasure = Treasure::with_coordinates(2, 9)
    .label("oats".to_owned())
    .build();
```

Генерируется сигнатура:
```rust
fn with_coordinates(x: u32, y: u32) -> TreasureBuilder { /* */ }
```

### Позиционные параметры в финальной функции

```rust
#[derive(Builder)]
#[builder(start_fn = with_coordinates)]
#[builder(finish_fn = claim)]  // Переименовываем финальную функцию
struct Treasure {
    #[builder(start_fn)]
    x: u32,
    #[builder(start_fn)]
    y: u32,
    
    #[builder(finish_fn)]  // Позиционные в конце
    claimed_by_first_name: String,
    #[builder(finish_fn)]
    claimed_by_last_name: String,
    
    label: Option<String>,
}

let treasure = Treasure::with_coordinates(2, 9)
    .label("oats".to_owned())
    .claim("Lyra".to_owned(), "Heartstrings".to_owned());
```

**Важно:** Порядок членов с `#[builder(start_fn)]` и `#[builder(finish_fn)]` имеет значение — они появляются в той же последовательности в сигнатуре функции.

**Не рекомендуется** делать опциональные члены позиционными, т.к. их нельзя будет пропустить.

---

## Паттерн Typestate — compile-time безопасность

### Как работает typestate

Билдеры bon используют **паттерн typestate** — тип билдера меняется с каждым вызовом сеттера, кодируя какие поля уже установлены.

```rust
#[derive(Builder)]
struct Example { x1: u32, x2: u32 }

use example_builder::{SetX1, SetX2};

let b: ExampleBuilder             = Example::builder();
let b: ExampleBuilder<SetX1>      = b.x1(1);
let b: ExampleBuilder<SetX2<SetX1>> = b.x2(2);  // Вложенный паттерн
```

Каждый `Set{Member}<S>` тип оборачивает предыдущее состояние. **Порядок зависит от порядка вызова сеттеров:**
- `x1(1).x2(2)` → `SetX2<SetX1>`
- `x2(2).x1(1)` → `SetX1<SetX2>`

### Generic параметр S (State)

Билдер всегда содержит generic параметр `S` (означает "state") в **конце** списка параметров. Этот параметр хранит typestate.

```rust
pub struct ExampleBuilder<S: State> { /* */ }
```

**Default значение:** `S = Empty` для начального состояния.

### Модуль typestate

Типы состояний находятся в отдельном модуле:

```rust
// Модуль по умолчанию private
mod example_builder {
    pub struct SetX1<S = Empty> { /**/ }
    pub struct SetX2<S = Empty> { /**/ }
    pub struct Empty { /**/ }
}
```

**Паттерн "sealed":** Публичные символы внутри приватного модуля. Билдер доступен, но его typestate невидим извне.

**Сделать публичным:**
```rust
#[builder(state_mod(vis = "pub"))]
```

### Generics из функции/метода

Если функция имеет lifetime/generic параметры, они добавляются в начало списка параметров билдера, **перед** typestate:

```rust
#[builder]
fn method(x1: &impl Clone) { }

// lifetime param┐  type param┐  typestate (всегда последний)
let b: MethodBuilder<'_, bool, _> = method().x1(&true);
```

Порядок:
1. Named lifetimes (в порядке объявления)
2. Anonymous lifetimes из `&...`
3. Named generic types
4. Anonymous types из `impl Trait`
5. **S: State (всегда последний)**

---

## Расширение билдеров — Custom Methods

Typestate API позволяет добавлять кастомные методы к билдерам. Генерируемые traits контролируют доступность методов.

### Основные traits

**`State`:** Bound для параметра `S`; содержит associated types вроде `S::X1`

**`IsUnset`:** Член еще не установлен (предотвращает двойную установку)

**`IsSet`:** Член уже установлен

**`IsComplete`:** Все обязательные члены установлены (для финальных функций)

### Пример кастомного метода

```rust
#[derive(Builder)]
struct Example { x1: u32 }

use example_builder::{IsUnset, State, SetX1};

impl<S: State> ExampleBuilder<S> {
    fn x1_doubled(self, value: u32) -> ExampleBuilder<SetX1<S>>
    where
        S::X1: IsUnset,  // Можно вызвать, только если x1 не установлен
    {
        self.x1(value * 2)
    }
}

let result = Example::builder().x1_doubled(3).build();
assert_eq!(result.x1, 6);
```

### Кастомные методы могут быть

- **Fallible** (возвращать `Result`)
- **Async** (возвращать `Future`)
- **Unsafe**
- Принимать дополнительные generic параметры

### Кастомные финальные функции

```rust
impl<S: example_builder::IsComplete> ExampleBuilder<S> {
    pub fn custom_build(self) -> Result<Example, Error> {
        let example = self.build();  // Вызываем обычный build
        // Кастомная логика валидации
        validate(&example)?;
        Ok(example)
    }
}
```

---

## Builder Fields — кастомные поля в билдере

С помощью `#[builder(field)]` можно добавить **кастомные приватные поля** в билдер:

```rust
#[derive(Builder)]
#[builder(field(
    name = custom_field,
    type = String,
    default = "default".to_owned()
))]
struct Example {
    x1: u32,
}

impl<S: State> ExampleBuilder<S> {
    fn use_custom_field(&self) {
        println!("{}", self.custom_field);
    }
}
```

**Параметры:**
- `name` — имя поля
- `type` — тип поля
- `default` — начальное значение (опционально)

**Use case:** Хранение состояния между кастомными методами.

---

## Getters — инспекция состояния билдера

Атрибут `#[builder(getter)]` генерирует getter методы, доступные после установки значения.

### Базовый пример

```rust
#[derive(Builder)]
struct Example {
    #[builder(getter)]
    x: u32,
}

let builder = Example::builder().x(1);
let x: &u32 = builder.get_x();
assert_eq!(*x, 1);
```

### Типы возврата

**Обязательные члены:** Возвращают `&T` по умолчанию

**Опциональные члены:** Возвращают `Option<&T>`

```rust
#[derive(Builder)]
struct Example {
    #[builder(getter)]
    x1: Option<u32>,
    
    #[builder(getter, default = 99)]
    x2: u32,  // default тоже возвращает Option<&T>
}

let builder = Example::builder().x1(1).x2(2);
assert_eq!(builder.get_x1(), Some(&1));
assert_eq!(builder.get_x2(), Some(&2));
```

### Кастомизация getters

```rust
#[builder(getter(
    copy,                    // Возврат T через Copy
    clone,                   // Возврат T через Clone
    deref,                   // Возврат &<T as Deref>::Target
    name = custom_name,      // Кастомное имя
    vis = "pub(crate)",      // Кастомная видимость
    doc { /// Custom docs }  // Кастомная документация
))]
```

---

## Fallible Builders — билдеры с валидацией

Три подхода для создания билдеров, возвращающих `Result`.

### Подход 1: Constructor Function

Написать `new()` метод, возвращающий `Result`:

```rust
use bon::bon;

pub struct User { id: u32, name: String }

#[bon]
impl User {
    #[builder]
    pub fn new(id: u32, name: String) -> Result<Self, anyhow::Error> {
        if name.is_empty() {
            return Err(anyhow::anyhow!("Name cannot be empty"));
        }
        Ok(Self { id, name })
    }
}

let result = User::builder()
    .id(42)
    .name(String::new())
    .build();  // Возвращает Result
```

**Плюсы:** Простота  
**Минусы:** Валидация откладывается до `build()`

### Подход 2: Custom Finishing Function

Скрыть сгенерированный `build()` и определить свой:

```rust
#[derive(Builder)]
#[builder(finish_fn(vis = "", name = build_internal))]
pub struct User { id: u32, name: String }

impl<S: user_builder::IsComplete> UserBuilder<S> {
    pub fn build(self) -> Result<User, anyhow::Error> {
        let user = self.build_internal();
        if user.name.is_empty() {
            return Err(anyhow::anyhow!("Name cannot be empty"));
        }
        Ok(user)
    }
}
```

**Плюсы:** Больше контроля  
**Минусы:** Валидация все еще в конце

### Подход 3: Fallible Setters

Валидация при вызове сеттера с помощью `#[builder(with)]`:

```rust
#[derive(Builder)]
struct Example {
    #[builder(with = |s: &str| -> Result<_, ParseIntError> { 
        s.parse() 
    })]
    value: u32,
}

Example::builder()
    .value("42")?   // Сеттер возвращает Result
    .build();
```

**Плюсы:** Ранняя валидация  
**Минусы:** Не все валидации можно сделать на уровне отдельных полей

---

## Conditional Building Patterns — условное построение

Поскольку билдеры bon используют typestate и consuming setters, требуются специфические паттерны для условного кода.

### Паттерн 1: Shared Partial Builder

Извлечь общую настройку, ветвиться с разными завершениями:

```rust
let builder = User::builder()
    .name("Bon")
    .tags(vec!["dev".to_owned()]);

let user = if is_admin {
    builder.role("admin").permissions(all_perms).build()
} else {
    builder.role("user").build()
};
```

**Важно:** Вызывать `.build()` внутри каждой ветки для схождения на одном типе возврата.

### Паттерн 2: Variables with maybe_ Setters

Вычислить значения до построения:

```rust
let extra_role = is_admin.then_some("admin");  // Option<&str>

let user = User::builder()
    .name("Bon")
    .maybe_role(extra_role)   // Принимает Option<T>
    .build();
```

### Паттерн 3: Переменные для всех условий

```rust
let knows_math = 2 + 2 == 4;

let alias = if knows_math { Some("Good girl") } else { None };
let description = if knows_math { 
    "Knows mathematics 🐱" 
} else { 
    "Skipped math classes 😿" 
};

let user = User::builder()
    .name("Bon")
    .maybe_alias(alias)
    .description(description)
    .build();
```

### Комбинирование паттернов

Все три паттерна можно комбинировать в зависимости от сложности условий.

---

## Derives для билдера

Добавить standard derives к самому билдеру:

```rust
#[derive(Builder)]
#[builder(derive(Clone, Debug))]
struct Example {
    name: String,
    level: u32,
}

let builder = Example::builder().name("Bon".to_owned());

println!("{builder:?}");        // Debug
let cloned = builder.clone();   // Clone
```

### Поддерживаемые derives

**`Clone`:** Требует `Clone` для всех членов

**`Debug`:** Требует `Debug` для всех членов
- Формат вывода нестабилен
- Показывает только установленные поля

**`Into`:** Генерирует `From<Builder> for T`

```rust
#[builder(derive(Into))]

// Использование:
let result: User = User::builder()
    .name("Bon".to_owned())
    .into();  // Вместо .build()
```

**`IntoFuture`:** Для async builders

```rust
#[builder(derive(IntoFuture))]

// Использование:
let result = User::builder()
    .name("Bon".to_owned())
    .await;  // Вместо .build()
```

### Кастомные bounds для derives

Если автоматические bounds слишком строгие:

```rust
use std::rc::Rc;

#[derive(Builder)]
#[builder(derive(Clone(bounds(U: Clone))))]
struct Example<T, U> {
    x: Rc<T>,  // Rc<T> клонируется независимо от T: Clone
    y: U,
}
```

---

## Документирование билдеров

### Документация на аргументах функций

В обычном Rust нельзя писать doc comments на аргументах функций. С `#[builder]` — можно! Документация переносится на сеттеры:

```rust
#[bon::builder]
fn greet(
    /// Name of the person to greet.
    /// 
    /// **Example:**
    /// ```
    /// greet().name("John");
    /// ```
    name: &str,
    
    /// Age in full years since birth date.
    age: u32
) -> String {
    format!("Hello {name} with age {age}!")
}
```

### Документация на полях структур

При `#[derive(Builder)]` документация на полях копируется на сеттеры:

```rust
#[derive(Builder)]
struct User {
    /// User's display name
    name: String,
    
    /// Administrative privileges flag
    is_admin: bool,
}
```

### Кастомная документация для generated items

```rust
#[derive(Builder)]
#[builder(
    builder_type(doc { /// Custom builder docs }),
    start_fn(doc { /// Custom builder() docs }),
    finish_fn(doc { /// Custom build() docs })
)]
struct Example {}
```

**Можно документировать:**
- `builder_type` — сам тип билдера
- `start_fn` — стартовая функция (builder())
- `finish_fn` — финальная функция (build()/call())
- Отдельные сеттеры через параметры на членах

### Генерируемая документация

bon автоматически включает в документацию:
- Является ли член обязательным или опциональным
- Дефолтные значения для `#[builder(default)]`
- Информацию о типах для `#[builder(into)]`

---

## Compatibility — совместимость изменений

### Делаем required член optional — безопасно! ✅

Полностью обратно совместимо менять тип с `T` на `Option<T>` или добавлять `#[builder(default)]`:

```rust
// Было:
#[builder]
fn get_page(password: &str) -> String { /* */ }

// Стало:
#[builder]
fn get_page(password: Option<&str>) -> String { /* */ }

// Старый код все еще работает:
get_page().password("secret").call();
```

**Почему безопасно:** Оба (required и optional) имеют сеттер, принимающий `T`. Единственное изменение — добавляется новый `maybe_` сеттер.

### Переключение Option<T> ↔ #[builder(default)] — совместимо! ✅

```rust
// Было:
fn example(filter: Option<String>) {}

// Стало:
fn example(#[builder(default)] filter: String) {}

// Код не меняется:
example().maybe_filter(Some("filter".to_owned())).call();
```

### Префикс _ для unused членов

Leading underscores автоматически убираются из имен сеттеров:

```rust
#[derive(Builder)]
struct Example {
    _name: String  // Временно не используется
}

Example::builder()
    .name("Setter still called `name`".to_owned())
    .build();
```

### Рефакторинг структуры без breaking changes ⭐

**Ключевое преимущество bon:** Можно переключаться между `#[derive(Builder)]` на struct и `#[builder]` на `new()` методе **без breaking changes**.

```rust
// Было:
#[derive(Builder)]
pub struct Line {
    x1: u32, y1: u32,
    x2: u32, y2: u32,
}

// Внутренняя структура изменилась:
struct Point { x: u32, y: u32 }
pub struct Line {
    point1: Point,
    point2: Point,
}

// Публичный API остался прежним:
#[bon]
impl Line {
    #[builder]
    fn new(x1: u32, y1: u32, x2: u32, y2: u32) -> Self {
        Self {
            point1: Point { x: x1, y: y1 },
            point2: Point { x: x2, y: y2 },
        }
    }
}

// Код пользователей не меняется:
Line::builder().x1(1).y1(2).x2(3).y2(4).build();
```

### Сохранение positional API

Если нужно сохранить старый positional API вместе с builder:

```rust
#[builder(expose_positional_fn = positional_name)]
```

**Внимание:** В версии 3.0+ этот атрибут удален. Используйте `start_fn` вместо этого.

---

## Optional Generic Members — избегаем проблем с type inference

### Проблема

Generic type parameters, используемые **только** в опциональных членах, ломают type inference:

```rust
#[bon::builder]
fn bad<T: Into<String>>(x1: Option<T>) {
    let x1 = x1.map(Into::into);
}

// Компилируется:
bad().x1("&str").call();

// НЕ компилируется:
bad().call();
// ERROR: cannot infer type of the type parameter `T`
```

### Решение: #[builder(into)]

Сделать тип члена **неgeneric** и переместить generics в сигнатуру сеттера:

```rust
#[bon::builder]
fn good(#[builder(into)] x1: Option<String>) {
    // ...
}

good().x1("&str").call();  // ✅
good().call();             // ✅
```

### Сравнение generated кода

**С `#[builder(into)]`:**
```rust
fn good() -> GoodBuilder { /**/ }

impl<S: State> GoodBuilder<S> {
    fn x1(self, value: impl Into<String>) -> GoodBuilder<SetX1<S>> {
        // Conversion внутри сеттера
    }
}
```

**С `Option<T: Into>`:**
```rust
fn bad<T>() -> BadBuilder<T> { /**/ }

impl<T: Into<String>, S: State> BadBuilder<T, S> {
    fn x1(self, value: T) -> BadBuilder<T, SetX1<S>> {
        // Generic T торчит наружу
    }
}
```

**Принцип:** Делать конвертации в сеттерах, а не в финальной функции.

---

## Into Conversions In-Depth — детали

### Когда bon НЕ добавляет автоматический Into

1. **Primitive types** (u8, i32, f64, etc.)
   - Причина: ломает type inference для числовых литералов

2. **Explicit `impl Trait`** в параметрах
   - Причина: вложенный `impl Into<impl Into<T>>` усложняет inference

3. **Generic parameters** из сигнатуры функции
   - Причина: аналогично пункту 2

4. **Complex type expressions**
   - Tuples, arrays, references, function pointers

### Явное включение Into

```rust
#[builder(into)]           // На конкретном члене
#[builder(on(Type, into))] // На типе через паттерн
#[builder(on(_, into))]    // На всех типах (wildcard)
```

### Явное отключение Into

```rust
#[builder(into = false)]
```

### Best practices

✅ **DO:** Используйте `on(String, into)` для строковых полей  
✅ **DO:** Используйте `on(PathBuf, into)` для путей  
✅ **DO:** Используйте `#[builder(into)]` для `Box<T>`, `Arc<T>`, `Cow<'a, str>`  
❌ **DON'T:** Не используйте `on(_, into)` без разбора  
❌ **DON'T:** Не используйте для primitive types

---

## Shared Configuration — переиспользование конфигурации

### Проблема дублирования

```rust
#[derive(Builder)]
#[builder(
    on(String, into),
    on(Box<_>, into),
    finish_fn = finish,
)]
struct MyStruct1 { /**/ }

#[derive(Builder)]
#[builder(
    on(String, into),
    on(Box<_>, into),
    finish_fn = finish,
)]
struct MyStruct2 { /**/ }
```

### Решение: macro_rules_attribute

```rust
use macro_rules_attribute::{attribute_alias, apply};

// Объявляем alias с общей конфигурацией
attribute_alias! {
    #[apply(shared_builder!)] =
        #[derive(bon::Builder)]
        #[builder(
            on(String, into),
            on(Box<_>, into),
            finish_fn = finish,
        )];
}

// Используем alias
#[apply(shared_builder!)]
struct MyStruct1 { /**/ }

#[apply(shared_builder!)]
struct MyStruct2 { /**/ }
```

**Преимущества:**
- Единое место конфигурации
- Легко обновлять для всех структур
- Меньше boilerplate

---

## Performance Benchmarks

### Runtime Benchmarks

Builder syntax производит **идентичный assembly** обычным вызовам функций во многих случаях.

| Benchmark | Assembly | Результат |
|-----------|----------|-----------|
| 3 primitive args | Идентичный | Нет overhead |
| 10 primitive args | Идентичный | Нет overhead |
| 10 args с heap alloc | Разный | Builder **на 7% быстрее** |
| 20 primitive args | Идентичный | Нет overhead |

**Вывод:** Builder syntax в release builds имеет zero-cost или даже отрицательный cost.

### Compilation Benchmarks

| Crate | 10 structs / 50 fields | Комментарий |
|-------|------------------------|-------------|
| bon | 2.10s | Typestate проверки |
| typed-builder | 2.09s | Аналогичный overhead |
| derive_builder | 0.45s | Без typestate, runtime валидация |

**Почему bon медленнее derive_builder:**
- bon/typed-builder используют generics для typestate
- derive_builder без generics, но `build()` возвращает `Result`

**Оптимизация:** `#[builder(overwritable)]` — отключить compile-time проверки перезаписи опциональных членов для ускорения компиляции (tradeoff: меньше безопасности).

**Будущие улучшения:** С стабилизацией `associated_type_defaults` в Rust возможно улучшение на **16-58%**.

---

## Troubleshooting — известные ограничения

### 1. `Self` references в doc comments

**Проблема:** `[`Self`]` в документации на членах билдера не работает.

**Решение:** Используйте явное имя типа вместо `Self`.

### 2. Elided lifetime parameters

**Проблема:** Макросы видят tokens, не типы. Неявные lifetimes не видны.

```rust
// ❌ НЕ РАБОТАЕТ:
fn example(value: User)  // Lifetime не указан

// ✅ РАБОТАЕТ:
fn example(value: User<'_>)
```

**Решение:** Включите lint `elided_lifetimes_in_paths` для отлова этого.

### 3. const fn

**Ограничение:** Методы билдера не будут `const`, потому что могут использовать `Into::into`.

### 4. Conditional compilation (#[cfg])

**Проблема:** `#[cfg]` атрибуты на членах не полностью поддерживаются.

**Причина:** Ограничения Rust для атрибутов в where bounds.

### 5. Workarounds

Большинство ограничений обходятся через:
- Явные аннотации lifetime
- Использование function syntax вместо struct derive
- Применение `#[builder(skip)]` для проблемных полей

---

## Alternatives — сравнение с другими крейтами

### Таблица сравнения

| Feature | bon | typed-builder | derive_builder | buildstructor |
|---------|-----|---------------|----------------|---------------|
| Function builders | ✅ | ❌ | ❌ | ✅ |
| Method builders | ✅ | ❌ | ❌ | ✅ |
| Compile-time checked | ✅ | ✅ | ❌ (runtime) | ❌ (runtime) |
| Option<T> auto-optional | ✅ | ❌ | ❌ | ❌ |
| Human-readable typestate | ✅ | ❌ | N/A | N/A |
| Custom methods | ✅ Full | ⚠️ Mutators | ✅ | ⚠️ Limited |
| `impl Trait` support | ✅ | ❌ | ❌ | ⚠️ Partial |
| Clean rustdoc | ✅ | ❌ | ✅ | ⚠️ |
| `#[builder(default)]` | ✅ | ✅ | ✅ | ❌ |
| `#[builder(into)]` | ✅ | ✅ | ❌ | ❌ |

### Ключевые преимущества bon

#### 1. Function-based builders

Возможность переключаться между `#[derive(Builder)]` и `#[builder]` на `new()` **без breaking changes**:

```rust
// Начали со struct derive
#[derive(Builder)]
pub struct Line { x1: u32, y1: u32, x2: u32, y2: u32 }

// Рефакторинг внутренней структуры
pub struct Line { point1: Point, point2: Point }

#[bon]
impl Line {
    #[builder]
    fn new(x1: u32, y1: u32, x2: u32, y2: u32) -> Self {
        // Новая реализация
    }
}

// API пользователей НЕ МЕНЯЕТСЯ!
```

#### 2. Flexibility без стены

- typed-builder/derive_builder: упираетесь в стену → переписываете вручную
- bon/buildstructor: переключаетесь на function syntax → полная гибкость

#### 3. Human-readable typestate

```rust
// bon:
ExampleBuilder<SetX2<SetX1>>

// typed-builder:
TypedBuilderBuilder<((Private,), (i32,), ())>
// ^^^^ Leaked private type, tuple hell
```

#### 4. Clean documentation

bon генерирует чистую документацию без шума:
- Показывает дефолтные значения
- Trait-based design без generic noise
- Включает информацию о required/optional

#### 5. No panics

bon никогда не паникует в runtime. Все ошибки — на этапе компиляции.

### Когда использовать альтернативы

**derive_builder:**
- Нужна максимальная скорость компиляции
- Не критична compile-time валидация
- Runtime `Result` от `build()` приемлем

**typed-builder:**
- Нужен typestate, но не нужны function builders
- Не важна читаемость typestate
- Не планируется расширение через custom methods

**buildstructor:**
- Нужны только function builders
- Не нужна compile-time валидация
- Устраивает runtime проверки

---

## Advanced Patterns

### Pattern 1: Builder с кастомными полями

```rust
#[derive(Builder)]
#[builder(field(name = config, type = Config, default = Config::default()))]
struct Request {
    url: String,
    method: String,
}

impl<S: State> RequestBuilder<S> {
    fn apply_config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }
    
    fn with_default_config(self) -> Self {
        self.apply_config(Config::default())
    }
}
```

### Pattern 2: Builder Chains для сложных объектов

```rust
#[derive(Builder)]
struct Database {
    host: String,
    port: u16,
    #[builder(default)]
    pool_size: usize,
}

impl Database {
    fn connection_builder(&self) -> ConnectionBuilder {
        Connection::builder()
            .database(self.clone())
    }
}

let db = Database::builder()
    .host("localhost".to_owned())
    .port(5432)
    .build();

let conn = db.connection_builder()
    .user("admin")
    .build();
```

### Pattern 3: Conditional Type States

```rust
trait HasCredentials {}
trait NoCredentials {}

impl<S: State> RequestBuilder<S> where S::Auth: NoCredentials {
    fn with_api_key(self, key: String) -> RequestBuilder<SetAuth<HasCredentials, S>> {
        // ...
    }
}

impl<S: State> RequestBuilder<S> where S::Auth: HasCredentials {
    fn execute(self) -> Result<Response> {
        // Можно вызвать только если есть credentials
    }
}
```

---

## Best Practices

### 1. Именование билдеров

✅ **DO:**
```rust
#[builder(start_fn = with_config)]  // Описательное имя
#[builder(finish_fn = connect)]     // Действие
```

❌ **DON'T:**
```rust
#[builder(start_fn = new)]   // Общее имя
#[builder(finish_fn = done)] // Vague
```

### 2. Группировка related полей

```rust
#[derive(Builder)]
struct Server {
    // Network settings
    #[builder(start_fn)]
    host: String,
    #[builder(start_fn)]
    port: u16,
    
    // Auth settings
    api_key: Option<String>,
    secret: Option<String>,
    
    // Advanced settings
    #[builder(default = 10)]
    timeout: u64,
}
```

### 3. Документирование constraints

```rust
#[builder]
fn create_user(
    /// Username must be 3-20 characters, alphanumeric only.
    /// 
    /// # Examples
    /// ```
    /// create_user().username("john_doe123");
    /// ```
    username: String,
) -> Result<User> {
    // Валидация
}
```

### 4. Использование Into разумно

```rust
#[builder(on(String, into))]     // ✅ Хорошо
#[builder(on(PathBuf, into))]    // ✅ Хорошо
#[builder(on(Vec<_>, into))]     // ⚠️ Подумайте дважды
#[builder(on(_, into))]          // ❌ Слишком широко
```

### 5. Error Handling

```rust
// ✅ Хорошо: Early validation
#[builder(with = |s: &str| -> Result<_, ParseError> { 
    s.parse() 
})]
value: u32,

// ⚠️ Допустимо: Late validation
#[builder]
fn new(value: String) -> Result<Self> {
    validate(&value)?;
    Ok(Self { value })
}
```

---

## Quick Reference Card

### Основные атрибуты

```rust
// Структуры
#[derive(Builder)]

// Функции и методы
#[builder]
#[bon]  // На impl блоке для методов
```

### Члены

```rust
#[builder(default)]              // Дефолтное значение (Default trait)
#[builder(default = expr)]       // Кастомный дефолт
#[builder(into)]                 // impl Into<T> сеттер
#[builder(with = closure)]       // Кастомная логика
#[builder(skip)]                 // Исключить из билдера
#[builder(start_fn)]             // Позиционный в начале
#[builder(finish_fn)]            // Позиционный в конце
#[builder(required)]             // Форсить Option<T> быть обязательным
#[builder(getter)]               // Генерировать getter
```

### Top-level конфигурация

```rust
#[builder(
    on(Type, into),              // Into для типа
    start_fn = name,             // Имя стартовой функции
    finish_fn = name,            // Имя финальной функции
    derive(Clone, Debug),        // Derives для билдера
    state_mod(vis = "pub"),      // Видимость typestate модуля
)]
```

### Typestate API

```rust
use builder_module::{State, IsUnset, IsSet, IsComplete};

impl<S: State> Builder<S> {
    fn custom_method(self) -> Builder<NewState<S>>
    where S::Member: IsUnset { /* */ }
}
```

---

## Migration Guide: v2 → v3

### Основные breaking changes (минимальные)

1. **Порядок вызова сеттеров влияет на typestate:**
   ```rust
   // v2: Всегда одинаковый тип
   // v3: Порядок важен
   let b1 = builder().x1(1).x2(2);  // SetX2<SetX1>
   let b2 = builder().x2(2).x1(1);  // SetX1<SetX2>
   ```

2. **Удален `#[bon::builder]` на structs:**
   ```rust
   // ❌ v2:
   #[bon::builder]
   struct Example { }
   
   // ✅ v3:
   #[derive(bon::Builder)]
   struct Example { }
   ```

3. **Удален `expose_positional_fn`:**
   ```rust
   // ❌ v2:
   #[builder(expose_positional_fn = name)]
   
   // ✅ v3:
   #[builder(start_fn = name)]
   ```

99% кода обновляется без изменений!

---

## FAQ

**Q: Почему компиляция медленнее?**  
A: Typestate generics. Tradeoff за compile-time безопасность. Используйте `#[builder(overwritable)]` для ускорения.

**Q: Можно ли использовать с async?**  
A: Да! `#[builder] async fn` работает, `#[builder(derive(IntoFuture))]` для `.await`.

**Q: Как добавить валидацию?**  
A: Три способа: fallible constructor, custom finish_fn, fallible setters с `#[builder(with)]`.

**Q: Совместим ли с serde?**  
A: Да, но билдер — для construction, не для serialization. Используйте serde на самой структуре.

**Q: Работает в no_std?**  
A: Да! Используйте `default-features = false`.

**Q: Как debugить typestate ошибки?**  
A: Читайте ошибки внимательно — они указывают какой Member отсутствует. Используйте `cargo expand` для просмотра generated кода.

**Q: Можно ли использовать с generics?**  
A: Да, но избегайте generics ТОЛЬКО в optional members (проблемы с inference).

**Q: Как тестировать билдеры?**  
A: Тестируйте результирующие объекты, не сами билдеры. Компилятор гарантирует корректность билдера.

---

## Resources

- **Официальный сайт:** [bon-rs.com](https://bon-rs.com)
- **Docs.rs:** [docs.rs/bon](https://docs.rs/bon)
- **GitHub:** [github.com/elastio/bon](https://github.com/elastio/bon)
- **Discord:** [Официальный Discord](https://bon-rs.com/discord)
- **Блог:** [bon-rs.com/blog](https://bon-rs.com/blog)

### Полезные статьи

- [How to do named function arguments in Rust](https://bon-rs.com/blog/how-to-do-named-function-arguments-in-rust)
- [Bon 3.0 Release - Revolutionary typestate design](https://bon-rs.com/blog/bon-v3-release)
- [Bon 2.0 Release](https://bon-rs.com/blog/bon-builder-generator-v2-release)

---

## Заключение

**bon** — это мощный и гибкий инструмент для создания билдеров в Rust с compile-time гарантиями безопасности. Основные преимущества:

✅ Compile-time проверки всех параметров  
✅ Zero-cost abstractions в release builds  
✅ Function-based builders для максимальной гибкости  
✅ Human-readable typestate API  
✅ Seamless compatibility при рефакторинге  
✅ Чистая документация без шума  
✅ Production-ready (используется в crates.io)  

Начните с простого `#[derive(Builder)]` и постепенно изучайте advanced features по мере необходимости. bon растет вместе с вашими потребностями!

---

**Версия документации:** 3.8  
**Дата:** Декабрь 2024  
**Автор компиляции:** Comprehensive guide based on bon-rs.com  
**License:** Apache-2.0 / MIT (как и сам bon)