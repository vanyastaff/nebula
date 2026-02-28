# Archived From "docs/archive/nebula-parameters.md"

# Система параметров Nebula - Полная документация

## 📖 Введение

Система параметров Nebula предоставляет типобезопасное создание конфигурационных форм для Actions и Credentials в платформе рабочих процессов. Каждый параметр представляет поле ввода с метаданными, валидацией и минимальными опциями UI.

### Ключевые принципы

- **Типовая безопасность** - каждый параметр строго типизирован
- **Минимальные опции UI** - только критические бизнес-настройки, которые фундаментально меняют поведение
- **Ответственность ядра платформы** - ядро автоматически обрабатывает все стандартные поведения
- **Чистая архитектура** - параметры определяют типы данных и валидацию, а не визуальный вид
- **Единые стандарты** - платформа контролирует внешний вид, размеры, общие поведения
- **Композиция** - сложные формы из простых, сфокусированных компонентов
- **Поддержка выражений** - любой параметр может использовать выражения через ядро платформы

### 🚨 Критические архитектурные принципы

**НЕ добавляйте опции UI, которые должно обрабатывать ядро платформы:**
- ❌ Высоты, ширины, размеры (`height: 10`, `cols: 80`)
- ❌ Визуальная стилизация (цвета, темы, отступы)
- ❌ Стандартные поведения (счетчики символов, авто-форматирование)
- ❌ Переменные выражений (`available_variables: vec!["$json"]`)
- ❌ Конфигурация автодополнения
- ❌ Настройки предпросмотра (`show_preview: true`)
- ❌ Стандартные помощники валидации (`show_schema_hints`)

**ВКЛЮЧАЙТЕ опции UI, которые меняют фундаментальное поведение:**
- ✅ Типы данных и ограничения (`min/max`, `required`)
- ✅ Типы ввода, которые меняют валидацию (`TextInputType::Email`)
- ✅ Язык для подсветки синтаксиса (`CodeLanguage::JavaScript`)
- ✅ Критические поведенческие различия (`multiline: true`)
- ✅ Бизнес-логические ограничения (`creatable: true` для селектов)
- ✅ Выражения должны быть внутри кода в CodeParameter(не снаружи)

**Ядро платформы обрабатывает автоматически:**
- Кнопки переключения выражений и определение режима
- Автодополнение переменных ($json, $node, $workflow и т.д.)
- Стандартные размеры и адаптивный макет
- Цветовые темы и визуальная стилизация
- Общие поведения (обратная связь валидации, состояния загрузки)
- Функции доступности
- Обработка ошибок и восстановление

## 💡 Архитектура выражений

### ParameterValue с поддержкой выражений

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParameterValue {
    String(String),
    Number(f64),
    Boolean(bool),
    Array(Vec<ParameterValue>),
    Object(HashMap<String, ParameterValue>),
    DateTime(DateTime<Utc>),
    File(FileData),
    Color(String),
    
    // Обертка выражения для любого значения
    Expression(String),
}
```

### Конвейер выполнения выражений

**Фаза 1: Преобразование** - Преобразование выражений в конкретные значения
```rust
// Ввод из базы данных
let raw_value = ParameterValue::Expression("{{$json.baseUrl}}/users");

// Преобразование с контекстом
let context = ExecutionContext {
    json: previous_step_output,
    node: current_node_data,
    workflow: workflow_context,
};

let transformed = transform_expression(raw_value, &context)?;
// Результат: ParameterValue::String("https://api.example.com/users")
```

**Фаза 2: Валидация** - Валидация преобразованных значений против определений параметров
```rust
let validated = validate_parameters(transformed_values, &parameter_definitions)?;
```

**Фаза 3: Обработка** - Выполнение действия с чистыми значениями
```rust
let result = action.execute(validated_values)?;
```

### Архитектура UI на стороне клиента

**Подход с двумя полями для максимального UX:**
```rust
// Ввод параметра на стороне клиента
pub struct ParameterInput {
    pub key: String,
    pub static_value: String,     // Обычное поле ввода
    pub expression_value: String, // Поле выражения
    pub current_mode: InputMode,  // Какое поле показывать
}

pub enum InputMode {
    Static,
    Expression,
}
```

**Преобразование: Клиент ↔ База данных:**
```rust
// ИЗ базы данных В клиент (загрузка формы)
fn parameter_value_to_input(value: &ParameterValue) -> ParameterInput {
    match value {
        ParameterValue::Expression(expr) => ParameterInput {
            static_value: String::new(),
            expression_value: expr.clone(),
            current_mode: InputMode::Expression,
        },
        ParameterValue::String(s) => ParameterInput {
            static_value: s.clone(),
            expression_value: String::new(),
            current_mode: InputMode::Static,
        },
        // ... другие типы
    }
}

// ИЗ клиента В базу данных (сохранение)
fn input_to_parameter_value(input: &ParameterInput, param_type: &ParameterType) -> Option<ParameterValue> {
    match input.current_mode {
        InputMode::Expression => {
            if input.expression_value.trim().is_empty() {
                None
            } else {
                Some(ParameterValue::Expression(input.expression_value.clone()))
            }
        },
        InputMode::Static => {
            // Разбор в соответствии с типом параметра
            match param_type {
                ParameterType::Text => Some(ParameterValue::String(input.static_value.clone())),
                ParameterType::Number => input.static_value.parse::<f64>().ok().map(ParameterValue::Number),
                // ... другие типы
            }
        }
    }
}
```

**Определение режима выражения:**
```rust
fn get_input_mode(value: &Option<ParameterValue>) -> InputMode {
    match value {
        Some(ParameterValue::Expression(expr)) => {
            if is_valid_expression(expr) {
                InputMode::Expression
            } else {
                // Недопустимое выражение → показать как статическое с пустым полем
                InputMode::Static
            }
        }
        _ => InputMode::Static,
    }
}

fn is_valid_expression(expr: &str) -> bool {
    let trimmed = expr.trim();
    
    // Должно содержать хотя бы одну пару {{}}
    if !trimmed.contains("{{") || !trimmed.contains("}}") {
        return false;
    }
    
    // Проверка, что все {{}} правильно спарены
    let mut brace_count = 0;
    let mut chars = trimmed.chars().peekable();
    
    while let Some(c) = chars.next() {
        if c == '{' && chars.peek() == Some(&'{') {
            chars.next(); // потребить вторую {
            brace_count += 1;
        } else if c == '}' && chars.peek() == Some(&'}') {
            chars.next(); // потребить вторую }
            brace_count -= 1;
            if brace_count < 0 {
                return false;
            }
        }
    }
    
    brace_count == 0 // все пары должны быть закрыты
}
```

**Функции платформы (автоматические):**
- Кнопка переключения между статическим/выражением режимами
- Автодополнение для переменных ($json, $node, $workflow)
- Предпросмотр выражения
- Подсветка синтаксиса для {{}} выражений
- Валидация ошибок для неправильно сформированных выражений

## 🔧 Базовые компоненты

### ParameterMetadata

Основная информация о параметре:

```rust
pub struct ParameterMetadata {
    /// Уникальный ключ параметра
    pub key: ParameterKey,
    
    /// Отображаемое имя
    pub name: Cow<'static, str>,
    
    /// Является ли параметр обязательным
    pub required: bool,
    
    /// Описание параметра
    pub description: Option<Cow<'static, str>>,
    
    /// Текст-заполнитель для пустого поля
    pub placeholder: Option<Cow<'static, str>>,
    
    /// Дополнительная информация или инструкции
    pub hint: Option<Cow<'static, str>>,
}
```

### Создание метаданных

```rust
// Простое создание
let metadata = ParameterMetadata::simple("api_key", "API Key")?;

// С дополнительной информацией
let metadata = ParameterMetadata::builder()
    .key("timeout")
    .name("Request Timeout")
    .required(false)
    .description("Maximum time to wait for API response")
    .placeholder("30")
    .hint("Value in seconds, between 1 and 300")
    .build()?;
```

## 📝 Типы параметров

### 1. TextParameter

**Назначение:** Текстовый ввод для строковых данных (однострочный и многострочный).

**Когда использовать:**
- Имена пользователей, заголовки
- URL-адреса, email-адреса
- Однострочный и многострочный текст
- Временные пароли для форм входа
- Длинные описания и комментарии

**Хранимые данные:** `ParameterValue::String(String)` или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "String",
  "value": "John Doe"
}

// Выражение
{
  "type": "Expression",
  "value": "{{$json.user.firstName}} {{$json.user.lastName}}"
}
```

**Примеры кода:**
```rust
// Однострочный текст
let name = TextParameter::builder()
    .metadata(ParameterMetadata::simple("name", "User Name")?)
    .ui_options(TextUiOptions {
        input_type: TextInputType::Text,
        multiline: false,
    })
    .build()?;

// Email с валидацией
let email = TextParameter::builder()
    .metadata(metadata)
    .ui_options(TextUiOptions {
        input_type: TextInputType::Email,
        multiline: false,
    })
    .build()?;

// Многострочное описание
let description = TextParameter::builder()
    .metadata(metadata)
    .ui_options(TextUiOptions {
        input_type: TextInputType::Text,
        multiline: true,
        rows: Some(5), // Только когда критично для UX
    })
    .build()?;

// URL с валидацией
let website = TextParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("website")
        .name("Website URL")
        .required(true)
        .placeholder("https://example.com")
        .build()?)
    .ui_options(TextUiOptions {
        input_type: TextInputType::URL,
        multiline: false,
    })
    .build()?;

// Поле поиска
let search = TextParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("search")
        .name("Search Query")
        .placeholder("Enter search terms...")
        .build()?)
    .ui_options(TextUiOptions {
        input_type: TextInputType::Search,
        multiline: false,
    })
    .build()?;
```

**Опции UI:**
- `input_type` - тип ввода (Text, Password, Email, URL, Tel, Search)
- `multiline` - включить многострочный режим
- `rows` - высота для многострочного режима (только когда критично для UX)

**Примечание:** Большинство текстового поведения (форматирование, лимиты символов, стилизация) обрабатывается ядром платформы.

---

### 2. SecretParameter

**Назначение:** Безопасное хранение конфиденциальных данных с автоматическим обнулением памяти.

**Когда использовать:**
- API ключи и токены
- Пароли баз данных
- OAuth секреты
- Любые долгосрочные учетные данные

**Хранимые данные:** `ParameterValue::String(String)` (зашифровано) или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение (зашифровано)
{
  "type": "String",
  "value": "encrypted:AES256:base64encodeddata...",
  "encrypted": true
}

// Выражение для динамических секретов
{
  "type": "Expression",
  "value": "{{$workflow.secrets.apiKey}}"
}
```

**Примечание:** Режим выражений полезен для динамических секретов вроде `"{{$workflow.secrets.apiKey}}"`, но выражения вычисляются во время выполнения, а не хранятся как обычные секреты.

**Примеры кода:**
```rust
// API ключ
let api_key = SecretParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("api_key")
        .name("API Key")
        .required(true)
        .description("Your service API key")
        .placeholder("sk-...")
        .build()?)
    .build()?;

// Пароль базы данных
let db_password = SecretParameter::builder()
    .metadata(ParameterMetadata::required("db_password", "Database Password")?)
    .build()?;

// OAuth токен (только для чтения)
let oauth_token = SecretParameter::builder()
    .metadata(metadata)
    .readonly(true)  // Генерируется автоматически
    .build()?;

// Webhook секрет
let webhook_secret = SecretParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("webhook_secret")
        .name("Webhook Secret")
        .description("Secret for validating webhook signatures")
        .hint("Will be used to compute HMAC signatures")
        .build()?)
    .build()?;
