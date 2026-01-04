# nebula-parameter

Комплексная типобезопасная система параметров для движка рабочих процессов Nebula, обеспечивающая гибкое определение параметров, валидацию и возможности условного отображения с интеграцией expression system.

## Содержание

1. [Обзор](#обзор)
2. [Ключевые концепции](#ключевые-концепции)
3. [Архитектура](#архитектура)
4. [Типы параметров](#типы-параметров)
5. [Система валидации](#система-валидации)
6. [Система отображения](#система-отображения)
7. [Коллекции параметров](#коллекции-параметров)
8. [Интеграция с Expression System](#интеграция-с-expression-system)
9. [Расширенные возможности](#расширенные-возможности)
10. [Лучшие практики](#лучшие-практики)
11. [Справочник API](#справочник-api)

## Обзор

Система параметров Nebula решает фундаментальную проблему **типобезопасной конфигурации workflow узлов**. Она предоставляет мощную систему для определения, валидации и управления параметрами в workflow узлах с акцентом на типовую безопасность, расширяемость и четкое разделение обязанностей между хранением данных, логикой валидации и представлением в UI.

### Ключевые возможности

- **Типовая безопасность**: Строго типизированные параметры с гарантиями на этапе компиляции
- **Высокая производительность**: Инкрементальная валидация O(k) вместо O(n)
- **Детерминированный порядок**: Стабильная последовательность параметров для UI
- **Гибкая архитектура**: Поддержка как программного подхода, так и derive макросов
- **Комплексная валидация**: Гибкая система валидации со встроенными и пользовательскими правилами
- **Условное отображение**: Показ/скрытие параметров на основе динамических условий
- **Безопасность прежде всего**: Безопасные типы параметров с автоматической очисткой памяти
- **Builder Pattern**: Эргономичный API для конструирования параметров
- **Expression Integration**: Встроенная поддержка динамических выражений
- **Rich UI Configuration**: Отдельная UI конфигурация от бизнес-логики
- **Полная сериализация**: Полная поддержка serde с умными значениями по умолчанию

### Философия дизайна

Система параметров следует следующим ключевым принципам:

1. **Разделение обязанностей**: Данные, валидация, отображение и UI разделены
2. **Композиционность**: Сложное поведение построено из простых, композиционных частей
3. **Zero-Cost Abstractions**: Никаких накладных расходов на типовую безопасность
4. **Progressive Disclosure**: Простые вещи просты, сложные возможны
5. **Fail Fast**: Ошибки валидации обнаруживаются как можно раньше
6. **Производительность**: Инкрементальные обновления и эффективные структуры данных

## Ключевые концепции

### Типизированные параметры (статическая схема)

Каждый параметр содержит схему, чистое значение и метаданные для оптимизации:

```rust
use nebula_value::{Value, ValidationRule};

pub struct TextParameter {
    // Статическая схема
    pub metadata: ParameterMetadata,
    pub validation: Option<Vec<ValidationRule>>,
    pub display: Option<ParameterDisplay>,
    pub ui_options: TextUIOptions,
    
    // Чистое значение (после разрешения Expression и валидации)
    pub value: Option<String>,
    pub default_value: Option<String>,
}

pub struct NumberParameter {
    // Статическая схема
    pub metadata: ParameterMetadata,
    pub validation: Option<Vec<ValidationRule>>,
    pub display: Option<ParameterDisplay>,
    pub ui_options: NumberUIOptions,
    
    // Чистое значение
    pub value: Option<f64>,
    pub default_value: Option<f64>,
}

pub struct BooleanParameter {
    // Статическая схема
    pub metadata: ParameterMetadata,
    pub validation: Option<Vec<ValidationRule>>,
    pub display: Option<ParameterDisplay>,
    pub ui_options: BooleanUIOptions,
    
    // Чистое значение
    pub value: Option<bool>,
    pub default_value: Option<bool>,
}
```

### Метаданные

Метаданные параметра предоставляют основную информацию:

```rust
pub struct ParameterMetadata {
    /// Уникальный идентификатор в пределах узла
    pub key: ParameterKey,

    /// Человекочитаемое имя
    pub name: Cow<'static, str>,

    /// Является ли параметр обязательным
    pub required: bool,

    /// Подробное описание
    pub description: Option<Cow<'static, str>>,

    /// Placeholder текст для пустых полей
    pub placeholder: Option<Cow<'static, str>>,

    /// Дополнительная информация или инструкции
    pub hint: Option<Cow<'static, str>>,

    /// Группа для организации
    pub group: Option<Cow<'static, str>>,

    /// Порядок отображения (опционально - по умолчанию порядок добавления)
    pub order: Option<u32>,
}

impl ParameterMetadata {
    pub fn builder() -> ParameterMetadataBuilder {
        ParameterMetadataBuilder::new()
    }
    
    pub fn required(key: &str, name: &str) -> Result<Self, ParameterError> {
        Self::builder()
            .key(key)
            .name(name)
            .required(true)
            .build()
    }
    
    pub fn optional(key: &str, name: &str) -> Result<Self, ParameterError> {
        Self::builder()
            .key(key)
            .name(name)
            .required(false)
            .build()
    }
}
```

### Жизненный цикл параметра

1. **Определение**: Параметр определяется с метаданными, валидацией и UI опциями (статическая схема)
2. **Получение значений**: Загрузка сохраненных значений из базы данных 
3. **Разрешение**: Expression значения разрешаются в конкретные значения через ExpressionContext
4. **Валидация**: Разрешенные значения валидируются против правил параметра
5. **Установка**: Чистые проваличированные значения устанавливаются в параметр

### Архитектура разделения

```rust
use nebula_value::{Value, Expression, ValidationRule, ExpressionContext};

// 1. Схема параметра (статическая структура)
pub struct TextParameter {
    pub metadata: ParameterMetadata,
    pub validation: Option<Vec<ValidationRule>>,
    pub display: Option<ParameterDisplay>, 
    pub ui_options: TextUIOptions,
    
    // Текущее ЧИСТОЕ значение (после разрешения и валидации)
    pub value: Option<String>,
    pub default: Option<String>,
}

// 2. Сохраненное значение из базы (может быть Expression)
#[derive(Serialize, Deserialize)]
pub struct ParameterValueEntry {
    pub parameter_key: String,
    pub value: Value, // Либо статическое значение, либо Expression из nebula-value
}

// 3. Процесс установки значений
impl ParameterCollection {
    /// Установить значения из базы данных с разрешением expressions
    pub async fn set_values_from_storage(
        &mut self,
        stored_values: Vec<ParameterValueEntry>,
        expression_context: &ExpressionContext,
    ) -> Result<(), ParameterError> {
        for entry in stored_values {
            if let Some(parameter) = self.get_parameter_mut(&entry.parameter_key) {
                // 1. Разрешаем значение если это Expression
                let resolved_value = match &entry.value {
                    Value::Expression(expr) => {
                        expression_context.evaluate_expression(expr).await?
                    }
                    static_value => static_value.clone(),
                };
                
                // 2. Валидируем разрешенное значение
                parameter.validate_value(&resolved_value)?;
                
                // 3. Устанавливаем чистое значение
                parameter.set_resolved_value(resolved_value)?;
            }
        }
        Ok(())
    }
}
```

## Архитектура

### Иерархия трейтов

Система параметров построена на иерархии трейтов:

```rust
/// Базовый трейт для всех параметров
pub trait ParameterType {
    fn kind(&self) -> ParameterKind;
    fn metadata(&self) -> &ParameterMetadata;
    fn is_required(&self) -> bool;
    fn get_group(&self) -> Option<&str>;
}

/// Трейт для работы с чистыми значениями
pub trait HasValue: ParameterType {
    type Value: Clone + PartialEq + Debug + 'static;

    // Работа с чистыми значениями (после разрешения Expression)
    fn get_value(&self) -> Option<&Self::Value>;
    fn set_value(&mut self, value: Self::Value) -> Result<(), ParameterError>;
    fn clear_value(&mut self);
    fn has_value(&self) -> bool;
    
    // Значения по умолчанию
    fn get_default_value(&self) -> Option<&Self::Value>;
    fn is_using_default(&self) -> bool;
    fn reset_to_default(&mut self) -> Result<(), ParameterError>;
    
    // Установка значения с разрешением Expression и валидацией
    fn set_value_from_storage(
        &mut self, 
        stored_value: Value,
        expression_context: &ExpressionContext,
    ) -> Result<(), ParameterError> {
        // 1. Разрешаем Expression если нужно
        let resolved_value = match stored_value {
            Value::Expression(expr) => {
                expression_context.evaluate_expression(&expr)?
            }
            static_value => static_value,
        };
        
        // 2. Конвертируем в нужный тип
        let typed_value = Self::Value::try_from(resolved_value)?;
        
        // 3. Валидируем
        self.validate_value(&typed_value)?;
        
        // 4. Устанавливаем чистое значение
        self.set_value(typed_value)
    }
    
    // Валидация значения
    fn validate_value(&self, value: &Self::Value) -> Result<(), ValidationError>;
    
    // Конвертация типов
    fn try_from_value(value: Value) -> Result<Self::Value, ConversionError>;
    fn to_value(&self) -> Option<Value>;
}

/// Параметры, поддерживающие валидацию
pub trait Validatable: HasValue {
    fn validate(&self, value: &Self::Value) -> Result<(), ValidationError>;
    fn validation_rules(&self) -> &[ValidationRule];
    fn add_validation_rule(&mut self, rule: ValidationRule);
}

/// Параметры, поддерживающие условное отображение
pub trait Displayable: ParameterType {
    fn display(&self) -> Option<&ParameterDisplay>;
    fn should_display(&self, context: &DisplayContext) -> bool;
    fn display_dependencies(&self) -> Vec<ParameterKey>;
}

/// Параметры с поддержкой expressions
pub trait ExpressionEnabled: HasValue {
    fn supports_expressions(&self) -> bool { true }
    fn set_expression(&mut self, expression: String) -> Result<(), ParameterError>;
    fn get_expression(&self) -> Option<&str>;
    fn resolve_expression(&self, context: &ExpressionContext) -> Result<Self::Value, ExpressionError>;
}
```

### Коллекция параметров с улучшенной производительностью

```rust
pub struct ParameterCollection {
    // IndexMap для детерминированного порядка (важно для UI)
    parameters: IndexMap<ParameterKey, Parameter>,
    groups: Vec<ParameterGroup>,
    
    // Метаданные для оптимизации производительности
    metadata: CollectionMetadata,
}

pub struct CollectionMetadata {
    // Версия для отслеживания изменений
    version: u64,
    
    // Битовая маска изменённых параметров - O(1) операции
    dirty_mask: BitSet,
    
    // Кэш результатов валидации
    validation_cache: ValidationCache,
    
    // Предвычисленные зависимости для быстрого поиска
    dependency_graph: DependencyGraph,
}

/// Инкрементальная валидация - ключевое архитектурное улучшение
impl ParameterCollection {
    pub fn validate_incremental(&mut self) -> Result<ValidationResult, ParameterError> {
        // Быстрая проверка - есть ли изменения?
        if self.metadata.dirty_mask.is_empty() {
            return Ok(ValidationResult::valid()); // O(1) выход!
        }
        
        // Вычисляем минимальный набор для валидации
        let dirty_params = self.metadata.dirty_mask.clone();
        let affected_params = self.metadata.dependency_graph
            .compute_affected_set(&dirty_params);
        
        let mut validation_errors = Vec::new();
        
        // Валидируем только затронутые параметры
        for param_index in affected_params.iter() {
            let param_key = self.get_key_by_index(param_index);
            
            // Проверяем кэш сначала
            if let Some(cached_result) = self.metadata.validation_cache
                .get(param_key, self.get_parameter_version(param_key)) {
                if let Err(errors) = cached_result {
                    validation_errors.extend(errors);
                }
                continue;
            }
            
            // Валидируем и кэшируем результат
            match self.validate_parameter(param_key) {
                Ok(()) => {
                    self.metadata.validation_cache.insert_success(param_key);
                }
                Err(errors) => {
                    self.metadata.validation_cache.insert_errors(param_key, &errors);
                    validation_errors.extend(errors);
                }
            }
        }
        
        // Очищаем dirty флаги
        self.metadata.dirty_mask.clear();
        self.metadata.version += 1;
        
        if validation_errors.is_empty() {
            Ok(ValidationResult::valid())
        } else {
            Ok(ValidationResult::invalid(validation_errors))
        }
    }
    
    /// Обновление значения с автоматическим отслеживанием изменений
    pub fn set_value(&mut self, key: &ParameterKey, value: Value) -> Result<(), ParameterError> {
        let param_index = self.get_parameter_index(key)
            .ok_or_else(|| ParameterError::ParameterNotFound(key.clone()))?;
            
        let param = self.parameters.get_mut(key).unwrap();
        let old_value = param.get_value().cloned();
        
        // Устанавливаем новое значение
        param.set_value_internal(value)?;
        
        // Проверяем изменилось ли значение
        let new_value = param.get_value();
        if old_value.as_ref() != new_value {
            // Отмечаем параметр как изменённый
            self.metadata.dirty_mask.insert(param_index);
            
            // Инвалидируем кэш для зависимых параметров
            let dependents = self.metadata.dependency_graph
                .get_dependents(param_index);
            for &dependent_index in dependents {
                self.metadata.dirty_mask.insert(dependent_index);
                self.metadata.validation_cache.invalidate_by_index(dependent_index);
            }
        }
        
        Ok(())
    }
}
```

### Система типов

Параметры используют систему типов Rust для безопасности:

- Строгая типизация предотвращает путаницу типов
- `Option<T>` для опциональных значений
- `Result<T, E>` для операций, которые могут завершиться ошибкой
- Phantom типы для гарантий на этапе компиляции
- Zero-cost newtype обертки

## Типы параметров

### TextParameter

Для ввода текста в одну или несколько строк:

```rust
// Простой текстовый ввод
let username = TextParameter::builder()
    .metadata(
        ParameterMetadata::builder()
            .key("username")
            .name("Username")
            .required(true)
            .placeholder("Enter username")
            .build()?
    )
    .validation(vec![
        ValidationRule::MinLength(3),
        ValidationRule::MaxLength(50),
        ValidationRule::Pattern(r"^[a-zA-Z0-9_]+$".into()),
    ])
    .build()?;

// Многострочный с дополнительными опциями
let description = TextParameter::builder()
    .metadata(ParameterMetadata::optional("description", "Description")?)
    .ui_options(TextUIOptions {
        multiline: true,
        input_type: TextInputType::Text,
        ..Default::default()
    })
    .validation(vec![
        ValidationRule::MinLength(10),
        ValidationRule::MaxLength(500),
    ])
    .build()?;

// С input mask
let phone = TextParameter::builder()
    .metadata(ParameterMetadata::required("phone", "Phone Number")?)
    .ui_options(TextUIOptions {
        input_type: TextInputType::Tel,
        mask: Some("(999) 999-9999".into()),
        ..Default::default()
    })
    .build()?;
```

**UI опции:**
- `multiline` - включить многострочный режим
- `input_type` - тип ввода (Text, Password, Email, URL, Tel, Search)
- `mask` - маска для форматирования ввода

**Типы ввода:**
```rust
pub enum TextInputType {
    Text,     // Обычный текст
    Password, // Скрытый ввод
    Email,    // Email с валидацией браузера
    URL,      // URL с валидацией браузера
    Tel,      // Телефон
    Search,   // Поиск
}
```

### SecretParameter

Безопасный параметр с автоматической очисткой памяти:

```rust
// API ключ
let api_key = SecretParameter::builder()
    .metadata(
        ParameterMetadata::builder()
            .key("api_key")
            .name("API Key")
            .required(true)
            .description("Your service API key")
            .placeholder("sk-...")
            .build()?
    )
    .ui_options(SecretUIOptions {
        show_reveal_button: true,
        mask_character: Some('*'),
        strength_meter: false,
    })
    .build()?;

// Пароль с требованиями силы
let password = SecretParameter::builder()
    .metadata(ParameterMetadata::required("password", "Password")?)
    .validation(vec![
        ValidationRule::MinLength(8),
        ValidationRule::Custom {
            validator: Arc::new(validate_password_strength),
            message: "Password must contain uppercase, lowercase, digit, and special character".into(),
        },
    ])
    .ui_options(SecretUIOptions {
        show_reveal_button: true,
        strength_meter: true,
        ..Default::default()
    })
    .build()?;
```

**Особенности безопасности:**
- Автоматическая очистка памяти при удалении
- Маскировка в логах и отладочном выводе
- Шифрование при сериализации
- Защита от случайного отображения

### NumberParameter

Числовой ввод с валидацией и форматированием:

```rust
// Таймаут в секундах
let timeout = NumberParameter::builder()
    .metadata(ParameterMetadata::optional("timeout", "Timeout")?)
    .ui_options(NumberUIOptions {
        format: NumberFormat::Integer,
        min: Some(1.0),
        max: Some(300.0),
        step: Some(1.0),
        unit: Some("seconds".into()),
        show_spinner: true,
    })
    .default_value(30.0)
    .build()?;

// Цена в валюте
let price = NumberParameter::builder()
    .metadata(ParameterMetadata::optional("price", "Price")?)
    .ui_options(NumberUIOptions {
        format: NumberFormat::Currency { currency: "USD".into() },
        min: Some(0.0),
        step: Some(0.01),
        show_spinner: false,
        ..Default::default()
    })
    .build()?;

// Процент со слайдером
let confidence = NumberParameter::builder()
    .metadata(ParameterMetadata::optional("confidence", "Confidence")?)
    .ui_options(NumberUIOptions {
        format: NumberFormat::Percentage,
        min: Some(0.0),
        max: Some(100.0),
        slider: Some(SliderOptions {
            show_value: true,
            marks: vec![0.0, 25.0, 50.0, 75.0, 100.0],
        }),
        ..Default::default()
    })
    .build()?;
```

**Форматы чисел:**
```rust
pub enum NumberFormat {
    Integer,                           // только целые числа
    Decimal { precision: u8 },         // фиксированные десятичные знаки
    Currency { currency: String },     // форматирование валюты
    Percentage,                        // показывать как процент
    Scientific { precision: u8 },      // научная нотация
}
```

### BooleanParameter

Булевые значения для включения/отключения опций:

```rust
// Простой checkbox
let enabled = BooleanParameter::builder()
    .metadata(
        ParameterMetadata::builder()
            .key("enabled")
            .name("Enable Feature")
            .description("Enable this feature")
            .build()?
    )
    .default_value(true)
    .build()?;

// Switch с пользовательскими метками
let production = BooleanParameter::builder()
    .metadata(ParameterMetadata::optional("production", "Environment")?)
    .ui_options(BooleanUIOptions {
        display_type: BooleanDisplayType::Switch,
        true_label: Some("Production".into()),
        false_label: Some("Development".into()),
        show_labels: true,
    })
    .build()?;
```

**Типы отображения:**
```rust
pub enum BooleanDisplayType {
    Checkbox, // стандартный checkbox
    Switch,   // toggle switch
    Button,   // кнопка переключения
}
```

### SelectParameter

Выбор одного значения из предопределенного списка:

```rust
// HTTP метод
let method = SelectParameter::builder()
    .metadata(ParameterMetadata::required("method", "HTTP Method")?)
    .options(vec![
        SelectOption::new("GET", "GET"),
        SelectOption::new("POST", "POST"), 
        SelectOption::new("PUT", "PUT"),
        SelectOption::new("DELETE", "DELETE"),
    ])
    .ui_options(SelectUIOptions {
        searchable: false,
        clearable: false,
        ..Default::default()
    })
    .build()?;

// Большой список с поиском
let country = SelectParameter::builder()
    .metadata(ParameterMetadata::optional("country", "Country")?)
    .options(country_list)
    .ui_options(SelectUIOptions {
        searchable: true,
        clearable: true,
        placeholder: Some("Select country...".into()),
        no_options_message: Some("No countries found".into()),
    })
    .build()?;

// Combobox (можно добавлять новые)
let tag = SelectParameter::builder()
    .metadata(ParameterMetadata::optional("tag", "Tag")?)
    .options(predefined_tags)
    .ui_options(SelectUIOptions {
        searchable: true,
        creatable: true,
        create_option_message: Some("Create tag: '{value}'".into()),
        ..Default::default()
    })
    .build()?;

// С группировкой опций
let timezone = SelectParameter::builder()
    .metadata(ParameterMetadata::optional("timezone", "Timezone")?)
    .options(vec![
        SelectOption::builder()
            .value("EST")
            .label("Eastern Time")
            .group("Americas")
            .build(),
        SelectOption::builder()
            .value("PST") 
            .label("Pacific Time")
            .group("Americas")
            .build(),
        SelectOption::builder()
            .value("GMT")
            .label("Greenwich Mean Time")
            .group("Europe")
            .build(),
    ])
    .build()?;
```

**SelectOption структура:**
```rust
pub struct SelectOption {
    pub value: String,              // Значение для обработки
    pub label: String,              // Отображаемый текст
    pub description: Option<String>, // Дополнительное описание
    pub group: Option<String>,      // Группа для организации
    pub disabled: bool,             // Отключенная опция
    pub icon: Option<String>,       // Иконка
}
```

### MultiSelectParameter

Множественный выбор с ограничениями:

```rust
let permissions = MultiSelectParameter::builder()
    .metadata(ParameterMetadata::optional("permissions", "Permissions")?)
    .options(vec![
        SelectOption::new("read", "Read Access"),
        SelectOption::new("write", "Write Access"),
        SelectOption::new("admin", "Admin Access"),
    ])
    .constraints(MultiSelectConstraints {
        min_selections: Some(1),
        max_selections: None,
    })
    .ui_options(MultiSelectUIOptions {
        searchable: true,
        show_checkboxes: true,
        close_on_select: false,
        ..Default::default()
    })
    .build()?;
```

### DateTimeParameter

Ввод даты и времени с поддержкой часовых поясов:

```rust
// Только дата
let birthday = DateTimeParameter::builder()
    .metadata(ParameterMetadata::optional("birthday", "Birthday")?)
    .ui_options(DateTimeUIOptions {
        mode: DateTimeMode::DateOnly,
        max_date: Some(today()),
        show_week_numbers: true,
        ..Default::default()
    })
    .build()?;

// Дата и время с предустановками
let schedule = DateTimeParameter::builder()
    .metadata(ParameterMetadata::optional("schedule", "Schedule")?)
    .ui_options(DateTimeUIOptions {
        mode: DateTimeMode::DateTime,
        timezone_handling: TimezoneHandling::UserLocal,
        presets: vec![
            DateTimePreset::now(),
            DateTimePreset::start_of_day(),
            DateTimePreset::in_hours(1),
            DateTimePreset::tomorrow(),
        ],
        ..Default::default()
    })
    .build()?;
```

**Режимы даты и времени:**
```rust
pub enum DateTimeMode {
    DateOnly,    // Только дата
    TimeOnly,    // Только время
    DateTime,    // Дата и время
}

pub enum TimezoneHandling {
    UserLocal,   // Локальный часовой пояс пользователя
    UTC,         // Всегда UTC
    Specific(String), // Конкретный часовой пояс
}
```

### CodeParameter

Редактор кода с подсветкой синтаксиса:

```rust
// JavaScript выражение
let expression = CodeParameter::builder()
    .metadata(ParameterMetadata::required("expression", "JavaScript Expression")?)
    .ui_options(CodeUIOptions {
        language: CodeLanguage::JavaScript,
        theme: CodeTheme::Dark,
        show_line_numbers: true,
        word_wrap: false,
    })
    .validation(vec![
        ValidationRule::Custom {
            validator: Arc::new(validate_javascript_syntax),
            message: "Invalid JavaScript syntax".into(),
        },
    ])
    .build()?;

// SQL запрос
let query = CodeParameter::builder()
    .metadata(ParameterMetadata::required("query", "SQL Query")?)
    .ui_options(CodeUIOptions {
        language: CodeLanguage::SQL,
        show_minimap: true,
        autocomplete: true,
        ..Default::default()
    })
    .build()?;
```

**Поддерживаемые языки:**
```rust
pub enum CodeLanguage {
    JavaScript,
    TypeScript,
    Python,
    SQL,
    JSON,
    YAML,
    XML,
    HTML,
    CSS,
    Rust,
    Go,
    Java,
    CSharp,
    Bash,
    PowerShell,
}
```

### FileParameter

Загрузка файлов с валидацией:

```rust
let avatar = FileParameter::builder()
    .metadata(ParameterMetadata::optional("avatar", "Profile Picture")?)
    .ui_options(FileUIOptions {
        accept: vec!["image/jpeg".into(), "image/png".into()],
        max_size: Some(5 * 1024 * 1024), // 5MB
        preview: true,
        drag_drop: true,
        multiple: false,
    })
    .validation(vec![
        ValidationRule::Custom {
            validator: Arc::new(validate_image_dimensions),
            message: "Image must be at least 100x100 pixels".into(),
        },
    ])
    .build()?;

// Множественная загрузка документов
let documents = FileParameter::builder()
    .metadata(ParameterMetadata::optional("documents", "Documents")?)
    .ui_options(FileUIOptions {
        accept: vec![
            "application/pdf".into(),
            "application/msword".into(),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document".into(),
        ],
        max_size: Some(10 * 1024 * 1024), // 10MB per file
        max_total_size: Some(50 * 1024 * 1024), // 50MB total
        multiple: true,
        preview: false,
    })
    .build()?;
```

### ObjectParameter

Контейнер структурированных данных с фиксированными полями:

```rust
// HTTP заголовок
let header = ObjectParameter::builder()
    .metadata(ParameterMetadata::required("header", "HTTP Header")?)
    .add_field("name", TextParameter::builder()
        .metadata(ParameterMetadata::required("name", "Header Name")?)
        .build()?)
    .add_field("value", TextParameter::builder()
        .metadata(ParameterMetadata::required("value", "Header Value")?)
        .build()?)
    .layout(ObjectLayout::Horizontal)
    .build()?;

// Подключение к базе данных
let db_connection = ObjectParameter::builder()
    .metadata(ParameterMetadata::required("connection", "Database Connection")?)
    .add_field("host", TextParameter::builder()
        .metadata(ParameterMetadata::required("host", "Host")?)
        .default_value("localhost")
        .build()?)
    .add_field("port", NumberParameter::builder()
        .metadata(ParameterMetadata::required("port", "Port")?) 
        .ui_options(NumberUIOptions {
            format: NumberFormat::Integer,
            min: Some(1.0),
            max: Some(65535.0),
            ..Default::default()
        })
        .default_value(5432.0)
        .build()?)
    .add_field("database", TextParameter::builder()
        .metadata(ParameterMetadata::required("database", "Database Name")?)
        .build()?)
    .add_field("username", TextParameter::builder()
        .metadata(ParameterMetadata::optional("username", "Username")?)
        .build()?)
    .add_field("password", SecretParameter::builder()
        .metadata(ParameterMetadata::optional("password", "Password")?)
        .build()?)
    .layout(ObjectLayout::Vertical)
    .validate(|fields| {
        let host = fields.get("host").and_then(|v| v.as_str()).unwrap_or("");
        let port = fields.get("port").and_then(|v| v.as_f64()).unwrap_or(0.0);
        
        if host.is_empty() {
            return Err("Host is required".to_string());
        }
        
        if port <= 0.0 || port >= 65536.0 {
            return Err("Port must be between 1 and 65535".to_string());
        }
        
        Ok(())
    })
    .build()?;
```

### ListParameter

Динамические списки однотипных элементов:

```rust
// Список HTTP заголовков
let headers = ListParameter::builder()
    .metadata(ParameterMetadata::optional("headers", "HTTP Headers")?)
    .item_template(
        ObjectParameter::builder()
            .metadata(ParameterMetadata::required("header", "Header")?)
            .add_field("name", TextParameter::required("name", "Name").build()?)
            .add_field("value", TextParameter::required("value", "Value").build()?)
            .build()?
    )
    .min_items(0)
    .max_items(20)
    .default_items(vec![
        json!({"name": "Content-Type", "value": "application/json"}),
        json!({"name": "Accept", "value": "application/json"}),
    ])
    .ui_options(ListUIOptions {
        add_button_text: Some("Add Header".into()),
        remove_button_text: Some("Remove".into()),
        reorderable: true,
        collapsible_items: false,
    })
    .build()?;

// Список строк (тэги)
let tags = ListParameter::builder()
    .metadata(ParameterMetadata::optional("tags", "Tags")?)
    .item_template(
        TextParameter::required("tag", "Tag")
            .validation(vec![
                ValidationRule::MinLength(1),
                ValidationRule::MaxLength(50),
                ValidationRule::Pattern(r"^[a-zA-Z0-9_-]+$".into()),
            ])
            .build()?
    )
    .min_items(1)
    .max_items(10)
    .build()?;
```

### GroupParameter

Логическая группировка связанных параметров:

```rust
// Группа настроек аутентификации
let auth_group = GroupParameter::builder()
    .metadata(GroupMetadata::builder()
        .key("auth")
        .name("Authentication")
        .description("Configure authentication settings")
        .build()?)
    .parameters(vec![
        Parameter::Text(TextParameter::required("username", "Username").build()?),
        Parameter::Secret(SecretParameter::required("password", "Password").build()?),
    ])
    .ui_options(GroupUIOptions {
        collapsible: true,
        default_expanded: false,
        layout: GroupLayout::Vertical,
    })
    .build()?;
```

### ResourceParameter

Динамическая загрузка опций из внешних API:

```rust
// Список репозиториев GitHub
let github_repos = ResourceParameter::builder()
    .metadata(ParameterMetadata::required("repository", "Repository")?)
    .resource_loader(ResourceLoader {
        url_template: "https://api.github.com/users/{username}/repos".into(),
        method: HttpMethod::GET,
        headers: hashmap! {
            "Accept".into() => "application/vnd.github.v3+json".into(),
            "Authorization".into() => "Bearer {github_token}".into(),
        },
        transform: Some("data.map(repo => ({value: repo.full_name, label: repo.name, description: repo.description}))".into()),
    })
    .depends_on(vec!["username".into(), "github_token".into()])
    .cache_duration(Duration::minutes(5))
    .ui_options(SelectUIOptions {
        searchable: true,
        placeholder: Some("Select repository...".into()),
        loading_message: Some("Loading repositories...".into()),
        ..Default::default()
    })
    .build()?;

// Список таблиц базы данных
let database_tables = ResourceParameter::builder()
    .metadata(ParameterMetadata::required("table", "Table")?)
    .resource_loader(ResourceLoader::custom(Box::new(|context| {
        Box::pin(async move {
            let connection_string = context.get_parameter_value("connection_string")?;
            let db = Database::connect(connection_string).await?;
            
            let tables = db.list_tables().await?;
            let options = tables.into_iter()
                .map(|table| SelectOption {
                    value: table.name.clone(),
                    label: table.name,
                    description: Some(format!("{} rows", table.row_count)),
                    ..Default::default()
                })
                .collect();
                
            Ok(options)
        })
    })))
    .depends_on(vec!["connection_string".into()])
    .validate(|items| {
        if items.is_empty() {
            Err("No tables found. Check your database connection.".to_string())
        } else {
            Ok(())
        }
    })
    .cache_duration(Duration::hours(1))
    .build()?;
```

## Система валидации

### Архитектура валидации

Система валидации использует типы из `nebula-value`:

```rust
use nebula_value::{ValidationRule, ValidationError, Value as ParameterValue};

// ValidationRule импортируется из nebula-value
// Поддерживает все стандартные операции:
// - Equal, NotEqual, GreaterThan, LessThan, Between
// - MinLength, MaxLength, Pattern, StartsWith, EndsWith, Contains  
// - Required, NotEmpty
// - And, Or, Not (логические операции)
// - Custom валидаторы

// ParameterValue это alias для nebula-value::Value
pub type ParameterValue = nebula_value::Value;
```

### Построение валидаций

Используйте builder pattern для читаемых правил валидации:

```rust
let validation = ParameterValidation::builder()
    // Ограничения значений
    .greater_than(0)
    .less_than_or_equal(100)

    // Ограничения строк
    .not_empty()
    .min_length(3)
    .max_length(50)
    .pattern(r"^[a-zA-Z0-9_]+$")

    // Сложные условия
    .all_of(vec![
        ValidationRule::MinLength(8),
        ValidationRule::Or(vec![
            ValidationRule::Contains("@".into()),
            ValidationRule::Contains("+".into()),
        ]),
    ])
    .build();
```

### Встроенные валидаторы

Используйте валидаторы из `nebula-value`:

```rust
use nebula_value::validators;

// Email валидация
let email_validation = validators::email();

// URL валидация  
let url_validation = validators::url();

// Строка с ограничениями
let username_validation = validators::string()
    .min_length(3)
    .max_length(20)
    .pattern(r"^[a-zA-Z0-9_]+$");

// Диапазон чисел
let percentage_validation = validators::number()
    .min(0.0)
    .max(100.0)
    .integer_only(false);
```

### Пользовательская валидация

Используйте `ValidationRule::Custom` из `nebula-value`:

```rust
use nebula_value::{ValidationRule, ParameterValue};

// Inline пользовательская валидация
let validation = ParameterValidation::builder()
    .add_rule(ValidationRule::Custom {
        validator: Arc::new(|value: &ParameterValue| {
            let string_value = value.as_str()
                .ok_or("Expected string value")?;
                
            if string_value.split('@').count() != 2 {
                return Err("Email must contain exactly one @ symbol".into());
            }
            Ok(())
        }),
        message: "Invalid email format".into(),
    })
    .build();

// Переиспользуемый валидатор
pub fn validate_strong_password(value: &ParameterValue) -> Result<(), String> {
    let password = value.as_str().ok_or("Expected string")?;
    
    if password.len() < 8 {
        return Err("Password must be at least 8 characters".into());
    }

    let has_upper = password.chars().any(|c| c.is_uppercase());
    let has_lower = password.chars().any(|c| c.is_lowercase());
    let has_digit = password.chars().any(|c| c.is_digit(10));
    let has_special = password.chars().any(|c| "!@#$%^&*".contains(c));

    if !(has_upper && has_lower && has_digit && has_special) {
        return Err("Password must contain uppercase, lowercase, digit, and special character".into());
    }

    Ok(())
}

// Использование переиспользуемого валидатора
let password_validation = ParameterValidation::builder()
    .add_rule(ValidationRule::Custom {
        validator: Arc::new(validate_strong_password),
        message: "Password does not meet security requirements".into(),
    })
    .build();
```

### ValidationCache для производительности

```rust
pub struct ValidationCache {
    // Кэш результатов валидации по версии параметра
    entries: HashMap<ParameterKey, CacheEntry>,
    max_size: usize,
    current_generation: u32,
}

struct CacheEntry {
    result: Result<(), Vec<ValidationError>>,
    parameter_version: u64,
    generation: u32,
    last_access: Instant,
    cost: u32, // Стоимость вычисления для приоритизации
}

impl ValidationCache {
    /// Получить результат из кэша или вычислить
    pub fn get_or_compute<F>(&mut self, 
        key: &ParameterKey, 
        version: u64, 
        compute_fn: F
    ) -> Result<(), Vec<ValidationError>>
    where F: FnOnce() -> (Result<(), Vec<ValidationError>>, u32)
    {
        // Проверяем кэш
        if let Some(entry) = self.entries.get_mut(key) {
            if entry.parameter_version == version {
                entry.last_access = Instant::now();
                return entry.result.clone();
            }
        }
        
        // Вычисляем новый результат
        let start = Instant::now();
        let (result, cost) = compute_fn();
        let computation_cost = start.elapsed().as_micros() as u32;
        
        // Кэшируем только дорогие вычисления
        if computation_cost > 1000 { // 1ms threshold
            self.insert(key.clone(), CacheEntry {
                result: result.clone(),
                parameter_version: version,
                generation: self.current_generation,
                last_access: Instant::now(),
                cost: computation_cost,
            });
        }
        
        result
    }
    
    /// Автоматическая очистка устаревших записей
    pub fn maintenance(&mut self) {
        if self.entries.len() > self.max_size {
            // Удаляем старые поколения
            let min_generation = self.current_generation.saturating_sub(2);
            self.entries.retain(|_, entry| entry.generation >= min_generation);
            
            // Если всё ещё много, удаляем по LRU
            if self.entries.len() > self.max_size {
                let mut entries: Vec<_> = self.entries.iter().collect();
                entries.sort_by_key(|(_, entry)| entry.last_access);
                
                let remove_count = self.entries.len() - self.max_size;
                for (key, _) in entries.iter().take(remove_count) {
                    self.entries.remove(*key);
                }
            }
        }
    }
}
```

### Ошибки валидации

Используйте типы ошибок из `nebula-value`:

```rust
use nebula_value::{ValidationError, ValidationResult};

match parameter.validate(&value) {
    Ok(()) => println!("Valid!"),
    Err(ValidationError::Multiple(errors)) => {
        for error in errors {
            match error {
                ValidationError::StringTooShort { min, actual } => {
                    println!("Too short: need {} chars, got {}", min, actual);
                }
                ValidationError::PatternMismatch { pattern, value } => {
                    println!("Pattern mismatch: expected '{}', got '{}'", pattern, value);
                }
                ValidationError::NumberOutOfRange { min, max, value } => {
                    println!("Out of range: expected {}-{}, got {}", min, max, value);
                }
                ValidationError::Custom(message) => {
                    println!("Validation failed: {}", message);
                }
            }
        }
    }
    Err(e) => println!("Unexpected error: {}", e),
}
```

## Система отображения

### Трейты отображения

Система отображения использует трейты для гибкости:

```rust
/// Базовый трейт для условного отображения
pub trait Displayable: ParameterType {
    fn display(&self) -> Option<&ParameterDisplay>;
    fn should_display(&self, context: &DisplayContext) -> bool;
    fn display_dependencies(&self) -> Vec<ParameterKey>;
}

/// Расширенные операции отображения
pub trait DisplayableExt: Displayable {
    fn set_display(&mut self, display: Option<ParameterDisplay>);
    fn add_display_condition(&mut self, property: ParameterKey, condition: ValidationRule);
    fn clear_display_conditions(&mut self);
}

/// Реактивное поведение отображения
pub trait DisplayReactive: Displayable {
    fn on_show(&mut self, context: &DisplayContext);
    fn on_hide(&mut self, context: &DisplayContext);
    fn on_display_change(&mut self, old_visible: bool, new_visible: bool, context: &DisplayContext);
}
```

### ParameterDisplay

```rust
pub struct ParameterDisplay {
    /// Условия для показа параметра
    pub show_conditions: Vec<DisplayCondition>,
    
    /// Условия для скрытия параметра  
    pub hide_conditions: Vec<DisplayCondition>,
    
    /// Режим объединения условий
    pub condition_mode: ConditionMode, // All, Any
}

#[derive(Debug, Clone)]
pub struct DisplayCondition {
    pub parameter_key: ParameterKey,
    pub condition: ValidationRule, // Переиспользуем ValidationRule для условий
}

#[derive(Debug, Clone, Copy)]
pub enum ConditionMode {
    All, // Все условия должны быть выполнены
    Any, // Любое условие должно быть выполнено
}

impl ParameterDisplay {
    pub fn builder() -> ParameterDisplayBuilder {
        ParameterDisplayBuilder::new()
    }
    
    pub fn show_when(parameter_key: &str, condition: ValidationRule) -> Self {
        Self::builder()
            .show_when(parameter_key, condition)
            .build()
    }
    
    pub fn hide_when(parameter_key: &str, condition: ValidationRule) -> Self {
        Self::builder()
            .hide_when(parameter_key, condition)
            .build()
    }
}
```

### Контекст отображения

Богатый контекст для оценки отображения:

```rust
pub struct DisplayContext {
    /// Текущие значения параметров
    pub parameter_values: HashMap<ParameterKey, ParameterValue>,

    /// Роль/права пользователя
    pub user_role: Option<String>,

    /// Текущий режим UI
    pub ui_mode: Option<UIMode>,

    /// Дополнительные метаданные
    pub metadata: HashMap<String, ParameterValue>,
}

// Создание контекста с builder pattern
let context = DisplayContext::builder()
    .parameter_values(current_values)
    .user_role("admin".into())
    .ui_mode(UIMode::Advanced)
    .build();
```

### Условия отображения

Используйте `ValidationRule` из `nebula-value` для условий отображения:

```rust
use nebula_value::ValidationRule;

// Простые условия отображения
let display = ParameterDisplay::builder()
    .show_when("mode", ValidationRule::Equal("advanced".into()))
    .hide_when("debug", ValidationRule::Equal(false.into()))
    .build();

// Сложные условия
let display = ParameterDisplay::builder()
    .show_when("level", ValidationRule::GreaterThanOrEqual(5.into()))
    .show_when("role", ValidationRule::Or(vec![
        ValidationRule::Equal("admin".into()),
        ValidationRule::Equal("developer".into()),
    ]))
    .hide_when("environment", ValidationRule::Equal("production".into()))
    .build();

// Использование display chain для читаемости
let display = DisplayChain::show()
    .when("feature_flag", ValidationRule::Equal(true.into()))
    .when("user_level", ValidationRule::GreaterThan(10.into()))
    .when_any_of("department", vec![
        ValidationRule::Equal("engineering".into()),
        ValidationRule::Equal("qa".into()),
    ])
    .build();
```

### DependencyGraph для производительности

```rust
pub struct DependencyGraph {
    // Прямые зависимости: кто зависит от кого
    forward: HashMap<ParameterIndex, Vec<ParameterIndex>>,
    
    // Обратные зависимости: от кого зависит
    backward: HashMap<ParameterIndex, Vec<ParameterIndex>>,
    
    // Предвычисленные транзитивные замыкания для O(1) lookup
    transitive_closure: HashMap<ParameterIndex, BitSet>,
}

impl DependencyGraph {
    /// Построение графа зависимостей из параметров
    pub fn build_from_parameters(parameters: &IndexMap<ParameterKey, Parameter>) -> Self {
        let mut graph = Self::new();
        
        for (param_index, (param_key, param)) in parameters.iter().enumerate() {
            if let Some(display) = &param.display() {
                // Извлекаем зависимости из условий отображения
                for condition in &display.show_conditions {
                    if let Some(dep_index) = parameters.get_index_of(&condition.parameter_key) {
                        graph.add_dependency(dep_index, param_index);
                    }
                }
                
                for condition in &display.hide_conditions {
                    if let Some(dep_index) = parameters.get_index_of(&condition.parameter_key) {
                        graph.add_dependency(dep_index, param_index);
                    }
                }
            }
        }
        
        // Предвычисляем транзитивные замыкания
        graph.compute_transitive_closure();
        graph
    }
    
    /// O(1) получение всех зависимых параметров
    pub fn get_all_dependents(&self, param_index: ParameterIndex) -> &BitSet {
        self.transitive_closure.get(&param_index)
            .map(|set| set)
            .unwrap_or(&BitSet::new())
    }
    
    /// Вычисление минимального набора для валидации
    pub fn compute_affected_set(&self, changed: &BitSet) -> BitSet {
        let mut affected = changed.clone();
        
        for changed_param in changed.iter() {
            let dependents = self.get_all_dependents(changed_param);
            affected.union_with(dependents);
        }
        
        affected
    }
}
```

### Реактивные параметры

Параметры, которые реагируют на изменения видимости:

```rust
impl DisplayReactive for SelectParameter {
    fn on_show(&mut self, context: &DisplayContext) {
        // Загружаем опции при становлении видимыми
        if self.options.is_dynamic() {
            self.refresh_options(context);
        }
    }

    fn on_hide(&mut self, _context: &DisplayContext) {
        // Очищаем кэш для экономии памяти
        self.clear_option_cache();  
    }
}
```

## Коллекции параметров

### Группы параметров

Организация связанных параметров:

```rust
let database_config = ParameterGroup::builder()
    .metadata(
        GroupMetadata::builder()
            .key("database")
            .name("Database Configuration")
            .description("Configure database connection")
            .icon("database")
            .build()
    )
    .parameters(vec![
        Parameter::Text(host),
        Parameter::Number(port),
        Parameter::Text(username),
        Parameter::Secret(password),
        Parameter::Boolean(use_ssl),
    ])
    .layout(GroupLayout::Vertical)
    .collapsible(true)
    .default_expanded(false)
    .display(
        ParameterDisplay::builder()
            .show_when_equals("use_database", true)
            .build()
    )
    .build();
```

### Списки параметров

Динамические коллекции параметров:

```rust
let http_headers = ParameterList::builder()
    .metadata(
        ListMetadata::builder()
            .key("headers")
            .name("HTTP Headers")
            .description("Custom HTTP headers")
            .add_button_text("Add Header")
            .empty_text("No headers configured")
            .build()
    )
    .item_template(
        Parameter::Object(
            ObjectParameter::builder()
                .add_field("name", TextParameter::new(/* ... */))
                .add_field("value", TextParameter::new(/* ... */))
                .build()
        )
    )
    .min_items(0)
    .max_items(20)
    .default_items(vec![
        // Обычные заголовки по умолчанию
    ])
    .build();
```

## Интеграция с Expression System

### Поддержка выражений

Все параметры поддерживают expressions через `MaybeExpression<T>` из `nebula-value`:

```rust
use nebula_value::{Value, Expression, MaybeExpression};

// MaybeExpression<T> может быть либо статическим значением, либо выражением
pub type ParameterValue<T> = MaybeExpression<T>;

// Статическое значение
let static_param = TextParameter::builder()
    .metadata(ParameterMetadata::required("static", "Static Value")?)
    .value(MaybeExpression::Value("static value".to_string()))
    .build()?;

// Expression значение
let dynamic_param = TextParameter::builder()
    .metadata(ParameterMetadata::required("dynamic", "Dynamic Value")?)
    .value(MaybeExpression::Expression(
        Expression::parse("$nodes.previous.result.email")?
    ))
    .build()?;

// Или через удобные методы
let flexible_param = TextParameter::builder()
    .metadata(ParameterMetadata::required("flexible", "Flexible Value")?)
    .static_value("default value") // MaybeExpression::Value
    .or_expression("$nodes.create_user.result.email") // MaybeExpression::Expression
    .build()?;
```

### Builder методы для MaybeExpression

```rust
impl TextParameterBuilder {
    /// Установить статическое значение
    pub fn static_value<S: Into<String>>(mut self, value: S) -> Self {
        self.value = Some(MaybeExpression::Value(value.into()));
        self
    }
    
    /// Установить expression
    pub fn expression_value<S: AsRef<str>>(mut self, expression: S) -> Result<Self, ExpressionError> {
        let expr = Expression::parse(expression.as_ref())?;
        self.value = Some(MaybeExpression::Expression(expr));
        Ok(self)
    }
    
    /// Установить MaybeExpression напрямую
    pub fn maybe_value(mut self, maybe_value: MaybeExpression<String>) -> Self {
        self.value = Some(maybe_value);
        self
    }
    
    /// Аналогично для default значения
    pub fn default_static<S: Into<String>>(mut self, value: S) -> Self {
        self.default = Some(MaybeExpression::Value(value.into()));
        self
    }
    
    pub fn default_expression<S: AsRef<str>>(mut self, expression: S) -> Result<Self, ExpressionError> {
        let expr = Expression::parse(expression.as_ref())?;
        self.default = Some(MaybeExpression::Expression(expr));
        Ok(self)
    }
}

// Аналогично для других типов параметров
impl NumberParameterBuilder {
    pub fn static_value(mut self, value: f64) -> Self {
        self.value = Some(MaybeExpression::Value(value));
        self
    }
    
    pub fn expression_value<S: AsRef<str>>(mut self, expression: S) -> Result<Self, ExpressionError> {
        let expr = Expression::parse(expression.as_ref())?;
        self.value = Some(MaybeExpression::Expression(expr));
        Ok(self)
    }
}

impl BooleanParameterBuilder {
    pub fn static_value(mut self, value: bool) -> Self {
        self.value = Some(MaybeExpression::Value(value));
        self
    }
    
    pub fn expression_value<S: AsRef<str>>(mut self, expression: S) -> Result<Self, ExpressionError> {
        let expr = Expression::parse(expression.as_ref())?;
        self.value = Some(MaybeExpression::Expression(expr));
        Ok(self)
    }
}
```

### Динамические параметры

Параметры с MaybeExpression для динамических значений:

```rust
use nebula_value::{MaybeExpression, Expression};

// URL с динамическими частями
let api_endpoint = TextParameter::builder()
    .metadata(ParameterMetadata::required("endpoint", "API Endpoint")?)
    .value(MaybeExpression::Expression(
        Expression::parse("${workflow.variables.base_url}/users/${nodes.create_user.result.id}")?
    ))
    .build()?;

// Условное значение
let processing_mode = SelectParameter::builder()
    .metadata(ParameterMetadata::required("mode", "Processing Mode")?)
    .value(MaybeExpression::Expression(
        Expression::parse("if $user.premium && $nodes.validation.result.score > 80 then 'premium' else 'standard'")?
    ))
    .options(vec![
        SelectOption::new("standard", "Standard Processing"),
        SelectOption::new("premium", "Premium Processing"),
    ])
    .build()?;

// Удобные builder методы
let timeout = NumberParameter::builder()
    .metadata(ParameterMetadata::required("timeout", "Timeout")?)
    .static_value(30.0) // MaybeExpression::Value(30.0)
    .build()?;

let dynamic_timeout = NumberParameter::builder()
    .metadata(ParameterMetadata::required("dynamic_timeout", "Dynamic Timeout")?)
    .expression_value("$workflow.variables.default_timeout * 2") // MaybeExpression::Expression
    .build()?;
```

## Расширенные возможности

### Кросс-параметрическая валидация

Валидация отношений между параметрами:

```rust
pub struct CrossParameterValidation {
    pub rules: Vec<CrossParameterRule>,
}

impl CrossParameterValidation {
    pub fn validate(&self, values: &HashMap<ParameterKey, ParameterValue>) -> Result<(), ValidationError> {
        for rule in &self.rules {
            rule.validate(values)?;
        }
        Ok(())
    }
}

// Пример: подтверждение пароля
let password_confirmation = CrossParameterRule::builder()
    .parameters(vec!["password", "confirm_password"])
    .condition(CrossParameterCondition::Equal)
    .error_message("Passwords do not match")
    .build();

// Пример: взаимоисключающие опции
let exclusive_rule = CrossParameterRule::builder()
    .parameters(vec!["use_default", "custom_value"])
    .condition(CrossParameterCondition::MutuallyExclusive)
    .error_message("Cannot use both default and custom value")
    .build();
```

### Шаблоны параметров

Переиспользуемые конфигурации параметров:

```rust
pub struct ParameterTemplate {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub parameters: Vec<Parameter>,
    pub variables: HashMap<String, ParameterValue>,
}

// Определение шаблона
let email_template = ParameterTemplate::builder()
    .id("email_input")
    .name("Email Input Template")
    .parameters(vec![
        Parameter::Text(
            TextParameter::builder()
                .metadata(
                    ParameterMetadata::builder()
                        .key("{{key}}")
                        .name("{{name}}")
                        .placeholder("user@example.com")
                        .build()?
                )
                .validation(validators::email())
                .build()?
        ),
    ])
    .build();

// Использование шаблона
let user_email = email_template.instantiate(hashmap! {
    "key" => "user_email",
    "name" => "User Email",
});
```

### Асинхронная валидация

Для валидаций, требующих внешние ресурсы:

```rust
#[async_trait]
pub trait AsyncValidatable: HasValue {
    async fn validate_async(&self, value: &Self::Value) -> Result<(), ValidationError>;
}

// Пример: проверка доступности имени пользователя
impl AsyncValidatable for TextParameter {
    async fn validate_async(&self, value: &String) -> Result<(), ValidationError> {
        if self.metadata.key == "username" {
            let available = check_username_availability(value).await?;
            if !available {
                return Err(ValidationError::Custom(
                    format!("Username '{}' is already taken", value)
                ));
            }
        }

        Ok(())
    }
}
```

### Провайдеры значений

Динамическое вычисление значений:

```rust
#[async_trait]
pub trait ValueProvider: Send + Sync {
    async fn provide_value(&self, context: &ValueProviderContext) -> Result<ParameterValue, Error>;
}

// Пример: значение по умолчанию из окружения
pub struct EnvValueProvider {
    env_var: String,
    fallback: Option<ParameterValue>,
}

#[async_trait]
impl ValueProvider for EnvValueProvider {
    async fn provide_value(&self, _context: &ValueProviderContext) -> Result<ParameterValue, Error> {
        std::env::var(&self.env_var)
            .map(ParameterValue::String)
            .or_else(|_| self.fallback.clone().ok_or(Error::NotFound))
    }
}
```

## Лучшие практики

### 1. Используйте Builders для сложных параметров

Всегда используйте builders для параметров с множественными опциями:

```rust
// Хорошо
let param = TextParameter::builder()
    .metadata(ParameterMetadata::required("key", "Name")?)
    .validation(vec![ValidationRule::MinLength(3)])
    .display(ParameterDisplay::show_when("advanced", ValidationRule::Equal(true.into())))
    .build()?;

// Избегайте ручного конструирования
let param = TextParameter {
    metadata,
    value: None,
    // Легко забыть поля...
};
```

### 2. Валидируйте на правильном уровне

- Используйте UI опции для базовых ограничений (min/max)
- Используйте валидацию для бизнес-правил
- Используйте кросс-параметрическую валидацию для отношений

```rust
// UI ограничение
.ui_options(NumberUIOptions::builder().min(0.0).max(100.0).build())

// Бизнес-правило
.validation(vec![ValidationRule::Custom {
    validator: Arc::new(|value| {
        // Сложная бизнес-логика
        validate_business_rule(value)
    }),
    message: "Business rule violation".into(),
}])
```

### 3. Предоставляйте четкие сообщения об ошибках

Всегда включайте полезные сообщения об ошибках:

```rust
.validation(vec![
    ValidationRule::Custom {
        validator: Arc::new(|value| validate_isbn(value)),
        message: "Please enter a valid ISBN-10 or ISBN-13".into(),
    }
])
```

### 4. Используйте подходящие типы параметров

Выбирайте правильный тип параметра для данных:

- `TextParameter` для свободного текста
- `SecretParameter` для конфиденциальных данных
- `SelectParameter` когда выбор ограничен
- `NumberParameter` для числовых значений с единицами измерения

### 5. Группируйте связанные параметры

Используйте группы параметров для лучшей организации:

```rust
// Хорошо: группировать связанные настройки
let network_group = ParameterGroup::builder()
    .metadata(GroupMetadata::new("network", "Network Settings"))
    .parameters(vec![proxy, timeout, retry_count])
    .build();

// Избегайте: плоский список несвязанных параметров
```

### 6. Учитывайте производительность

Для больших наборов параметров:

- Используйте инкрементальную валидацию `validate_incremental()`
- Реализуйте кэширование для динамических значений
- Очищайте кэши когда параметры скрыты

```rust
impl DisplayReactive for ExpensiveParameter {
    fn on_show(&mut self, context: &DisplayContext) {
        self.load_data_if_needed(context);
    }

    fn on_hide(&mut self, _context: &DisplayContext) {
        self.clear_cache();
    }
}
```

### 7. Документируйте назначение параметров

Всегда включайте четкую документацию:

```rust
ParameterMetadata::builder()
    .key("retry_count")
    .name("Retry Count")
    .description("Number of times to retry failed requests")
    .hint("Set to 0 to disable retries")
    .placeholder("3")
    .build()
```

### 8. Обрабатывайте граничные случаи

Рассматривайте граничные случаи в валидации:

```rust
// Обрабатывайте пустые строки
.validation(vec![
    ValidationRule::NotEmpty,
    ValidationRule::Custom {
        validator: Arc::new(|value| {
            let trimmed = value.as_str().unwrap().trim();
            if trimmed.is_empty() {
                Err("Value cannot be just whitespace".into())
            } else {
                Ok(())
            }
        }),
        message: "Please enter a valid value".into(),
    }
])

// Обрабатывайте точность чисел
.validation(vec![ValidationRule::Custom {
    validator: Arc::new(|value: &f64| {
        if value.fract() != 0.0 && value.fract().abs() < f64::EPSILON {
            // Обрабатывать проблемы точности чисел с плавающей точкой
        }
        Ok(())
    }),
    message: "Invalid number precision".into(),
}])
```

## Справочник API

### Основные трейты

#### ParameterType

```rust
pub trait ParameterType {
    /// Получить тип параметра
    fn kind(&self) -> ParameterKind;

    /// Получить метаданные параметра
    fn metadata(&self) -> &ParameterMetadata;

    /// Получить ключ параметра
    fn key(&self) -> &str;

    /// Получить имя параметра
    fn name(&self) -> &str;

    /// Проверить, является ли обязательным
    fn is_required(&self) -> bool;
}
```

#### HasValue

```rust
pub trait HasValue: ParameterType {
    type Value: Clone + PartialEq + Debug + 'static;

    // Доступ к значению
    fn get_value(&self) -> Option<&Self::Value>;
    fn get_value_mut(&mut self) -> Option<&mut Self::Value>;
    fn has_value(&self) -> bool;

    // Изменение значения
    fn set_value(&mut self, value: Self::Value) -> Result<(), ParameterError>;
    fn set_value_unchecked(&mut self, value: Self::Value) -> Result<(), ParameterError>;
    fn clear_value(&mut self);
    fn take_value(&mut self) -> Option<Self::Value>;
    fn replace_value(&mut self, new: Self::Value) -> Result<Option<Self::Value>, ParameterError>;

    // Обработка значений по умолчанию
    fn default_value(&self) -> Option<&Self::Value>;
    fn is_default(&self) -> bool;
    fn reset_to_default(&mut self) -> Result<(), ParameterError>;

    // Утилиты значений
    fn value_or_default(&self) -> Option<&Self::Value>;
    fn value_or<'a>(&'a self, default: &'a Self::Value) -> &'a Self::Value;
    fn value_or_else<F>(&self, f: F) -> Self::Value where F: FnOnce() -> Self::Value;

    // Преобразования
    fn get_parameter_value(&self) -> Option<ParameterValue>;
    fn set_parameter_value(&mut self, value: ParameterValue) -> Result<(), ParameterError>;
    fn map_value<U, F>(&self, f: F) -> Option<U> where F: FnOnce(&Self::Value) -> U;
}
```

### Типы ошибок

```rust
#[derive(Debug, thiserror::Error)]
pub enum ParameterError {
    #[error("Parameter '{parameter_key}' not found")]
    ParameterNotFound {
        parameter_key: ParameterKey,
    },
    
    #[error("Missing required parameter '{parameter_name}' ({parameter_key})")]
    MissingValue {
        parameter_key: ParameterKey,
        parameter_name: String,
        parameter_type: &'static str,
    },
    
    #[error("Validation failed for parameter '{parameter_name}': {error_message}")]
    ValidationFailed {
        parameter_key: ParameterKey,
        parameter_name: String,
        parameter_type: &'static str,
        rule_name: String,
        value: String,
        error_message: String,
    },
    
    #[error("Type mismatch for parameter '{parameter_key}': expected {expected}, got {actual}")]
    TypeMismatch {
        parameter_key: ParameterKey,
        expected: &'static str,
        actual: &'static str,
    },
    
    #[error("Builder error: {message}")]
    BuilderError {
        message: String,
        parameter_key: Option<ParameterKey>,
    },
    
    #[error("Dependency cycle detected: {cycle:?}")]
    DependencyCycle {
        cycle: Vec<ParameterKey>,
    },
    
    #[error("Multiple validation errors")]
    MultipleErrors(Vec<ParameterError>),
}

impl ParameterError {
    /// Получить ключ параметра если доступен
    pub fn parameter_key(&self) -> Option<&ParameterKey> {
        match self {
            Self::ParameterNotFound { parameter_key } => Some(parameter_key),
            Self::MissingValue { parameter_key, .. } => Some(parameter_key),
            Self::ValidationFailed { parameter_key, .. } => Some(parameter_key),
            Self::TypeMismatch { parameter_key, .. } => Some(parameter_key),
            Self::BuilderError { parameter_key, .. } => parameter_key.as_ref(),
            _ => None,
        }
    }
    
    /// Проверить является ли ошибка критической
    pub fn is_critical(&self) -> bool {
        matches!(self, 
            Self::DependencyCycle { .. } | 
            Self::BuilderError { .. }
        )
    }
}
```

### ValidationResult

```rust
pub struct ValidationResult {
    pub is_valid: bool,
    pub errors: Vec<ParameterError>,
    pub warnings: Vec<ParameterWarning>,
}

#[derive(Debug, Clone)]
pub struct ParameterWarning {
    pub parameter_key: ParameterKey,
    pub message: String,
    pub suggestion: Option<String>,
}

impl ValidationResult {
    pub fn valid() -> Self {
        Self {
            is_valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }
    
    pub fn invalid(errors: Vec<ParameterError>) -> Self {
        Self {
            is_valid: false,
            errors,
            warnings: Vec::new(),
        }
    }
    
    pub fn with_warnings(mut self, warnings: Vec<ParameterWarning>) -> Self {
        self.warnings = warnings;
        self
    }
    
    /// Группировка ошибок по параметрам для UI
    pub fn errors_by_parameter(&self) -> HashMap<ParameterKey, Vec<&ParameterError>> {
        let mut grouped = HashMap::new();
        
        for error in &self.errors {
            if let Some(key) = error.parameter_key() {
                grouped.entry(key.clone()).or_insert_with(Vec::new).push(error);
            }
        }
        
        grouped
    }
}
```

### Улучшенные билдеры

```rust
// Макрос для уменьшения дублирования в билдерах
macro_rules! impl_common_builder_methods {
    ($builder:ident) => {
        impl $builder {
            pub fn required(mut self, required: bool) -> Self {
                self.metadata.required = required;
                self
            }
            
            pub fn description<S: Into<Cow<'static, str>>>(mut self, desc: S) -> Self {
                self.metadata.description = Some(desc.into());
                self
            }
            
            pub fn placeholder<S: Into<Cow<'static, str>>>(mut self, placeholder: S) -> Self {
                self.metadata.placeholder = Some(placeholder.into());
                self
            }
            
            pub fn hint<S: Into<Cow<'static, str>>>(mut self, hint: S) -> Self {
                self.metadata.hint = Some(hint.into());
                self
            }
            
            pub fn group<S: Into<Cow<'static, str>>>(mut self, group: S) -> Self {
                self.metadata.group = Some(group.into());
                self
            }
            
            pub fn order(mut self, order: u32) -> Self {
                self.metadata.order = Some(order);
                self
            }
            
            pub fn validation(mut self, rules: Vec<ValidationRule>) -> Self {
                self.validation = rules;
                self
            }
            
            pub fn display(mut self, display: ParameterDisplay) -> Self {
                self.display = Some(display);
                self
            }
        }
    };
}

// Применение к конкретным билдерам
impl_common_builder_methods!(TextParameterBuilder);
impl_common_builder_methods!(NumberParameterBuilder);
impl_common_builder_methods!(BooleanParameterBuilder);
impl_common_builder_methods!(SelectParameterBuilder);
// ... и так далее для всех типов
```

### Служебные функции

```rust
/// Создать коллекцию параметров из списка параметров
pub fn create_collection(params: Vec<Parameter>) -> Result<ParameterCollection, ParameterError>;

/// Инкрементальная валидация всех параметров в коллекции
pub fn validate_collection_incremental(collection: &mut ParameterCollection) -> Result<ValidationResult, ParameterError>;

/// Извлечь все значения из коллекции
pub fn extract_values(collection: &ParameterCollection) -> HashMap<ParameterKey, ParameterValue>;

/// Применить значения к коллекции с отслеживанием изменений
pub fn apply_values(
    collection: &mut ParameterCollection,
    values: HashMap<ParameterKey, ParameterValue>
) -> Result<(), Vec<ParameterError>>;

/// Найти параметр по ключу в коллекции
pub fn find_parameter<'a>(
    collection: &'a ParameterCollection,
    key: &str
) -> Option<&'a Parameter>;

/// Получить все видимые параметры в коллекции
pub fn visible_parameters<'a>(
    collection: &'a ParameterCollection,
    context: &DisplayContext
) -> Vec<&'a Parameter>;

/// Построить граф зависимостей из коллекции
pub fn build_dependency_graph(collection: &ParameterCollection) -> DependencyGraph;

/// Вычислить параметры, затронутые изменениями
pub fn compute_affected_parameters(
    graph: &DependencyGraph,
    changed_keys: &[ParameterKey]
) -> Vec<ParameterKey>;
```

## Примеры использования

### Полное определение параметров узла

```rust
use nebula_parameter::*;

// Определение параметров для HTTP запроса
pub fn create_http_request_parameters() -> Result<ParameterCollection, ParameterError> {
    let mut collection = ParameterCollection::new();
    
    // URL
    let url = TextParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("url")
                .name("URL")
                .required(true)
                .placeholder("https://api.example.com/endpoint")
                .description("Target URL for the HTTP request")
                .build()?
        )
        .validation(vec![
            ValidationRule::Required,
            ValidationRule::Pattern(r"^https?://.*".into()),
        ])
        .ui_options(TextUIOptions {
            input_type: TextInputType::URL,
            ..Default::default()
        })
        .build()?;

    // Method
    let method = SelectParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("method")
                .name("HTTP Method")
                .required(true)
                .description("HTTP method to use for the request")
                .build()?
        )
        .options(vec![
            SelectOption::new("GET", "GET"),
            SelectOption::new("POST", "POST"),
            SelectOption::new("PUT", "PUT"),
            SelectOption::new("DELETE", "DELETE"),
            SelectOption::new("PATCH", "PATCH"),
        ])
        .default_value("GET")
        .build()?;

    // Headers list
    let headers = ListParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("headers")
                .name("HTTP Headers")
                .description("Custom HTTP headers to include")
                .build()?
        )
        .item_template(
            Parameter::Object(
                ObjectParameter::builder()
                    .metadata(ParameterMetadata::required("header", "Header")?)
                    .add_field("name", TextParameter::required("name", "Header Name").build()?)
                    .add_field("value", TextParameter::required("value", "Header Value").build()?)
                    .build()?
            )
        )
        .min_items(0)
        .max_items(20)
        .build()?;

    // Body (показывается только для POST/PUT/PATCH)
    let body = CodeParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("body")
                .name("Request Body")
                .description("JSON request body")
                .build()?
        )
        .ui_options(CodeUIOptions {
            language: CodeLanguage::JSON,
            ..Default::default()
        })
        .display(
            ParameterDisplay::builder()
                .show_when("method", ValidationRule::Or(vec![
                    ValidationRule::Equal("POST".into()),
                    ValidationRule::Equal("PUT".into()),
                    ValidationRule::Equal("PATCH".into()),
                ]))
                .build()
        )
        .build()?;

    // Timeout
    let timeout = NumberParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("timeout")
                .name("Timeout")
                .description("Request timeout in seconds")
                .build()?
        )
        .default_value(30.0)
        .ui_options(
            NumberUIOptions::builder()
                .min(1.0)
                .max(300.0)
                .step(1.0)
                .format(NumberFormat::Integer)
                .unit("seconds")
                .build()
        )
        .build()?;

    // Retry
    let retry_on_failure = BooleanParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("retry_on_failure")
                .name("Retry on Failure")
                .description("Automatically retry failed requests")
                .build()?
        )
        .default_value(true)
        .build()?;

    // Добавляем параметры в коллекцию
    collection.add_parameter(Parameter::Text(url))?;
    collection.add_parameter(Parameter::Select(method))?;
    collection.add_parameter(Parameter::List(headers))?;
    collection.add_parameter(Parameter::Code(body))?;
    collection.add_parameter(Parameter::Number(timeout))?;
    collection.add_parameter(Parameter::Boolean(retry_on_failure))?;

    Ok(collection)
}

// Использование коллекции
fn example_usage() -> Result<(), ParameterError> {
    let mut collection = create_http_request_parameters()?;
    
    // Установка значений
    collection.set_value(&ParameterKey::new("url"), "https://api.github.com".into())?;
    collection.set_value(&ParameterKey::new("method"), "GET".into())?;
    collection.set_value(&ParameterKey::new("timeout"), 60.0.into())?;
    
    // Инкрементальная валидация (валидирует только изменённые параметры)
    let validation_result = collection.validate_incremental()?;
    
    if !validation_result.is_valid {
        for error in &validation_result.errors {
            eprintln!("Validation error: {}", error);
        }
        return Err(ParameterError::MultipleErrors(validation_result.errors));
    }
    
    // Извлечение значений для выполнения
    let values = collection.extract_values();
    println!("URL: {}", values.get(&ParameterKey::new("url")).unwrap());
    println!("Method: {}", values.get(&ParameterKey::new("method")).unwrap());
    
    Ok(())
}
```

### Database Connection параметры

```rust
fn create_database_connection_parameters() -> Result<ParameterCollection, ParameterError> {
    let mut collection = ParameterCollection::new();
    
    // Database type selector
    let db_type = SelectParameter::required("db_type", "Database Type")
        .options(vec![
            SelectOption::new("postgresql", "PostgreSQL"),
            SelectOption::new("mysql", "MySQL"),
            SelectOption::new("sqlite", "SQLite"),
        ])
        .build()?;
    
    // Host (hidden for SQLite)
    let host = TextParameter::required("host", "Host")
        .default_value("localhost")
        .display(ParameterDisplay::show_when("db_type", 
            ValidationRule::Or(vec![
                ValidationRule::Equal("postgresql".into()),
                ValidationRule::Equal("mysql".into()),
            ])
        ))
        .build()?;
    
    // Port (with different defaults based on DB type)
    let port = NumberParameter::required("port", "Port")
        .ui_options(NumberUIOptions {
            format: NumberFormat::Integer,
            min: Some(1.0),
            max: Some(65535.0),
            ..Default::default()
        })
        .display(ParameterDisplay::show_when("db_type",
            ValidationRule::Or(vec![
                ValidationRule::Equal("postgresql".into()),
                ValidationRule::Equal("mysql".into()),
            ])
        ))
        .build()?;
        
    // Database name/file
    let database = TextParameter::required("database", "Database")
        .description("Database name or file path")
        .build()?;
    
    // Username (not needed for SQLite)
    let username = TextParameter::optional("username", "Username")
        .display(ParameterDisplay::show_when("db_type",
            ValidationRule::Or(vec![
                ValidationRule::Equal("postgresql".into()),
                ValidationRule::Equal("mysql".into()),
            ])
        ))
        .build()?;
    
    // Password (not needed for SQLite)
    let password = SecretParameter::optional("password", "Password")
        .display(ParameterDisplay::show_when("db_type",
            ValidationRule::Or(vec![
                ValidationRule::Equal("postgresql".into()),
                ValidationRule::Equal("mysql".into()),
            ])
        ))
        .build()?;
    
    collection.add_parameter(Parameter::Select(db_type))?;
    collection.add_parameter(Parameter::Text(host))?;
    collection.add_parameter(Parameter::Number(port))?;
    collection.add_parameter(Parameter::Text(database))?;
    collection.add_parameter(Parameter::Text(username))?;
    collection.add_parameter(Parameter::Secret(password))?;
    
    Ok(collection)
}
```

### Использование с Expression System

```rust
fn create_dynamic_parameters() -> Result<ParameterCollection, ParameterError> {
    let mut collection = ParameterCollection::new();
    
    // Статическое значение
    let static_param = TextParameter::required("static_value", "Static Value")
        .static_value("Hello World")
        .build()?;
    
    // Динамическое значение из предыдущего узла
    let dynamic_param = TextParameter::required("dynamic_value", "Dynamic Value")
        .expression_value("$nodes.previous.result.message")?
        .build()?;
    
    // Условное значение
    let conditional_param = TextParameter::required("conditional", "Conditional Value")
        .expression_value("if $nodes.check.result.success then 'Success' else 'Failed'")?
        .build()?;
    
    // Вычисляемое значение
    let computed_param = NumberParameter::required("computed", "Computed Value")
        .expression_value("$nodes.calculate.result.value * 1.2")?
        .build()?;
    
    collection.add_parameter(Parameter::Text(static_param))?;
    collection.add_parameter(Parameter::Text(dynamic_param))?;
    collection.add_parameter(Parameter::Text(conditional_param))?;
    collection.add_parameter(Parameter::Number(computed_param))?;
    
    Ok(collection)
}
```

## Заключение

Система параметров nebula-parameter предоставляет мощную и эффективную основу для создания типобезопасных конфигурационных форм. Ключевые архитектурные улучшения включают:

- **Инкрементальная валидация O(k)** - драматическое улучшение производительности для больших форм
- **Детерминированный порядок параметров** - стабильный UI через IndexMap
- **Граф зависимостей** - эффективное отслеживание связей между параметрами  
- **Кэширование валидации** - автоматическая оптимизация дорогих операций
- **Улучшенные билдеры** - уменьшенное дублирование кода через макросы
- **Структурированные ошибки** - богатый контекст для отладки и UI

Система готова для использования в desktop приложениях с egui и может быть легко расширена для web версий в будущем через простую сериализацию значений параметров.
        