```

**Безопасность:**
- Автоматическое обнуление памяти при удалении
- Маскирование в логах и отладочном выводе
- Шифрование при сериализации
- Защита от случайного отображения

---

### 3. NumberParameter

**Назначение:** Числовой ввод с валидацией и форматированием.

**Когда использовать:**
- Таймауты, лимиты, счетчики
- Цены, проценты
- Настройки производительности
- Любые числовые значения

**Хранимые данные:** `ParameterValue::Number(f64)` или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "Number",
  "value": 30.0
}

// Выражение
{
  "type": "Expression",
  "value": "{{$json.responseTime * 2}}"
}
```

**Примеры выражений:**
- Статическое: `30.0` → таймаут в секундах
- Выражение: `"{{$json.responseTime * 2}}"` → динамический расчет таймаута

**Примеры кода:**
```rust
// Таймаут в секундах
let timeout = NumberParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("timeout")
        .name("Request Timeout")
        .description("Maximum time to wait for response")
        .placeholder("30")
        .hint("Value in seconds")
        .build()?)
    .ui_options(NumberUiOptions {
        format: NumberFormat::Integer,
        min: Some(1.0),
        max: Some(300.0),
        step: Some(1.0),
        unit: Some("seconds".into()),
    })
    .build()?;

// Цена в валюте
let price = NumberParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("price")
        .name("Product Price")
        .required(true)
        .build()?)
    .ui_options(NumberUiOptions {
        format: NumberFormat::Currency,
        min: Some(0.0),
        max: None,
        step: Some(0.01),
        unit: Some("USD".into()),
    })
    .build()?;

// Процент (0-100)
let confidence = NumberParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("confidence")
        .name("Confidence Level")
        .description("How confident are you?")
        .build()?)
    .ui_options(NumberUiOptions {
        format: NumberFormat::Percentage,
        min: Some(0.0),
        max: Some(100.0),
        step: Some(1.0),
        unit: None,
    })
    .build()?;

// Десятичное число
let temperature = NumberParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("temperature")
        .name("Temperature")
        .build()?)
    .ui_options(NumberUiOptions {
        format: NumberFormat::Decimal,
        min: Some(-273.15), // Абсолютный ноль
        max: None,
        step: Some(0.1),
        unit: Some("°C".into()),
    })
    .build()?;
```

**Опции UI:**
- `format` - формат числа (Integer, Decimal, Currency, Percentage)
- `min/max` - ограничения значений
- `step` - шаг увеличения
- `unit` - единица измерения

---

### 4. BooleanParameter

**Назначение:** Булевы значения для включения/выключения опций.

**Когда использовать:**
- Флаги переключения функций
- Настройки да/нет
- Принятие условий

**Хранимые данные:** `ParameterValue::Boolean(bool)` или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "Boolean",
  "value": true
}

// Выражение
{
  "type": "Expression",
  "value": "{{$json.isPremium && $json.verified}}"
}
```

**Примеры выражений:**
- Статическое: `true` → SSL включен
- Выражение: `"{{$json.isPremium && $json.verified}}"` → условная логика

**Примеры кода:**
```rust
// Включить SSL
let use_ssl = BooleanParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("use_ssl")
        .name("Use SSL")
        .required(false)
        .description("Enable SSL encryption")
        .build()?)
    .default(true)
    .build()?;

// Принятие условий
let accept_terms = BooleanParameter::builder()
    .metadata(ParameterMetadata::required("accept_terms", "Accept Terms")?)
    .default(false)
    .build()?;

// Режим отладки
let debug_mode = BooleanParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("debug")
        .name("Debug Mode")
        .description("Enable debug logging")
        .build()?)
    .default(false)
    .build()?;

// Автоматический повтор
let auto_retry = BooleanParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("auto_retry")
        .name("Auto Retry")
        .description("Automatically retry failed requests")
        .hint("Will retry up to 3 times with exponential backoff")
        .build()?)
    .default(true)
    .build()?;
```

---

### 5. SelectParameter

**Назначение:** Выбор одного значения из предопределенного списка.

**Когда использовать:**
- HTTP методы, протоколы
- Статические списки опций
- Категории, типы
- Выбор из ограниченного набора

**Хранимые данные:** `ParameterValue::String(String)` или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "String",
  "value": "POST"
}

// Выражение
{
  "type": "Expression",
  "value": "{{$json.requestType || 'GET'}}"
}
```

**Примеры выражений:**
- Статическое: `"POST"` → фиксированный HTTP метод
- Выражение: `"{{$json.requestType || 'GET'}}"` → динамический выбор метода

**Примеры кода:**
```rust
// HTTP метод
let method = SelectParameter::builder()
    .metadata(ParameterMetadata::required("method", "HTTP Method")?)
    .options(vec![
        SelectOption::new("GET", "GET"),
        SelectOption::new("POST", "POST"),
        SelectOption::new("PUT", "PUT"),
        SelectOption::new("DELETE", "DELETE"),
        SelectOption::new("PATCH", "PATCH"),
    ])
    .ui_options(SelectUiOptions {
        searchable: false,
        creatable: false,
    })
    .build()?;

// Большой список с поиском (страны)
let country = SelectParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("country")
        .name("Country")
        .required(true)
        .placeholder("Select a country")
        .build()?)
    .options(vec![
        SelectOption::new("us", "United States"),
        SelectOption::new("uk", "United Kingdom"),
        SelectOption::new("ca", "Canada"),
        SelectOption::new("au", "Australia"),
        // ... много других стран
    ])
    .ui_options(SelectUiOptions {
        searchable: true,
        creatable: false,
    })
    .build()?;

// Combobox (можно добавить новое)
let tag = SelectParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("tag")
        .name("Tag")
        .description("Select or create a new tag")
        .build()?)
    .options(vec![
        SelectOption::new("bug", "Bug"),
        SelectOption::new("feature", "Feature"),
        SelectOption::new("enhancement", "Enhancement"),
    ])
    .ui_options(SelectUiOptions {
        searchable: true,
        creatable: true, // Позволяет создавать новые значения
    })
    .build()?;

// Уровень логирования
let log_level = SelectParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("log_level")
        .name("Log Level")
        .description("Minimum log level to capture")
        .build()?)
    .options(vec![
        SelectOption::new("debug", "Debug"),
        SelectOption::new("info", "Info"),
        SelectOption::new("warn", "Warning"),
        SelectOption::new("error", "Error"),
    ])
    .default("info")
    .ui_options(SelectUiOptions {
        searchable: false,
        creatable: false,
    })
    .build()?;
```

**Опции UI:**
- `searchable` - включить поиск по опциям
- `creatable` - разрешить создание новых значений

---

### 6. MultiSelectParameter

**Назначение:** Выбор нескольких значений из списка.

**Когда использовать:**
- Права доступа, роли
- Теги, категории
- Множественные настройки

**Хранимые данные:** `ParameterValue::Array(Vec<ParameterValue>)` или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "Array",
  "value": [
    {"type": "String", "value": "read"},
    {"type": "String", "value": "write"}
  ]
}

// Выражение
{
  "type": "Expression",
  "value": "{{$json.user.roles}}"
}
```

**Примеры выражений:**
- Статическое: `["read", "write"]` → фиксированные разрешения
- Выражение: `"{{$json.user.roles}}"` → динамическое назначение ролей

**Примеры кода:**
```rust
// Разрешения пользователя
let permissions = MultiSelectParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("permissions")
        .name("User Permissions")
        .description("Select user permissions")
        .required(true)
        .build()?)
    .options(vec![
        SelectOption::new("read", "Read Access"),
        SelectOption::new("write", "Write Access"),
        SelectOption::new("delete", "Delete Access"),
        SelectOption::new("admin", "Admin Access"),
    ])
    .constraints(MultiSelectConstraints {
        min_selections: Some(1), // Минимум одно разрешение
        max_selections: None,
    })
    .build()?;

// Теги статьи
let tags = MultiSelectParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("article_tags")
        .name("Article Tags")
        .description("Add tags to your article")
        .build()?)
    .options(vec![
        SelectOption::new("tech", "Technology"),
        SelectOption::new("business", "Business"),
        SelectOption::new("health", "Health"),
        SelectOption::new("science", "Science"),
        SelectOption::new("sports", "Sports"),
    ])
    .constraints(MultiSelectConstraints {
        min_selections: None,
        max_selections: Some(5), // Максимум 5 тегов
    })
    .build()?;

// Языки программирования
let languages = MultiSelectParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("languages")
        .name("Programming Languages")
        .description("Select languages you're proficient in")
        .build()?)
    .options(vec![
        SelectOption::new("rust", "Rust"),
        SelectOption::new("python", "Python"),
        SelectOption::new("javascript", "JavaScript"),
        SelectOption::new("go", "Go"),
        SelectOption::new("java", "Java"),
    ])
    .constraints(MultiSelectConstraints {
        min_selections: Some(1),
        max_selections: Some(10),
    })
    .build()?;
```

**Ограничения:**
- `min_selections` - минимальное количество выборов
- `max_selections` - максимальное количество выборов

---

### 7. RadioParameter

**Назначение:** Эксклюзивный выбор с визуальным представлением радиокнопки.

**Когда использовать:**
- Выбор метода аутентификации
- Режимы работы
- Взаимоисключающие опции

**Хранимые данные:** `ParameterValue::String(String)` или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "String",
  "value": "oauth"
}

// Выражение
{
  "type": "Expression",
  "value": "{{$json.hasApiKey ? 'api_key' : 'basic'}}"
}
```

**Примеры выражений:**
- Статическое: `"oauth"` → фиксированный метод авторизации
- Выражение: `"{{$json.hasApiKey ? 'api_key' : 'basic'}}"` → условный выбор авторизации

**Примеры кода:**
```rust
// Метод аутентификации
let auth_method = RadioParameter::builder()
    .metadata(ParameterMetadata::required("auth_method", "Authentication Method")?)
    .options(vec![
        RadioOption {
            value: "basic".into(),
            label: "Basic Auth".into(),
            description: Some("Username and password".into()),
            icon: Some("user".into()),
            disabled: false,
        },
        RadioOption {
            value: "oauth".into(),
            label: "OAuth 2.0".into(),
            description: Some("OAuth authentication".into()),
            icon: Some("key".into()),
            disabled: false,
        },
        RadioOption {
            value: "api_key".into(),
            label: "API Key".into(),
            description: Some("API key authentication".into()),
            icon: Some("lock".into()),
            disabled: false,
        },
    ])
    .ui_options(RadioUiOptions {
        layout: RadioLayout::Vertical,
        show_descriptions: true,
        show_icons: true,
    })
    .build()?;

// Режим работы
let operation_mode = RadioParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("mode")
        .name("Operation Mode")
        .description("Select how the action should operate")
        .build()?)
    .options(vec![
        RadioOption {
            value: "test".into(),
            label: "Test Mode".into(),
            description: Some("Run in test mode without making changes".into()),
            icon: Some("flask".into()),
            disabled: false,
        },
        RadioOption {
            value: "production".into(),
            label: "Production Mode".into(),
            description: Some("Run in production mode".into()),
            icon: Some("rocket".into()),
            disabled: false,
        },
    ])
    .default("test")
    .ui_options(RadioUiOptions {
        layout: RadioLayout::Horizontal,
        show_descriptions: true,
        show_icons: true,
    })
    .build()?;
```

**Опции UI:**
- `layout` - макет (Vertical, Horizontal, Grid)
- `show_descriptions` - показывать описания
- `show_icons` - показывать иконки

---

### 8. DateTimeParameter

**Назначение:** Ввод даты, времени или комбинированный.

**Когда использовать:**
- Планирование выполнения
- Фильтрация по дате
- События, дедлайны

**Хранимые данные:** `ParameterValue::DateTime(DateTime<Utc>)` или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "DateTime",
  "value": "2024-12-25T09:00:00Z"
}

// Выражение
{
  "type": "Expression",
  "value": "{{new Date(Date.now() + 24*60*60*1000).toISOString()}}"
}
```

**Примеры выражений:**
- Статическое: `"2024-12-25T09:00:00Z"` → фиксированная дата
- Выражение: `"{{$json.scheduledDate}}"` → динамическое планирование
- Выражение: `"{{new Date(Date.now() + 24*60*60*1000).toISOString()}}"` → завтра

**Примеры кода:**
```rust
// Только дата
let birth_date = DateTimeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("birth_date")
        .name("Date of Birth")
        .required(true)
        .build()?)
    .ui_options(DateTimeUiOptions {
        mode: DateTimeMode::DateOnly,
        timezone: TimezoneHandling::UTC,
        min_date: None,
        max_date: Some(today()),
    })
    .build()?;

// Дата и время с часовым поясом
let schedule = DateTimeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("scheduled_at")
        .name("Schedule Time")
        .description("When to execute this task")
        .build()?)
    .ui_options(DateTimeUiOptions {
        mode: DateTimeMode::DateTime,
        timezone: TimezoneHandling::UserLocal,
        min_date: Some(today()),
        max_date: None,
    })
    .build()?;

// Только время
let daily_run = DateTimeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("daily_run_time")
        .name("Daily Run Time")
        .description("Time to run daily task")
        .build()?)
    .ui_options(DateTimeUiOptions {
        mode: DateTimeMode::TimeOnly,
        timezone: TimezoneHandling::UserLocal,
        min_date: None,
        max_date: None,
    })
    .build()?;

// Дата начала проекта
let project_start = DateTimeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("project_start")
        .name("Project Start Date")
        .hint("Project cannot start in the past")
        .build()?)
    .ui_options(DateTimeUiOptions {
        mode: DateTimeMode::DateOnly,
        timezone: TimezoneHandling::UTC,
        min_date: Some(tomorrow()),
        max_date: Some(next_year()),
    })
    .build()?;
```

**Опции UI:**
- `mode` - режим (DateOnly, TimeOnly, DateTime)
- `timezone` - обработка часового пояса (UTC, UserLocal, Custom)
- `min_date/max_date` - ограничения даты

---

### 9. CodeParameter

**Назначение:** Редактор кода с подсветкой синтаксиса и автодополнением.

**Когда использовать:**
- JavaScript выражения
- SQL запросы
- JSON шаблоны
- HTML/CSS код

**Хранимые данные:** `ParameterValue::String(String)` или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статический код
{
  "type": "String",
  "value": "SELECT * FROM users WHERE active = true"
}

// Выражение для динамической генерации кода
{
  "type": "Expression",
  "value": "{{$json.customQuery || 'SELECT * FROM users'}}"
}
```

**Примечание:** CodeParameter обычно хранит статический код, но может использовать выражения для динамической генерации кода:
- Статическое: `"SELECT * FROM users WHERE active = true"`
- Выражение: `"{{$json.customQuery || 'SELECT * FROM users'}}"`

**Примеры кода:**
```rust
// JavaScript выражение
let expression = CodeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("transform")
        .name("Data Transform")
        .description("JavaScript code to transform data")
        .build()?)
    .ui_options(CodeUiOptions {
        language: CodeLanguage::JavaScript,
        height: 6,
        available_variables: vec![
            "$json".into(),
            "$node".into(),
            "$workflow".into(),
        ],
    })
    .default("// Transform input data\nreturn $json;")
    .build()?;

// SQL запрос
let query = CodeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("query")
        .name("SQL Query")
        .required(true)
        .placeholder("SELECT * FROM table_name")
        .build()?)
    .ui_options(CodeUiOptions {
        language: CodeLanguage::SQL,
        height: 10,
        available_variables: vec![
            "$input".into(),
            "$params".into(),
        ],
    })
    .build()?;

// JSON конфигурация
let json_config = CodeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("config")
        .name("JSON Configuration")
        .description("Configuration in JSON format")
        .build()?)
    .ui_options(CodeUiOptions {
        language: CodeLanguage::JSON,
        height: 8,
        available_variables: vec![],
    })
    .default("{\n  \"enabled\": true,\n  \"timeout\": 30\n}")
    .build()?;

// HTML шаблон
let html_template = CodeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("template")
        .name("HTML Template")
        .description("HTML template with variables")
        .hint("Use {{variable}} for template variables")
        .build()?)
    .ui_options(CodeUiOptions {
        language: CodeLanguage::HTML,
        height: 12,
        available_variables: vec![
            "user".into(),
            "data".into(),
        ],
    })
    .build()?;
```

**Опции UI:**
- `language` - язык программирования
- `height` - высота редактора в строках
- `available_variables` - переменные для автодополнения

---

### 10. ExpressionParameter (Удален)

**Примечание:** Этот тип был удален, так как универсальная поддержка выражений делает его избыточным. Используйте соответствующие типизированные параметры с переключателем выражений, предоставляемым ядром платформы.

---

### 11. ResourceParameter

**Назначение:** Универсальный SDK для динамической загрузки ресурсов из внешних API.

**Когда использовать:**
- Slack каналы, пользователи
- Таблицы баз данных
- Папки Google Drive
- GitHub репозитории
- Файловые системы
- Любой внешний источник данных

**Хранимые данные:** `ParameterValue::String(String)` (ID ресурса) или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "String",
  "value": "C1234567890"  // Slack channel ID
}

// Выражение
{
  "type": "Expression",
  "value": "{{$json.targetChannel}}"
}
```

**Примеры выражений:**
- Статическое: `"C1234567890"` → фиксированный ID канала Slack
- Выражение: `"{{$json.targetChannel}}"` → динамический выбор канала

#### Универсальная архитектура

```rust
pub struct ResourceParameter {
    pub metadata: ParameterMetadata,
    
    /// Универсальный загрузчик ресурсов
    pub loader: ResourceLoader,
    
    /// Конфигурация кеша
    pub cache_config: CacheConfig,
    
    /// Конфигурация UI
    pub ui_config: ResourceUIConfig,
    
    /// Обработка ошибок
    pub error_handling: ErrorHandling,
}

/// Универсальный загрузчик - строительный блок для любого ресурса
pub struct ResourceLoader {
    /// Функция загрузки данных
    pub load_fn: LoadFunction,
    
    /// Зависимости от других параметров
    pub dependencies: Vec<String>,
    
    /// Стратегия загрузки
    pub loading_strategy: LoadingStrategy,
    
    /// Валидация загруженных данных
    pub validation: Option<ValidationFunction>,
    
    /// Преобразование данных перед отображением
    pub transform: Option<TransformFunction>,
}
```

#### Примеры использования

**Простой HTTP ресурс:**
```rust
// Пользователи из API
let users_param = ResourceParameter::http_resource("https://api.example.com/users")
    .metadata(ParameterMetadata::required("user_id", "User")?)
    .cache(Duration::minutes(10))
    .transform(|mut items| {
        // Сортировка по имени
        items.sort_by(|a, b| a.label.cmp(&b.label));
        items
    })
    .build()?;
```

**Ресурс с зависимостями (Slack каналы):**
```rust
let channels_param = ResourceParameter::dependent_resource()
    .metadata(ParameterMetadata::required("channel_id", "Slack Channel")?)
    .depends_on(vec!["workspace_id", "credential"])
    .load_with(|ctx| Box::pin(async move {
        let workspace_id = ctx.dependencies.get("workspace_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LoadError::MissingDependency("workspace_id".into()))?;
            
        let credential = ctx.credentials.get("slack_oauth2")
            .ok_or_else(|| LoadError::MissingCredential("slack_oauth2".into()))?;
        
        let response = ctx.http_client
            .get("https://slack.com/api/conversations.list")
            .header("Authorization", format!("Bearer {}", credential.token))
            .query(&[("types", "public_channel,private_channel")])
            .send()
            .await?;
            
        let data: serde_json::Value = response.json().await?;
        let channels = data["channels"].as_array()
            .ok_or_else(|| LoadError::InvalidResponse("Missing channels array".into()))?;
        
        let mut items = Vec::new();
        for channel in channels {
            if let Some(id) = channel["id"].as_str() {
                let name = channel["name"].as_str().unwrap_or("Unknown");
                let is_private = channel["is_private"].as_bool().unwrap_or(false);
                
                items.push(ResourceItem {
                    id: id.to_string(),
                    label: format!("#{}", name),
                    description: channel["purpose"]["value"].as_str().map(String::from),
                    icon: Some(ResourceIcon::Icon(
                        if is_private { "lock" } else { "hash" }.to_string()
                    )),
                    metadata: {
                        let mut map = serde_json::Map::new();
                        map.insert("is_private".into(), json!(is_private));
                        map.insert("name".into(), json!(name));
                        map
                    },
                    enabled: true,
                    group: Some(if is_private { "Private" } else { "Public" }.to_string()),
                    sort_key: Some(name.to_lowercase()),
                });
            }
        }
        
        Ok(items)
    }))
    .cache_key(|ctx| {
        format!("slack_channels_{}", 
            ctx.dependencies.get("workspace_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
        )
    })
    .cache(Duration::minutes(10))
    .loading_strategy(LoadingStrategy::OnDemand)
    .build()?;
```

**Таблицы баз данных:**
```rust
let tables_param = ResourceParameter::dependent_resource()
    .metadata(ParameterMetadata::required("table_name", "Database Table")?)
    .depends_on(vec!["connection"])
    .load_with(|ctx| Box::pin(async move {
        let connection = ctx.credentials.get("database")
            .ok_or_else(|| LoadError::MissingCredential("database".into()))?;
        
        // Подключение к базе данных через строку подключения
        let db_url = format!("postgresql://{}:{}@{}:{}/{}",
            connection.username,
            connection.password,
            connection.host,
            connection.port,
            connection.database
        );
        
        let response = ctx.http_client
            .post("/api/database/query")
            .json(&json!({
                "connection": db_url,
                "query": "SELECT schemaname, tablename, tableowner FROM pg_tables ORDER BY schemaname, tablename"
            }))
            .send()
            .await?;
            
        let rows: Vec<serde_json::Value> = response.json().await?;
        
        let mut items = Vec::new();
        for row in rows {
            let schema = row["schemaname"].as_str().unwrap_or("public");
            let table = row["tablename"].as_str().unwrap_or("unknown");
            let owner = row["tableowner"].as_str().unwrap_or("unknown");
            
            items.push(ResourceItem {
                id: format!("{}.{}", schema, table),
                label: table.to_string(),
                description: Some(format!("Owner: {}", owner)),
                icon: Some(ResourceIcon::Icon("table".to_string())),
                metadata: {
                    let mut map = serde_json::Map::new();
                    map.insert("schema".into(), json!(schema));
                    map.insert("owner".into(), json!(owner));
                    map
                },
                enabled: true,
                group: Some(schema.to_string()),
                sort_key: Some(format!("{}_{}", schema, table)),
            });
        }
        
        Ok(items)
    }))
    .validate(|items| {
        if items.is_empty() {
            Err("No tables found. Check your database connection.".to_string())
        } else {
            Ok(())
        }
    })
    .cache(Duration::hours(1))
    .build()?;
```

**Ключевые преимущества:**
- **Полная гибкость** - разработчики могут создать ЛЮБОЙ ресурс
- **Простота для простых случаев** - простой HTTP ресурс в одну строку
- **Мощь для сложных случаев** - полный контроль над всем процессом
- **Композируемость** - можно создавать вспомогательные функции
- **Нет привязки к поставщику** - нет жестко закодированных типов ресурсов

---

### 12. FileParameter

**Назначение:** Загрузка и выбор файлов.

**Когда использовать:**
- Загрузка документов
- Аватары пользователей
- CSV файлы для обработки

**Хранимые данные:** `ParameterValue::File(FileData)` или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение (загруженный файл)
{
  "type": "File",
  "value": {
    "name": "data.csv",
    "size": 1024,
    "mime_type": "text/csv",
    "url": "https://storage.example.com/files/abc123.csv",
    "upload_date": "2024-01-15T10:30:00Z"
  }
}

// Выражение
{
  "type": "Expression",
  "value": "{{$json.attachmentUrl}}"
}
```

**Примеры выражений:**
- Статическое: Файл загружен напрямую
- Выражение: `"{{$json.attachmentUrl}}"` → динамическая ссылка на файл

**Примеры кода:**
```rust
// CSV файл
let csv_file = FileParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("csv_file")
        .name("CSV File")
        .description("Upload CSV file with data")
        .required(true)
        .build()?)
    .ui_options(FileUiOptions {
        accept: vec!["text/csv".into(), ".csv".into()],
        max_size: Some(10 * 1024 * 1024), // 10MB
        multiple: false,
        preview: false,
    })
    .build()?;

// Изображение аватара
let avatar = FileParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("avatar")
        .name("Profile Picture")
        .description("Upload your profile picture")
        .hint("Max size: 2MB, formats: JPG, PNG")
        .build()?)
    .ui_options(FileUiOptions {
        accept: vec!["image/jpeg".into(), "image/png".into()],
        max_size: Some(2 * 1024 * 1024), // 2MB
        multiple: false,
        preview: true, // Показать превью изображения
    })
    .build()?;

// Множественная загрузка документов
let documents = FileParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("documents")
        .name("Documents")
        .description("Upload multiple documents")
        .build()?)
    .ui_options(FileUiOptions {
        accept: vec![
            "application/pdf".into(),
            "application/msword".into(),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document".into(),
        ],
        max_size: Some(50 * 1024 * 1024), // 50MB total
        multiple: true, // Разрешить множественную загрузку
        preview: false,
    })
    .build()?;

// Любой файл
let any_file = FileParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("file")
        .name("File")
        .description("Upload any file")
        .build()?)
    .ui_options(FileUiOptions {
        accept: vec![], // Принимать любые файлы
        max_size: Some(100 * 1024 * 1024), // 100MB
        multiple: false,
        preview: false,
    })
    .build()?;
```

**Опции UI:**
- `accept` - разрешенные типы файлов
- `max_size` - максимальный размер файла
- `multiple` - множественная загрузка
- `preview` - показывать превью

---

### 13. ColorParameter

**Назначение:** Выбор цвета с поддержкой различных форматов.

**Когда использовать:**
- Цвета сообщений Slack
- Темы интерфейса
- Настройки внешнего вида

**Хранимые данные:** `ParameterValue::String(String)` (hex/rgb) или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "String",
  "value": "#36a64f"
}

// Выражение
{
  "type": "Expression",
  "value": "{{$json.status === 'error' ? '#ff0000' : '#36a64f'}}"
}
```

**Примеры выражений:**
- Статическое: `"#36a64f"` → фиксированный зеленый цвет
- Выражение: `"{{$json.status === 'error' ? '#ff0000' : '#36a64f'}}"` → условная раскраска

**Примеры кода:**
```rust
// Цвет сообщения
let message_color = ColorParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("color")
        .name("Message Color")
        .description("Color for the message attachment")
        .build()?)
    .ui_options(ColorUiOptions {
        format: ColorFormat::Hex,
        palette: vec![
            "#36a64f".into(), // зеленый (успех)
            "#ff0000".into(), // красный (ошибка)
            "#ffaa00".into(), // оранжевый (предупреждение)
            "#439fe0".into(), // синий (информация)
        ],
        alpha: false,
    })
    .build()?;

// Цвет темы с альфа-каналом
let theme_color = ColorParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("theme_color")
        .name("Theme Color")
        .description("Primary theme color")
        .build()?)
    .ui_options(ColorUiOptions {
        format: ColorFormat::RGBA,
        palette: vec![],
        alpha: true, // Включить альфа-канал
    })
    .build()?;

// Цвет фона
let background_color = ColorParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("bg_color")
        .name("Background Color")
        .placeholder("#ffffff")
        .build()?)
    .default("#ffffff")
    .ui_options(ColorUiOptions {
        format: ColorFormat::Hex,
        palette: vec![
            "#ffffff".into(),
            "#f5f5f5".into(),
            "#e0e0e0".into(),
            "#000000".into(),
        ],
        alpha: false,
    })
    .build()?;
```

**Опции UI:**
- `format` - формат цвета (Hex, RGB, RGBA, HSL)
- `palette` - предопределенная палитра цветов
- `alpha` - поддержка альфа-канала

---

### 14. HiddenParameter

**Назначение:** Скрытые параметры для внутренних нужд.

**Когда использовать:**
- Внутренние идентификаторы
- Состояние рабочего процесса
- Системные параметры

**Хранимые данные:** Любой тип `ParameterValue` или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "String",
  "value": "workflow_123"
}

// Выражение
{
  "type": "Expression",
  "value": "{{$workflow.instanceId}}"
}
```

**Примечание:** Скрытые параметры поддерживают выражения для динамических внутренних значений вроде `"{{$workflow.instanceId}}"`, но не видны в UI.

**Примеры кода:**
```rust
// Внутренний ID
let internal_id = HiddenParameter::builder()
    .metadata(ParameterMetadata::simple("internal_id", "Internal ID")?)
    .value(Some(ParameterValue::String("workflow_123".into())))
    .build()?;

// Состояние выполнения
let execution_state = HiddenParameter::builder()
    .metadata(ParameterMetadata::simple("state", "Execution State")?)
    .value(Some(ParameterValue::String("initialized".into())))
    .build()?;

// Метаданные системы
let system_metadata = HiddenParameter::builder()
    .metadata(ParameterMetadata::simple("metadata", "System Metadata")?)
    .value(Some(ParameterValue::Object({
        let mut map = HashMap::new();
        map.insert("version".to_string(), ParameterValue::String("1.0".into()));
        map.insert("created_at".to_string(), ParameterValue::DateTime(Utc::now()));
        map
    })))
    .build()?;

// Динамический ID экземпляра
let instance_id = HiddenParameter::builder()
    .metadata(ParameterMetadata::simple("instance_id", "Instance ID")?)
    .value(Some(ParameterValue::Expression("{{$workflow.instanceId}}".into())))
    .build()?;
```

---

### 15. NoticeParameter

**Назначение:** Отображение информационных сообщений.

**Когда использовать:**
- Предупреждения о лимитах API
- Инструкции по настройке
- Информация о статусе

**Хранимые данные:** Нет хранимого значения (параметр только для отображения)

**Примечание:** Параметры уведомлений предназначены для сообщений UI и не хранят данные. Они могут использовать выражения в своем содержимом для динамических сообщений.

**Примеры кода:**
```rust
// Предупреждение о лимите
let warning = NoticeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("api_warning")
        .name("API Rate Limit Warning")
        .description("You are approaching your API rate limit. Consider upgrading your plan.")
        .build()?)
    .notice_type(NoticeType::Warning)
    .ui_options(NoticeUiOptions {
        dismissible: true,
        show_icon: true,
        markdown: true,
    })
    .build()?;

// Информационное сообщение
let info = NoticeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("setup_info")
        .name("Setup Instructions")
        .description("## How to get your API key\n\n1. Go to Settings\n2. Click on API Keys\n3. Generate new key")
        .build()?)
    .notice_type(NoticeType::Info)
    .ui_options(NoticeUiOptions {
        dismissible: false,
        show_icon: true,
        markdown: true,
    })
    .build()?;

// Сообщение об ошибке
let error = NoticeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("error_notice")
        .name("Configuration Error")
        .description("Invalid credentials. Please check your API key.")
        .build()?)
    .notice_type(NoticeType::Error)
    .ui_options(NoticeUiOptions {
        dismissible: false,
        show_icon: true,
        markdown: false,
    })
    .build()?;

// Сообщение об успехе
let success = NoticeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("success_notice")
        .name("Success")
        .description("Connection established successfully!")
        .build()?)
    .notice_type(NoticeType::Success)
    .ui_options(NoticeUiOptions {
        dismissible: true,
        show_icon: true,
        markdown: false,
    })
    .build()?;
```

**Типы уведомлений:**
- `Info` - информационное сообщение
- `Warning` - предупреждение
- `Error` - ошибка
- `Success` - успех

**Опции UI:**
- `dismissible` - можно закрыть
- `show_icon` - показывать иконку
- `markdown` - поддержка markdown

---

### 16. CheckboxParameter

**Назначение:** Checkbox для булевых значений с особым UI представлением.

**Когда использовать:**
- Принятие условий
- Множественные независимые опции
- Когда нужен checkbox UI вместо toggle

**Хранимые данные:** `ParameterValue::Boolean(bool)` или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "Boolean",
  "value": true
}

// Выражение
{
  "type": "Expression",
  "value": "{{$json.user.hasConsent}}"
}
```

**Примеры выражений:**
- Статическое: `true` → условия приняты
- Выражение: `"{{$json.user.hasConsent}}"` → динамическая проверка согласия

**Примеры кода:**
```rust
// Принятие условий
let accept_terms = CheckboxParameter::builder()
    .metadata(ParameterMetadata::required("accept_terms", "Accept Terms and Conditions")?)
    .ui_options(CheckboxUiOptions {
        label_position: LabelPosition::Right,
        show_description: true,
        indeterminate: false,
    })
    .build()?;

// Email уведомления
let email_notifications = CheckboxParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("email_notifications")
        .name("Email Notifications")
        .description("Receive notifications via email")
        .build()?)
    .default(true)
    .build()?;

// Согласие на обработку данных
let data_consent = CheckboxParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("data_consent")
        .name("Data Processing Consent")
        .description("I agree to the processing of my personal data")
        .required(true)
        .build()?)
    .ui_options(CheckboxUiOptions {
        label_position: LabelPosition::Right,
        show_description: true,
        indeterminate: false,
    })
    .build()?;

// Подписка на новости
let newsletter = CheckboxParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("newsletter")
        .name("Subscribe to Newsletter")
        .description("Get weekly updates and tips")
        .build()?)
    .default(false)
    .ui_options(CheckboxUiOptions {
        label_position: LabelPosition::Right,
        show_description: true,
        indeterminate: false,
    })
    .build()?;
```

**Опции UI:**
- `label_position` - позиция текста относительно checkbox
- `show_description` - показывать описание
- `indeterminate` - поддержка третьего состояния

---

### 17. DateParameter

**Назначение:** Ввод только даты без времени.

**Когда использовать:**
- Дата рождения
- Дедлайны
- Даты событий без привязки ко времени

**Хранимые данные:** `ParameterValue::DateTime(DateTime<Utc>)` (время установлено в 00:00:00) или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "DateTime",
  "value": "2024-12-25T00:00:00Z"
}

// Выражение
{
  "type": "Expression",
  "value": "{{$json.deadline}}"
}
```

**Примеры выражений:**
- Статическое: `"2024-12-25"` → дата Рождества
- Выражение: `"{{$json.deadline}}"` → динамический дедлайн

**Примеры кода:**
```rust
// Дата рождения
let birth_date = DateParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("birth_date")
        .name("Date of Birth")
        .required(true)
        .build()?)
    .ui_options(DateUiOptions {
        format: Some("YYYY-MM-DD".into()),
        min_date: Some(NaiveDate::from_ymd_opt(1900, 1, 1).unwrap()),
        max_date: Some(today()),
        show_week_numbers: false,
    })
    .build()?;

// Дедлайн проекта
let deadline = DateParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("deadline")
        .name("Project Deadline")
        .description("Final date for project completion")
        .build()?)
    .ui_options(DateUiOptions {
        format: None, // Локальный формат
        min_date: Some(today()),
        max_date: None,
        show_week_numbers: true,
    })
    .build()?;

// Дата начала события
let event_date = DateParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("event_date")
        .name("Event Date")
        .placeholder("Select event date")
        .build()?)
    .ui_options(DateUiOptions {
        format: Some("DD/MM/YYYY".into()),
        min_date: Some(tomorrow()),
        max_date: Some(next_year()),
        show_week_numbers: false,
    })
    .build()?;

// Дата истечения срока
let expiry_date = DateParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("expiry_date")
        .name("Expiry Date")
        .description("When this item expires")
        .hint("Must be within the next 5 years")
        .build()?)
    .ui_options(DateUiOptions {
        format: None,
        min_date: Some(today()),
        max_date: Some(today().add(Duration::days(365 * 5))),
        show_week_numbers: false,
    })
    .build()?;
```

**Опции UI:**
- `format` - формат отображения даты
- `min_date/max_date` - ограничения даты
- `show_week_numbers` - показывать номера недель

---

### 18. TimeParameter

**Назначение:** Ввод только времени без даты.

**Когда использовать:**
- Время встречи
- Рабочие часы
- Расписания

**Хранимые данные:** `ParameterValue::String(String)` (формат времени как "09:30:00") или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "String",
  "value": "14:30:00"
}

// Выражение
{
  "type": "Expression",
  "value": "{{$json.meetingTime}}"
}
```

**Примеры выражений:**
- Статическое: `"14:30:00"` → 2:30 PM
- Выражение: `"{{$json.meetingTime}}"` → динамическое планирование времени

**Примеры кода:**
```rust
// Время встречи
let meeting_time = TimeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("meeting_time")
        .name("Meeting Time")
        .description("Schedule meeting time")
        .build()?)
    .ui_options(TimeUiOptions {
        format: TimeFormat::Hour24,
        step: Some(Duration::minutes(15)), // Шаг 15 минут
        show_seconds: false,
    })
    .build()?;

// Время с секундами
let precise_time = TimeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("precise_time")
        .name("Exact Time")
        .description("Time with seconds precision")
        .build()?)
    .ui_options(TimeUiOptions {
        format: TimeFormat::Hour12,
        step: Some(Duration::seconds(1)),
        show_seconds: true,
    })
    .build()?;

// Рабочие часы - начало
let work_start = TimeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("work_start")
        .name("Work Start Time")
        .placeholder("09:00")
        .build()?)
    .default("09:00:00")
    .ui_options(TimeUiOptions {
        format: TimeFormat::Hour24,
        step: Some(Duration::minutes(30)),
        show_seconds: false,
    })
    .build()?;

// Время ежедневного запуска
let daily_run = TimeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("daily_run")
        .name("Daily Run Time")
        .description("Time to run daily tasks")
        .hint("Tasks will run at this time every day")
        .build()?)
    .ui_options(TimeUiOptions {
        format: TimeFormat::Hour24,
        step: Some(Duration::minutes(5)),
        show_seconds: false,
    })
    .build()?;
```

**Опции UI:**
- `format` - формат времени (12/24 часа)
- `step` - шаг изменения времени
- `show_seconds` - показывать секунды

---

## 🗂️ Контейнерные параметры

### 19. GroupParameter

**Назначение:** Визуальная группировка связанных параметров.

**Когда использовать:**
- Секции настроек
- Логические группы параметров
- Сворачиваемые панели

**Хранимые данные:** `ParameterValue::Object(HashMap<String, ParameterValue>)` содержащий все значения дочерних параметров

**Примеры в базе данных:**
```json
{
  "type": "Object",
  "value": {
    "host": {"type": "String", "value": "localhost"},
    "port": {"type": "Number", "value": 5432},
    "username": {"type": "String", "value": "admin"},
    "password": {"type": "String", "value": "encrypted:...", "encrypted": true}
  }
}
```

**Примечание:** Параметры группы агрегируют значения своих дочерних элементов. Выражения могут использоваться в отдельных дочерних параметрах.

**Примеры кода:**
```rust
// Группа настроек базы данных
let db_group = GroupParameter::builder()
    .metadata(GroupMetadata::builder()
        .key("database")
        .name("Database Settings")
        .description("Configure database connection")
        .build()?)
    .parameters(vec![
        Parameter::Text(TextParameter::builder()
            .metadata(ParameterMetadata::required("host", "Host")?)
            .build()?),
        Parameter::Number(NumberParameter::builder()
            .metadata(ParameterMetadata::required("port", "Port")?)
            .default(5432.0)
            .build()?),
        Parameter::Text(TextParameter::builder()
            .metadata(ParameterMetadata::required("username", "Username")?)
            .build()?),
        Parameter::Secret(SecretParameter::builder()
            .metadata(ParameterMetadata::required("password", "Password")?)
            .build()?),
    ])
    .ui_options(GroupUiOptions {
        collapsible: true,
        default_expanded: false,
        layout: GroupLayout::Vertical,
    })
    .build()?;

// Группа настроек API
let api_settings = GroupParameter::builder()
    .metadata(GroupMetadata::builder()
        .key("api_settings")
        .name("API Configuration")
        .description("Configure API behavior")
        .build()?)
    .parameters(vec![
        Parameter::Text(TextParameter::builder()
            .metadata(ParameterMetadata::required("base_url", "Base URL")?)
            .ui_options(TextUiOptions {
                input_type: TextInputType::URL,
                multiline: false,
            })
            .build()?),
        Parameter::Number(NumberParameter::builder()
            .metadata(ParameterMetadata::builder()
                .key("timeout")
                .name("Timeout")
                .description("Request timeout in seconds")
                .build()?)
            .default(30.0)
            .ui_options(NumberUiOptions {
                format: NumberFormat::Integer,
                min: Some(1.0),
                max: Some(300.0),
                step: Some(1.0),
                unit: Some("seconds".into()),
            })
            .build()?),
        Parameter::Boolean(BooleanParameter::builder()
            .metadata(ParameterMetadata::builder()
                .key("retry")
                .name("Enable Retry")
                .description("Retry failed requests")
                .build()?)
            .default(true)
            .build()?),
    ])
    .ui_options(GroupUiOptions {
        collapsible: true,
        default_expanded: true,
        layout: GroupLayout::Vertical,
    })
    .build()?;

// Группа расширенных настроек
let advanced_group = GroupParameter::builder()
    .metadata(GroupMetadata::builder()
        .key("advanced")
        .name("Advanced Settings")
        .description("Advanced configuration options")
        .hint("Modify only if you know what you're doing")
        .build()?)
    .parameters(vec![
        Parameter::Boolean(debug_mode),
        Parameter::Number(max_retries),
        Parameter::Select(log_level),
        Parameter::Code(custom_headers),
    ])
    .ui_options(GroupUiOptions {
        collapsible: true,
        default_expanded: false,
        layout: GroupLayout::Vertical,
    })
    .build()?;
```

**Опции UI:**
- `collapsible` - можно свернуть/развернуть
- `default_expanded` - развернуто по умолчанию
- `layout` - макет (Vertical, Horizontal, Grid)

---

### 20. ObjectParameter

**Назначение:** Контейнер структурированных данных с фиксированными именованными полями, которые образуют единую логическую единицу.

**Когда использовать:**
- HTTP заголовки (всегда `name` + `value`)
- Подключения к базе данных (всегда `host` + `port` + `username` + `password`)
- API эндпоинты (всегда `method` + `url` + `headers`)
- Координаты (всегда `x` + `y` + опциональный `z`)
- Сложные конфигурации с взаимозависимыми полями

**Когда НЕ использовать:**
- Разные типы данных (используйте ModeParameter)
- Только UI группировка (используйте GroupParameter)
- Динамическая структура полей (используйте ResourceParameter с объектным ответом)

**Хранимые данные:** `ParameterValue::Object(HashMap<String, ParameterValue>)` или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "Object",
  "value": {
    "name": {"type": "String", "value": "Content-Type"},
    "value": {"type": "String", "value": "application/json"}
  }
}

// Выражение
{
  "type": "Expression",
  "value": "{{JSON.stringify({Authorization: 'Bearer ' + $json.token})}}"
}
```

**Примеры выражений:**
- Статическое: `{"name": "Content-Type", "value": "application/json"}`
- Выражение: `"{{JSON.stringify({Authorization: 'Bearer ' + $json.token})}}"` → динамический заголовок

#### Архитектурные принципы

**🎯 Основной принцип: Фиксированные именованные поля**
- Каждое поле имеет конкретное имя и тип
- Поля определяются во время создания, а не во время выполнения
- Все поля вместе образуют единую логическую единицу
- Перекрестная валидация и зависимости полей

**🔧 Ключевые характеристики:**
- **Семантическая связность** - поля значимо связаны
- **Фиксированная структура** - нельзя добавлять/удалять поля динамически
- **Типовая безопасность** - каждое поле строго типизировано
- **Перекрестная валидация** - поля могут валидироваться вместе

#### Примеры использования

**Простой HTTP заголовок:**
```rust
let http_header = ObjectParameter::builder()
    .metadata(ParameterMetadata::simple("header", "HTTP Header")?)
    .add_field("name", TextParameter::builder()
        .metadata(ParameterMetadata::required("name", "Header Name")?)
        .build()?)
    .add_field("value", TextParameter::builder()
        .metadata(ParameterMetadata::required("value", "Header Value")?)
        .build()?)
    .layout(ObjectLayout::Horizontal)
    .validate(|fields| {
        let name = fields.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let value = fields.get("value").and_then(|v| v.as_str()).unwrap_or("");
        
        if name.is_empty() {
            return Err("Header name is required".to_string());
        }
        
        if name.contains(" ") {
            return Err("Header name cannot contain spaces".to_string());
        }
        
        if value.is_empty() {
            return Err("Header value is required".to_string());
        }
        
        Ok(())
    })
    .build()?;
```

**Подключение к базе данных с группами полей:**
```rust
let db_connection = ObjectParameter::builder()
    .metadata(ParameterMetadata::required("connection", "Database Connection")?)
    .add_field("host", TextParameter::builder()
        .metadata(ParameterMetadata::builder()
            .key("host")
            .name("Host")
            .required(true)
            .placeholder("localhost")
            .build()?)
        .build()?)
    .add_field("port", NumberParameter::builder()
        .metadata(ParameterMetadata::builder()
            .key("port")
            .name("Port")
            .required(true)
            .build()?)
        .ui_options(NumberUiOptions {
            format: NumberFormat::Integer,
            min: Some(1.0),
            max: Some(65535.0),
            step: Some(1.0),
            unit: None,
        })
        .default(5432.0)
        .build()?)
    .add_field("database", TextParameter::builder()
        .metadata(ParameterMetadata::required("database", "Database Name")?)
        .build()?)
    .add_field("username", TextParameter::builder()
        .metadata(ParameterMetadata::required("username", "Username")?)
        .build()?)
    .add_field("password", SecretParameter::builder()
        .metadata(ParameterMetadata::required("password", "Password")?)
        .build()?)
    .layout(ObjectLayout::Grid { columns: 2 })
    .field_group("Connection", vec!["host", "port", "database"])
    .field_group("Authentication", vec!["username", "password"])
    .validate(|fields| {
        let host = fields.get("host").and_then(|v| v.as_str()).unwrap_or("");
        let port = fields.get("port").and_then(|v| v.as_f64()).unwrap_or(0.0);
        
        if host.is_empty() {
            return Err("Host is required".to_string());
        }
        
        if port < 1.0 || port > 65535.0 {
            return Err("Port must be between 1 and 65535".to_string());
        }
        
        // Перекрестная валидация
        if host == "localhost" && port != 5432.0 {
            return Err("For localhost, please use default port 5432".to_string());
        }
        
        Ok(())
    })
    .build()?;
```

**API эндпоинт с условными полями:**
```rust
let api_endpoint = ObjectParameter::builder()
    .metadata(ParameterMetadata::required("endpoint", "API Endpoint")?)
    .add_field("method", SelectParameter::builder()
        .metadata(ParameterMetadata::required("method", "HTTP Method")?)
        .options(vec![
            SelectOption::new("GET", "GET"),
            SelectOption::new("POST", "POST"),
            SelectOption::new("PUT", "PUT"),
            SelectOption::new("DELETE", "DELETE"),
        ])
        .build()?)
    .add_field("url", TextParameter::builder()
        .metadata(ParameterMetadata::required("url", "URL")?)
        .ui_options(TextUiOptions {
            input_type: TextInputType::URL,
            multiline: false,
        })
        .build()?)
    .add_field("headers", ListParameter::new(
        ObjectParameter::builder()
            .metadata(ParameterMetadata::simple("header", "Header")?)
            .add_field("name", text_parameter!("name", "Name"))
            .add_field("value", text_parameter!("value", "Value"))
            .layout(ObjectLayout::Horizontal)
            .build()?
    ).build()?)
    .add_field("body", TextParameter::builder()
        .metadata(ParameterMetadata::builder()
            .key("body")
            .name("Request Body")
            .required(false)
            .build()?)
        .ui_options(TextUiOptions {
            input_type: TextInputType::Text,
            multiline: true,
            rows: Some(6),
        })
        .display(ParameterDisplay::builder()
            .show_when("method", ParameterCondition::Or(vec![
                ParameterCondition::Eq(json!("POST")),
                ParameterCondition::Eq(json!("PUT")),
                ParameterCondition::Eq(json!("PATCH")),
            ]))
            .build())
        .build()?)
    .layout(ObjectLayout::Vertical)
    .validate(|fields| {
        let method = fields.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let url = fields.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let body = fields.get("body").and_then(|v| v.as_str()).unwrap_or("");
        
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err("URL must start with http:// or https://".to_string());
        }
        
        if ["POST", "PUT", "PATCH"].contains(&method) && body.is_empty() {
            return Err(format!("{} requests typically require a body", method));
        }
        
        Ok(())
    })
    .build()?;
```

**Сложная кнопка Telegram:**
```rust
let telegram_button = ObjectParameter::builder()
    .metadata(ParameterMetadata::required("button", "Telegram Button")?)
    .add_field("text", TextParameter::builder()
        .metadata(ParameterMetadata::required("text", "Button Text")?)
        .build()?)
    .add_field("type", SelectParameter::builder()
        .metadata(ParameterMetadata::required("type", "Button Type")?)
        .options(vec![
            SelectOption::new("url", "URL"),
            SelectOption::new("callback_data", "Callback Data"),
            SelectOption::new("switch_inline_query", "Switch Inline Query"),
            SelectOption::new("web_app", "Web App"),
        ])
        .build()?)
    .add_field("url", TextParameter::builder()
        .metadata(ParameterMetadata::builder()
            .key("url")
            .name("URL")
            .required(false)
            .build()?)
        .ui_options(TextUiOptions {
            input_type: TextInputType::URL,
            multiline: false,
        })
        .display(ParameterDisplay::builder()
            .show_when("type", ParameterCondition::Eq(json!("url")))
            .build())
        .build()?)
    .add_field("callback_data", TextParameter::builder()
        .metadata(ParameterMetadata::builder()
            .key("callback_data")
            .name("Callback Data")
            .required(false)
            .build()?)
        .display(ParameterDisplay::builder()
            .show_when("type", ParameterCondition::Eq(json!("callback_data")))
            .build())
        .build()?)
    .add_field("web_app", ObjectParameter::builder()
        .metadata(ParameterMetadata::builder()
            .key("web_app")
            .name("Web App")
            .required(false)
            .build()?)
        .add_field("url", TextParameter::builder()
            .metadata(ParameterMetadata::required("url", "Web App URL")?)
            .ui_options(TextUiOptions {
                input_type: TextInputType::URL,
                multiline: false,
            })
            .build()?)
        .display(ParameterDisplay::builder()
            .show_when("type", ParameterCondition::Eq(json!("web_app")))
            .build())
        .build()?)
    .layout(ObjectLayout::Vertical)
    .validate(|fields| {
        let button_type = fields.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let text = fields.get("text").and_then(|v| v.as_str()).unwrap_or("");
        
        if text.is_empty() {
            return Err("Button text is required".to_string());
        }
        
        if text.len() > 64 {
            return Err("Button text cannot exceed 64 characters".to_string());
        }
        
        match button_type {
            "url" => {
                let url = fields.get("url").and_then(|v| v.as_str()).unwrap_or("");
                if url.is_empty() {
                    return Err("URL is required for URL buttons".to_string());
                }
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    return Err("URL must start with http:// or https://".to_string());
                }
            },
            "callback_data" => {
                let callback_data = fields.get("callback_data").and_then(|v| v.as_str()).unwrap_or("");
                if callback_data.is_empty() {
                    return Err("Callback data is required for callback buttons".to_string());
                }
                if callback_data.len() > 64 {
                    return Err("Callback data cannot exceed 64 characters".to_string());
                }
            },
            "web_app" => {
                let web_app = fields.get("web_app").and_then(|v| v.as_object());
                if let Some(web_app_obj) = web_app {
                    let url = web_app_obj.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    if url.is_empty() {
                        return Err("Web App URL is required".to_string());
                    }
                    if !url.starts_with("https://") {
                        return Err("Web App URL must use HTTPS".to_string());
                    }
                } else {
                    return Err("Web App configuration is required".to_string());
                }
            },
            _ => {}
        }
        
        Ok(())
    })
    .build()?;
```

#### Ключевые преимущества

**🎯 Семантическая связность:**
- Все поля образуют логическую единицу
- Перекрестная валидация и зависимости полей
- Четкая структура данных со смыслом

**🔧 Типовая безопасность:**
- Каждое поле строго типизировано
- Валидация на уровне полей и объекта
- Четкие сообщения об ошибках при сбоях валидации

**🎨 UI гибкость:**
- Множественные опции макета (Vertical, Horizontal, Grid)
- Группировка полей для лучшей организации
- Условное отображение полей на основе других полей

**📊 Целостность данных:**
- Фиксированная структура предотвращает ошибки времени выполнения
- Перекрестная валидация обеспечивает согласованность данных
- Четкое разделение между структурой и значениями

#### Шаблоны проектирования

**✅ Хорошее использование:**
```rust
// HTTP заголовок - всегда name + value
ObjectParameter::builder()
    .add_field("name", text_parameter!("name", "Name"))
    .add_field("value", text_parameter!("value", "Value"))

// Подключение к базе данных - связанная конфигурация
ObjectParameter::builder()
    .add_field("host", text_parameter!("host", "Host"))
    .add_field("port", number_parameter!("port", "Port"))
    .add_field("database", text_parameter!("database", "Database"))

// Координата - связанные данные позиции
ObjectParameter::builder()
    .add_field("x", number_parameter!("x", "X"))
    .add_field("y", number_parameter!("y", "Y"))
    .add_field("z", number_parameter!("z", "Z"))
```

**❌ Анти-паттерны:**
```rust
// Не используйте для несвязанных полей
ObjectParameter::builder()
    .add_field("username", text_parameter!("username", "Username"))
    .add_field("color", color_parameter!("color", "Theme Color"))  // Не связано!
    .add_field("timeout", number_parameter!("timeout", "Timeout"))  // Не связано!

// Не используйте только для UI группировки (используйте GroupParameter)
ObjectParameter::builder()
    .add_field("setting1", text_parameter!("setting1", "Setting 1"))
    .add_field("setting2", text_parameter!("setting2", "Setting 2"))
    // Если это просто группировка для UI, используйте GroupParameter

// Не используйте для разных типов данных (используйте ModeParameter)
ObjectParameter::builder()
    .add_field("text_mode", text_parameter!("text", "Text"))
    .add_field("number_mode", number_parameter!("number", "Number"))
    // Это должен быть ModeParameter с разными режимами
```

#### Общие паттерны

**HTTP заголовки в списках:**
```rust
let headers_list = ListParameter::new(
    ObjectParameter::builder()
        .metadata(ParameterMetadata::simple("header", "Header")?)
        .add_field("name", text_parameter!("name", "Name"))
        .add_field("value", text_parameter!("value", "Value"))
        .layout(ObjectLayout::Horizontal)
        .build()?
).build()?;
```

**Вложенные объекты:**
```rust
let api_config = ObjectParameter::builder()
    .add_field("endpoint", ObjectParameter::builder()
        .add_field("url", text_parameter!("url", "URL"))
        .add_field("method", select_parameter!("method", "Method", methods))
        .build()?)
    .add_field("auth", ObjectParameter::builder()
        .add_field("type", select_parameter!("type", "Auth Type", auth_types))
        .add_field("token", secret_parameter!("token", "Token"))
        .build()?)
    .build()?;
```

Этот подход гарантирует, что ObjectParameter используется по назначению: представление связных структур данных с осмысленными отношениями между полями.

---

### 21. ListParameter

**Назначение:** Динамические массивы независимых элементов параметров.

**Когда использовать:**
- Список HTTP заголовков
- Множественные входные значения
- Строки встроенной клавиатуры Telegram
- Условия WHERE в базе данных
- Любая коллекция структурированных данных

**Хранимые данные:** `ParameterValue::Array(Vec<ParameterValue>)` или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "Array",
  "value": [
    {
      "type": "Object",
      "value": {
        "name": {"type": "String", "value": "Accept"},
        "value": {"type": "String", "value": "application/json"}
      }
    },
    {
      "type": "Object",
      "value": {
        "name": {"type": "String", "value": "User-Agent"},
        "value": {"type": "String", "value": "MyApp/1.0"}
      }
    }
  ]
}

// Выражение
{
  "type": "Expression",
  "value": "{{$json.headers}}"
}
```

**Примеры выражений:**
- Статическое: `[{"name": "Accept", "value": "application/json"}, {"name": "User-Agent", "value": "MyApp"}]`
- Выражение: `"{{$json.headers}}"` → динамический список заголовков из предыдущего шага

#### Архитектурные принципы

**🎯 Основной принцип: Независимые элементы**
- Каждый элемент списка полностью независим
- Нет зависимостей между элементами
- Платформа автоматически генерирует технические ID
- Четкое разделение обязанностей

**🔧 Ответственность платформы:**
- Автоматическая генерация ID для элементов списка (`item_0`, `item_1` и т.д.)
- Управление UI (добавить/удалить/переупорядочить)
- Управление состоянием и сохранение
- Анимация и визуальная обратная связь

**👨‍💻 Ответственность разработчика:**
- Определить бизнес-структуру через `item_template`
- Указать ограничения и правила валидации
- Фокусироваться на типах данных, а не технической реализации

#### Примеры использования

**Простой список строк:**
```rust
// Теги как текстовые элементы
let tags = ListParameter::new(
    TextParameter::builder()
        .metadata(ParameterMetadata::required("tag", "Tag")?)
        .build()?
)
.metadata(ParameterMetadata::simple("tags", "Tags")?)
.constraints(ListConstraints {
    min_items: 1,
    max_items: Some(10),
    sortable: false,
    unique_items: true,
})
.build()?;
```

**Структурированные объекты (HTTP заголовки):**
```rust
let headers = ListParameter::new(
    ObjectParameter::builder()
        .metadata(ParameterMetadata::simple("header", "Header")?)
        .add_field("name", TextParameter::builder()
            .metadata(ParameterMetadata::required("name", "Header Name")?)
            .build()?)
        .add_field("value", TextParameter::builder()
            .metadata(ParameterMetadata::required("value", "Header Value")?)
            .build()?)
        .build()?
)
.metadata(ParameterMetadata::simple("headers", "HTTP Headers")?)
.constraints(ListConstraints {
    min_items: 0,
    max_items: Some(50),
    sortable: true,
    unique_items: false,
})
.ui_config(ListUIConfig {
    add_button_text: Some("Add Header".into()),
    empty_text: Some("No headers configured".into()),
    show_indices: false,
    show_delete: true,
    show_reorder: true,
    layout: ListLayout::Vertical,
    animate: true,
})
.build()?;
```

**Вложенные списки (Telegram клавиатура):**
```rust
// Telegram встроенная клавиатура: Список строк, каждая строка - список кнопок
let inline_keyboard = ListParameter::new(
    // Каждая строка - это список кнопок
    ListParameter::new(
        // Каждая кнопка - это объект
        ObjectParameter::builder()
            .metadata(ParameterMetadata::simple("button", "Button")?)
            .add_field("text", TextParameter::builder()
                .metadata(ParameterMetadata::required("text", "Button Text")?)
                .build()?)
            .add_field("type", SelectParameter::builder()
                .metadata(ParameterMetadata::required("type", "Button Type")?)
                .options(vec![
                    SelectOption::new("url", "URL"),
                    SelectOption::new("callback_data", "Callback Data"),
                ])
                .build()?)
            .add_field("url", TextParameter::builder()
                .metadata(ParameterMetadata::simple("url", "URL")?)
                .ui_options(TextUiOptions {
                    input_type: TextInputType::URL,
                    multiline: false,
                })
                .display(ParameterDisplay::builder()
                    .show_when("type", ParameterCondition::Eq(json!("url")))
                    .build())
                .build()?)
            .add_field("callback_data", TextParameter::builder()
                .metadata(ParameterMetadata::simple("callback_data", "Callback Data")?)
                .display(ParameterDisplay::builder()
                    .show_when("type", ParameterCondition::Eq(json!("callback_data")))
                    .build())
                .build()?)
            .build()?
    )
    .metadata(ParameterMetadata::simple("buttons", "Buttons in Row")?)
    .constraints(ListConstraints {
        min_items: 1,
        max_items: Some(5), // Лимит Telegram
        sortable: false,
        unique_items: false,
    })
    .build()?
)
.metadata(ParameterMetadata::simple("keyboard", "Inline Keyboard")?)
.constraints(ListConstraints {
    min_items: 0,
    max_items: Some(20), // Разумный лимит
    sortable: true,
    unique_items: false,
})
.ui_config(ListUIConfig {
    add_button_text: Some("Add Keyboard Row".into()),
    empty_text: Some("No keyboard rows".into()),
    show_indices: true,
    show_delete: true,
    show_reorder: true,
    layout: ListLayout::Vertical,
    animate: true,
})
.build()?;
```

**Сложный список с валидацией (WHERE условия):**
```rust
let where_conditions = ListParameter::new(
    ObjectParameter::builder()
        .metadata(ParameterMetadata::simple("condition", "WHERE Condition")?)
        .add_field("field", TextParameter::builder()
            .metadata(ParameterMetadata::required("field", "Field Name")?)
            .build()?)
        .add_field("operator", SelectParameter::builder()
            .metadata(ParameterMetadata::required("operator", "Operator")?)
            .options(vec![
                SelectOption::new("=", "Equals"),
                SelectOption::new("!=", "Not Equals"),
                SelectOption::new(">", "Greater Than"),
                SelectOption::new("<", "Less Than"),
                SelectOption::new("LIKE", "Like"),
                SelectOption::new("IN", "In"),
            ])
            .build()?)
        .add_field("value", TextParameter::builder()
            .metadata(ParameterMetadata::required("value", "Value")?)
            .build()?)
        .build()?
)
.metadata(ParameterMetadata::simple("where", "WHERE Conditions")?)
.constraints(ListConstraints {
    min_items: 0,
    max_items: Some(10),
    sortable: true,
    unique_items: false,
})
.validate(|items| {
    // Валидация на отсутствие дублирующихся полей
    let mut fields = std::collections::HashSet::new();
    for item in items {
        if let Some(obj) = item.as_object() {
            if let Some(field) = obj.get("field").and_then(|v| v.as_str()) {
                if !fields.insert(field) {
                    return Err(format!("Duplicate field in WHERE conditions: {}", field));
                }
            }
        }
    }
    
    // Валидация разумного количества условий
    if items.len() > 5 {
        return Err("Too many WHERE conditions. Consider using a more specific query.".to_string());
    }
    
    Ok(())
})
.build()?;
```

**Список файлов:**
```rust
let attachments = ListParameter::new(
    FileParameter::builder()
        .metadata(ParameterMetadata::simple("attachment", "Attachment")?)
        .ui_options(FileUiOptions {
            accept: vec![
                "image/*".into(),
                "application/pdf".into(),
                ".doc".into(),
                ".docx".into(),
            ],
            max_size: Some(5 * 1024 * 1024), // 5MB на файл
            multiple: false,
            preview: true,
        })
        .build()?
)
.metadata(ParameterMetadata::simple("attachments", "Attachments")?)
.constraints(ListConstraints {
    min_items: 0,
    max_items: Some(10),
    sortable: false,
    unique_items: false,
})
.ui_config(ListUIConfig {
    add_button_text: Some("Add Attachment".into()),
    empty_text: Some("No attachments".into()),
    show_indices: false,
    show_delete: true,
    show_reorder: false,
    layout: ListLayout::Vertical,
    animate: true,
})
.build()?;
```

#### Ключевые преимущества

**🎯 Чистая архитектура:**
- Нет технических ID в бизнес-логике
- Независимые элементы с четкими границами
- Платформа обрабатывает все технические аспекты

**🔧 Удобство для разработчиков:**
- Простой подход на основе шаблонов
- Фокус на бизнес-структуре, а не реализации
- Мощная валидация и ограничения

**🎨 UI-агностичность:**
- Платформа обрабатывает все UI аспекты
- Единообразное поведение во всех списках
- Автоматические анимации и управление состоянием

**📊 Поток данных:**
```
Разработчик определяет шаблон → Платформа генерирует UI → Пользователь взаимодействует → Платформа управляет состоянием → Чистые данные в Action
```

#### Общие анти-паттерны, которых следует избегать

**❌ Не добавляйте технические ID:**
```rust
// Неправильно - не добавляйте внутренние ID
ObjectParameter::builder()
    .add_field("id", HiddenParameter::builder()...)  // Платформа обрабатывает это
    .add_field("index", NumberParameter::builder()...)  // Платформа обрабатывает это
```

**❌ Не создавайте зависимости между элементами:**
```rust
// Неправильно - элементы должны быть независимы
// Не пытайтесь сделать элемент N зависимым от элемента N-1
```

**❌ Не переопределяйте UI аспекты платформы:**
```rust
// Неправильно - позвольте платформе обрабатывать UI
.ui_config(ListUIConfig {
    custom_css: Some("..."),  // Платформа обрабатывает стилизацию
    custom_animations: Some("..."),  // Платформа обрабатывает анимации
})
```

**✅ Фокусируйтесь на бизнес-логике:**
```rust
// Правильно - чистая бизнес-структура
ListParameter::new(business_template)
    .constraints(business_constraints)
    .validate(business_validation)
```

Этот подход обеспечивает четкое разделение обязанностей и делает ListParameter действительно универсальным для любой коллекции структурированных данных.

---

### 22. RoutingParameter

**Назначение:** Обертка, которая добавляет возможности маршрутизации к параметрам списка, позволяя динамические точки подключения в рабочих процессах на основе узлов.

**Когда использовать:**
- Switch узлы с предопределенными случаями
- Условная маршрутизация на основе статических значений
- Многовыходные узлы, где выходы известны во время проектирования
- Любой сценарий, где нужно генерировать визуальные точки подключения из списка значений

**Когда НЕ использовать:**
- Динамическая маршрутизация на основе данных времени выполнения (используйте Action.outputs() вместо этого)
- Выходы запросов к базе данных (используйте Action.outputs() вместо этого)
- Выходы разбора JSON (используйте Action.outputs() вместо этого)
- Любой сценарий, где выходы зависят от анализа входных данных

**Хранимые данные:** `ParameterValue::Array(Vec<ParameterValue>)` (из внутреннего ListParameter) или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Статическое значение
{
  "type": "Array",
  "value": [
    {"type": "String", "value": "admin"},
    {"type": "String", "value": "user"},
    {"type": "String", "value": "guest"}
  ]
}

// Выражение
{
  "type": "Expression",
  "value": "{{$json.userRoles}}"
}
```

**Примеры выражений:**
- Статическое: `["admin", "user", "guest"]` → генерирует точки подключения admin, user, guest, default
- Выражение: `"{{$json.userRoles}}"` → динамический список случаев из предыдущего шага

#### Архитектурные принципы

**🎯 Основной принцип: Статическая генерация маршрутов**
- Маршруты генерируются из предопределенного списка случаев
- Каждый случай в списке становится точкой подключения в редакторе узлов
- Точки подключения известны во время проектирования, а не во время выполнения
- Автоматическая синхронизация между изменениями списка и визуальными выходами

**🔧 Визуальные точки подключения:**
- Добавление случая в список → появляется новая точка подключения
- Удаление случая → точка подключения исчезает
- Редактирование случая → точка подключения обновляется
- Платформа автоматически обрабатывает обновления UI

**🎨 Время проектирования vs Время выполнения:**
- **Время проектирования:** RoutingParameter генерирует визуальные точки подключения
- **Время выполнения:** Action.execute() направляет данные к соответствующим выходам
- **Динамические случаи:** Используйте Action.outputs() для маршрутизации, зависящей от времени выполнения

#### Примеры использования

**Простой Switch узел:**
```rust
let switch_cases = RoutingParameter::with_text_cases(
    ParameterMetadata::required("switch_cases", "Switch Cases")?
)
.routing_config(RoutingConfig {
    route_naming: RouteNaming::UseItemValues,
    max_routes: Some(20),
    include_default: true,
    default_route: Some(DefaultRoute {
        key: "default".to_string(),
        label: "Default".to_string(),
        description: Some("When no cases match".to_string()),
        icon: Some("arrow-down".to_string()),
    }),
    route_styling: RouteStyleConfig {
        default_color: "#4CAF50".to_string(),
        default_route_color: "#FF9800".to_string(),
        line_thickness: 2,
        line_pattern: LinePattern::Solid,
    },
})
.build()?;
```

**Сложная маршрутизация на основе объектов:**
```rust
let object_routing = RoutingParameter::with_object_cases(
    ParameterMetadata::required("complex_routes", "Complex Routes")?,
    ObjectParameter::builder()
        .metadata(ParameterMetadata::simple("route_config", "Route Config")?)
        .add_field("route_name", TextParameter::builder()
            .metadata(ParameterMetadata::required("route_name", "Route Name")?)
            .build()?)
        .add_field("condition", TextParameter::builder()
            .metadata(ParameterMetadata::required("condition", "Condition")?)
            .build()?)
        .add_field("priority", NumberParameter::builder()
            .metadata(ParameterMetadata::optional("priority", "Priority")?)
            .default(1.0)
            .build()?)
        .build()?
)
.routing_config(RoutingConfig {
    route_naming: RouteNaming::UseItemField { 
        field_name: "route_name".to_string() 
    },
    max_routes: Some(50),
    include_default: true,
    default_route: Some(DefaultRoute {
        key: "fallback".to_string(),
        label: "Fallback".to_string(),
        description: Some("When no conditions match".to_string()),
        icon: Some("shield".to_string()),
    }),
    route_styling: RouteStyleConfig {
        default_color: "#2196F3".to_string(),
        default_route_color: "#F44336".to_string(),
        line_thickness: 3,
        line_pattern: LinePattern::Solid,
    },
})
.build()?;
```

**Маршрутизация с именованием на основе шаблона:**
```rust
let templated_routing = RoutingParameter::with_text_cases(
    ParameterMetadata::required("templated_routes", "Templated Routes")?
)
.routing_config(RoutingConfig {
    route_naming: RouteNaming::Template { 
        template: "output_{value}".to_string() 
    },
    max_routes: Some(10),
    include_default: false,
    default_route: None,
    route_styling: RouteStyleConfig {
        default_color: "#9C27B0".to_string(),
        default_route_color: "#607D8B".to_string(),
        line_thickness: 2,
        line_pattern: LinePattern::Dotted,
    },
})
.build()?;
```

#### Интеграция с Actions

**Определение Action:**
```rust
pub fn create_switch_node() -> ActionDefinition {
    ActionDefinition::builder()
        .name("Switch")
        .description("Route data based on switch cases")
        .parameters(vec![
            // Входное значение для сравнения
            Parameter::Text(
                TextParameter::builder()
                    .metadata(ParameterMetadata::required("input_value", "Input Value")?)
                    .build()?
            ),
            
            // Параметр маршрутизации с случаями
            Parameter::Routing(
                RoutingParameter::with_text_cases(
                    ParameterMetadata::required("cases", "Switch Cases")?
                )
                .include_default(DefaultRoute {
                    key: "default".to_string(),
                    label: "Default".to_string(),
                    description: Some("When input doesn't match any case".to_string()),
                    icon: Some("arrow-down".to_string()),
                })
                .max_routes(20)
                .build()?
            ),
        ])
        .build()
}
```

**Реализация Action:**
```rust
impl Action for SwitchAction {
    fn execute(&self, params: &ParameterValues) -> Result<ExecutionResult, Error> {
        let input_value = params.get("input_value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::MissingParameter("input_value".into()))?;
        
        let cases = params.get("cases")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::MissingParameter("cases".into()))?;
        
        let input_data = params.get("input_data");
        
        // Проверка каждого случая на совпадение
        for case_value in cases {
            if let Some(case_str) = case_value.as_str() {
                if input_value == case_str {
                    // Найден совпадающий случай - направить к соответствующему выходу
                    return Ok(ExecutionResult {
                        outputs: vec![
                            (case_str.to_string(), input_data.cloned())
                        ].into_iter().collect(),
                        status: ExecutionStatus::Success,
                    });
                }
            }
        }
        
        // Нет совпадающего случая - направить к выходу по умолчанию
        Ok(ExecutionResult {
            outputs: vec![
                ("default".to_string(), input_data.cloned())
            ].into_iter().collect(),
            status: ExecutionStatus::Success,
        })
    }
}
```

#### Визуальное представление

**Отображение в редакторе узлов:**
```
┌─────────────────────────────────────────┐
│              Switch Node                │
├─────────────────────────────────────────┤
│ Input Value: [user_type_______________] │
│                                         │
│ Switch Cases:                           │
│   ┌─────────────────────────────────┐   │
│   │ admin                    [Del]  │   │  ●─── admin
│   │ user                     [Del]  │   │  ●─── user
│   │ guest                    [Del]  │   │  ●─── guest
│   │ moderator                [Del]  │   │  ●─── moderator
│   └─────────────────────────────────┘   │
│   [Add Case]                            │  ●─── default
│                                         │
└─────────────────────────────────────────┘
```

**Обновления в реальном времени:**
- Добавить случай "admin" → новая точка подключения появляется справа
- Удалить случай "user" → точка подключения исчезает
- Редактировать случай "guest" → метка точки подключения обновляется
- Все изменения синхронизируются автоматически

#### Ключевые преимущества

**🎯 Статическая маршрутизация во время проектирования:**
- Точки подключения известны во время проектирования
- Визуальная обратная связь для всех возможных маршрутов
- Нет сюрпризов во время выполнения о доступных выходах

**🔄 Автоматическая синхронизация:**
- Изменения списка мгновенно обновляют точки подключения
- Не требуется ручное обновление или перестройка
- Согласованное управление состоянием UI

**🎨 Визуальная ясность:**
- Четкая связь между конфигурацией параметра и выходами узла
- Интуитивное отношение между случаями и маршрутами
- Немедленная визуальная обратная связь для изменений конфигурации

**🛠️ Удобство для разработчиков:**
- Простой API для общих сценариев маршрутизации
- Гибкая конфигурация для сложных случаев
- Четкое разделение от логики маршрутизации времени выполнения

#### Общие паттерны

**Маршрутизация ролей пользователей:**
```rust
let user_roles = RoutingParameter::with_text_cases(
    ParameterMetadata::required("user_roles", "User Roles")?
)
.include_default(DefaultRoute {
    key: "anonymous".to_string(),
    label: "Anonymous".to_string(),
    description: Some("Users without specific roles".to_string()),
    icon: Some("user".to_string()),
})
.build()?;
```

**Маршрутизация кодов статуса HTTP:**
```rust
let http_status = RoutingParameter::with_text_cases(
    ParameterMetadata::required("status_codes", "HTTP Status Codes")?
)
.routing_config(RoutingConfig {
    route_naming: RouteNaming::Template { 
        template: "status_{value}".to_string() 
    },
    include_default: true,
    default_route: Some(DefaultRoute {
        key: "unexpected".to_string(),
        label: "Unexpected".to_string(),
        description: Some("Unexpected status codes".to_string()),
        icon: Some("warning".to_string()),
    }),
    // ... другая конфигурация
})
.build()?;
```

#### Принципы проектирования

**✅ Хорошее использование:**
- Логика switch/case с предопределенными значениями
- Многовыходные узлы с известными выходами
- Условная маршрутизация на основе статической конфигурации
- Любой сценарий, где маршруты можно определить во время проектирования

**❌ Избегайте для:**
- Динамической маршрутизации на основе анализа входных данных
- Маршрутизации результатов запросов к базе данных
- Маршрутизации ключей объектов JSON
- Любого сценария, где маршруты зависят от данных времени выполнения

Для динамических сценариев маршрутизации используйте метод `Action.outputs()`, который имеет доступ к данным времени выполнения и может генерировать выходы динамически.

Этот подход обеспечивает четкое разделение между конфигурацией времени проектирования (RoutingParameter) и поведением времени выполнения (Action.outputs()).

---

### 23. ModeParameter

**Назначение:** Переключение между различными режимами ввода.

**Когда использовать:**
- Простой выбор vs пользовательский ввод
- Разные уровни сложности
- Адаптивные интерфейсы

**Хранимые данные:** Значение зависит от выбранного режима - может быть любой тип `ParameterValue` или `ParameterValue::Expression(String)`

**Примеры в базе данных:**
```json
// Режим текста
{
  "type": "String",
  "value": "https://custom-api.example.com",
  "mode": "text"
}

// Режим выбора
{
  "type": "String",
  "value": "https://api.example.com",
  "mode": "select"
}

// Режим кода
{
  "type": "String",
  "value": "function getUrl() {\n  return 'https://api.example.com';\n}",
  "mode": "code"
}
```

**Примечание:** ModeParameter хранит значение из режима, который в данный момент активен (текст, выбор, код и т.д.)

**Примеры кода:**
```rust
// Гибкий ввод URL
let url_input = ModeParameter::builder()
    .metadata(ParameterMetadata::required("url", "API URL")?)
    .text_mode("Custom", TextParameter::builder()
        .metadata(ParameterMetadata::simple("custom_url", "Custom URL")?)
        .ui_options(TextUiOptions {
            input_type: TextInputType::URL,
            multiline: false,
        })
        .build()?)
    .select_mode("Predefined", SelectParameter::builder()
        .metadata(ParameterMetadata::simple("predefined_url", "Select URL")?)
        .options(vec![
            SelectOption::new("https://api.example.com", "Production API"),
            SelectOption::new("https://staging.example.com", "Staging API"),
            SelectOption::new("https://dev.example.com", "Development API"),
        ])
        .build()?)
    .expression_mode("Dynamic", ExpressionParameter::builder()
        .metadata(ParameterMetadata::simple("dynamic_url", "Dynamic URL")?)
        .ui_options(ExpressionUiOptions {
            mode: ExpressionMode::Mixed,
            available_variables: vec![
                ExpressionVariable {
                    name: "Environment".into(),
                    path: "$json.environment".into(),
                    description: "Current environment".into(),
                    example_value: json!("production"),
                }
            ],
            show_preview: true,
            highlight_expressions: true,
        })
        .build()?)
    .default_mode(ModeType::Select)
    .ui_options(ModeUiOptions::tabs())
    .build()?;

// Ввод данных с разными уровнями сложности
let data_input = ModeParameter::builder()
    .metadata(ParameterMetadata::required("data", "Input Data")?)
    .simple_mode("Simple", TextParameter::builder()
        .metadata(ParameterMetadata::simple("simple_data", "Simple Text")?)
        .ui_options(TextUiOptions {
            input_type: TextInputType::Text,
            multiline: true,
            rows: Some(3),
        })
        .build()?)
    .json_mode("JSON", CodeParameter::builder()
        .metadata(ParameterMetadata::simple("json_data", "JSON Data")?)
        .ui_options(CodeUiOptions {
            language: CodeLanguage::JSON,
            height: 8,
            available_variables: vec![],
        })
        .build()?)
    .builder_mode("Builder", ObjectParameter::builder()
        .metadata(ParameterMetadata::simple("builder_data", "Data Builder")?)
        .add_field("key", TextParameter::builder()
            .metadata(ParameterMetadata::required("key", "Key")?)
            .build()?)
        .add_field("value", TextParameter::builder()
            .metadata(ParameterMetadata::required("value", "Value")?)
            .build()?)
        .add_field("type", SelectParameter::builder()
            .metadata(ParameterMetadata::required("type", "Type")?)
            .options(vec![
                SelectOption::new("string", "String"),
                SelectOption::new("number", "Number"),
                SelectOption::new("boolean", "Boolean"),
            ])
            .build()?)
        .build()?)
    .default_mode(ModeType::Simple)
    .ui_options(ModeUiOptions::dropdown())
    .build()?;

// Авторизация с разными методами
let auth_config = ModeParameter::builder()
    .metadata(ParameterMetadata::required("auth", "Authorization")?)
    .mode("basic", "Basic Auth", ObjectParameter::builder()
        .metadata(ParameterMetadata::simple("basic_auth", "Basic Authentication")?)
        .add_field("username", TextParameter::builder()
            .metadata(ParameterMetadata::required("username", "Username")?)
            .build()?)
        .add_field("password", SecretParameter::builder()
            .metadata(ParameterMetadata::required("password", "Password")?)
            .build()?)
        .build()?)
    .mode("oauth", "OAuth 2.0", ObjectParameter::builder()
        .metadata(ParameterMetadata::simple("oauth", "OAuth 2.0")?)
        .add_field("client_id", TextParameter::builder()
            .metadata(ParameterMetadata::required("client_id", "Client ID")?)
            .build()?)
        .add_field("client_secret", SecretParameter::builder()
            .metadata(ParameterMetadata::required("client_secret", "Client Secret")?)
            .build()?)
        .add_field("scope", TextParameter::builder()
            .metadata(ParameterMetadata::optional("scope", "Scope")?)
            .placeholder("read write")
            .build()?)
        .build()?)
    .mode("api_key", "API Key", SecretParameter::builder()
        .metadata(ParameterMetadata::required("api_key", "API Key")?)
        .build()?)
    .mode("none", "No Auth", NoticeParameter::builder()
        .metadata(ParameterMetadata::simple("no_auth", "No Authentication")?)
        .notice_type(NoticeType::Info)
        .build()?)
    .default_mode("none")
    .ui_options(ModeUiOptions::radio())
    .build()?;

