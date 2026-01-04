# Полное решение nebula-parameter

Объединяем анализы ChatGPT (архитектура), Grok (техническая безопасность) и Gemini (практические улучшения) в единое решение.

## Roadmap улучшений

### 🔴 Phase 1: Критические исправления (1-2 месяца)
- Архитектурные проблемы (ChatGPT)
- Безопасность и DoS защита (Grok)
- Thread safety

### 🟡 Phase 2: Функциональные улучшения (2-3 месяца)  
- Расширенная валидация (Gemini)
- Улучшенные типы параметров
- Локализация

### 🟢 Phase 3: Advanced features (3-4 месяца)
- Версионирование и миграции
- Advanced UI features
- Performance optimization

---

## 🔴 Phase 1: Критические улучшения

### 1. Решение проблемы циклических зависимостей (Gemini)

```rust
/// Детектор циклических зависимостей с детальной диагностикой
pub struct CyclicDependencyDetector {
    graph: HashMap<ParameterKey, HashSet<ParameterKey>>,
    visiting: HashSet<ParameterKey>,
    visited: HashSet<ParameterKey>,
}

impl CyclicDependencyDetector {
    pub fn check_for_cycles(&mut self, parameters: &ParameterCollection) -> Result<(), DependencyError> {
        self.visiting.clear();
        self.visited.clear();
        
        // Строим граф зависимостей
        self.build_dependency_graph(parameters);
        
        // Проверяем каждый параметр
        for param_key in self.graph.keys() {
            if !self.visited.contains(param_key) {
                if let Some(cycle) = self.detect_cycle_from(param_key)? {
                    return Err(DependencyError::CyclicDependency {
                        cycle,
                        suggestions: self.suggest_cycle_fixes(&cycle),
                    });
                }
            }
        }
        
        Ok(())
    }
    
    fn detect_cycle_from(&mut self, start: &ParameterKey) -> Result<Option<Vec<ParameterKey>>, DependencyError> {
        let mut path = Vec::new();
        self.dfs_cycle_detection(start, &mut path)
    }
    
    fn dfs_cycle_detection(
        &mut self, 
        current: &ParameterKey, 
        path: &mut Vec<ParameterKey>
    ) -> Result<Option<Vec<ParameterKey>>, DependencyError> {
        if self.visiting.contains(current) {
            // Найден цикл! Извлекаем его из пути
            if let Some(cycle_start) = path.iter().position(|k| k == current) {
                let cycle = path[cycle_start..].to_vec();
                return Ok(Some(cycle));
            }
        }
        
        if self.visited.contains(current) {
            return Ok(None); // Уже проверен
        }
        
        self.visiting.insert(current.clone());
        path.push(current.clone());
        
        // Проверяем все зависимости
        if let Some(dependencies) = self.graph.get(current) {
            for dep in dependencies {
                if let Some(cycle) = self.dfs_cycle_detection(dep, path)? {
                    return Ok(Some(cycle));
                }
            }
        }
        
        path.pop();
        self.visiting.remove(current);
        self.visited.insert(current.clone());
        
        Ok(None)
    }
    
    fn suggest_cycle_fixes(&self, cycle: &[ParameterKey]) -> Vec<String> {
        let mut suggestions = Vec::new();
        
        if cycle.len() == 2 {
            suggestions.push(format!(
                "Mutual dependency between '{}' and '{}'. Consider making one dependency optional or using a common parent parameter.",
                cycle[0], cycle[1]
            ));
        } else {
            suggestions.push(format!(
                "Cycle involves {} parameters. Consider breaking the cycle by:",
                cycle.len()
            ));
            suggestions.push("1. Making some display conditions optional".to_string());
            suggestions.push("2. Using intermediate computed parameters".to_string());
            suggestions.push("3. Redesigning the parameter dependencies".to_string());
        }
        
        suggestions
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DependencyError {
    #[error("Cyclic dependency detected: {cycle:?}. Suggestions: {suggestions:?}")]
    CyclicDependency {
        cycle: Vec<ParameterKey>,
        suggestions: Vec<String>,
    },
}
```

### 2. Расширенные встроенные валидаторы (Gemini)

```rust
/// Библиотека встроенных валидаторов
pub mod validators {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use uuid::Uuid;
    
    /// Email валидация с поддержкой различных RFC
    pub fn email() -> ValidationRule {
        ValidationRule::Custom {
            validator: Arc::new(|value| {
                let email = value.as_str().ok_or("Expected string")?;
                
                // Используем библиотеку email-address для RFC-compliant валидации
                email_address::EmailAddress::parse(email)
                    .map_err(|_| "Invalid email format")?;
                    
                Ok(())
            }),
            message: "Please enter a valid email address".into(),
        }
    }
    
    /// URL валидация с схемой
    pub fn url() -> ValidationRule {
        ValidationRule::Custom {
            validator: Arc::new(|value| {
                let url_str = value.as_str().ok_or("Expected string")?;
                
                url::Url::parse(url_str)
                    .map_err(|_| "Invalid URL format")?;
                    
                Ok(())
            }),
            message: "Please enter a valid URL".into(),
        }
    }
    
    /// IP адрес (IPv4 или IPv6)
    pub fn ip_address() -> ValidationRule {
        ValidationRule::Custom {
            validator: Arc::new(|value| {
                let ip_str = value.as_str().ok_or("Expected string")?;
                
                ip_str.parse::<IpAddr>()
                    .map_err(|_| "Invalid IP address format")?;
                    
                Ok(())
            }),
            message: "Please enter a valid IP address".into(),
        }
    }
    
    /// IPv4 адрес
    pub fn ipv4() -> ValidationRule {
        ValidationRule::Custom {
            validator: Arc::new(|value| {
                let ip_str = value.as_str().ok_or("Expected string")?;
                
                ip_str.parse::<Ipv4Addr>()
                    .map_err(|_| "Invalid IPv4 address format")?;
                    
                Ok(())
            }),
            message: "Please enter a valid IPv4 address".into(),
        }
    }
    
    /// UUID валидация
    pub fn uuid() -> ValidationRule {
        ValidationRule::Custom {
            validator: Arc::new(|value| {
                let uuid_str = value.as_str().ok_or("Expected string")?;
                
                Uuid::parse_str(uuid_str)
                    .map_err(|_| "Invalid UUID format")?;
                    
                Ok(())
            }),
            message: "Please enter a valid UUID".into(),
        }
    }
    
    /// JSON валидация
    pub fn json() -> ValidationRule {
        ValidationRule::Custom {
            validator: Arc::new(|value| {
                let json_str = value.as_str().ok_or("Expected string")?;
                
                serde_json::from_str::<serde_json::Value>(json_str)
                    .map_err(|e| format!("Invalid JSON: {}", e))?;
                    
                Ok(())
            }),
            message: "Please enter valid JSON".into(),
        }
    }
    
    /// Regex валидация (с компиляцией)
    pub fn regex() -> ValidationRule {
        ValidationRule::Custom {
            validator: Arc::new(|value| {
                let regex_str = value.as_str().ok_or("Expected string")?;
                
                regex::Regex::new(regex_str)
                    .map_err(|e| format!("Invalid regex: {}", e))?;
                    
                Ok(())
            }),
            message: "Please enter a valid regular expression".into(),
        }
    }
    
    /// Номер кредитной карты (Luhn algorithm, без сохранения)
    pub fn credit_card() -> ValidationRule {
        ValidationRule::Custom {
            validator: Arc::new(|value| {
                let card_str = value.as_str().ok_or("Expected string")?;
                
                // Удаляем пробелы и дефисы
                let digits: String = card_str.chars()
                    .filter(|c| c.is_ascii_digit())
                    .collect();
                
                if digits.len() < 13 || digits.len() > 19 {
                    return Err("Credit card number must be 13-19 digits".to_string());
                }
                
                // Luhn algorithm
                let mut sum = 0;
                let mut double = false;
                
                for digit_char in digits.chars().rev() {
                    let mut digit = digit_char.to_digit(10).unwrap() as u32;
                    
                    if double {
                        digit *= 2;
                        if digit > 9 {
                            digit -= 9;
                        }
                    }
                    
                    sum += digit;
                    double = !double;
                }
                
                if sum % 10 != 0 {
                    return Err("Invalid credit card number".to_string());
                }
                
                Ok(())
            }),
            message: "Please enter a valid credit card number".into(),
        }
    }
    
    /// Композитные валидаторы
    pub fn password_strong() -> Vec<ValidationRule> {
        vec![
            ValidationRule::MinLength(8),
            ValidationRule::Custom {
                validator: Arc::new(|value| {
                    let password = value.as_str().ok_or("Expected string")?;
                    
                    let has_upper = password.chars().any(|c| c.is_uppercase());
                    let has_lower = password.chars().any(|c| c.is_lowercase());
                    let has_digit = password.chars().any(|c| c.is_ascii_digit());
                    let has_special = password.chars().any(|c| "!@#$%^&*()_+-=[]{}|;:,.<>?".contains(c));
                    
                    let mut missing = Vec::new();
                    if !has_upper { missing.push("uppercase letter"); }
                    if !has_lower { missing.push("lowercase letter"); }
                    if !has_digit { missing.push("digit"); }
                    if !has_special { missing.push("special character"); }
                    
                    if !missing.is_empty() {
                        return Err(format!("Password must contain: {}", missing.join(", ")));
                    }
                    
                    Ok(())
                }),
                message: "Password must contain uppercase, lowercase, digit, and special character".into(),
            }
        ]
    }
}
```

### 3. Улучшенный CodeParameter с LSP поддержкой (Gemini)

```rust
/// Расширенный CodeParameter с продвинутыми возможностями
pub struct AdvancedCodeParameter {
    metadata: ParameterMetadata,
    value: Option<String>,
    default: Option<String>,
    
    // Конфигурация редактора
    editor_config: CodeEditorConfig,
    
    // LSP интеграция
    lsp_client: Option<Arc<dyn LanguageServerClient>>,
    
    // Валидация и форматирование
    syntax_validator: Option<Arc<dyn SyntaxValidator>>,
    formatter: Option<Arc<dyn CodeFormatter>>,
}

#[derive(Debug, Clone)]
pub struct CodeEditorConfig {
    pub language: CodeLanguage,
    pub theme: CodeTheme,
    pub show_line_numbers: bool,
    pub show_minimap: bool,
    pub word_wrap: bool,
    pub auto_format_on_save: bool,
    pub auto_complete: bool,
    pub show_syntax_errors: bool,
    pub show_warnings: bool,
    pub indent_size: u8,
    pub use_tabs: bool,
}

/// Асинхронная валидация синтаксиса
#[async_trait]
pub trait SyntaxValidator: Send + Sync {
    async fn validate_syntax(&self, code: &str) -> Result<SyntaxValidationResult, ValidationError>;
    fn supported_language(&self) -> CodeLanguage;
}

#[derive(Debug)]
pub struct SyntaxValidationResult {
    pub is_valid: bool,
    pub errors: Vec<SyntaxError>,
    pub warnings: Vec<SyntaxWarning>,
}

#[derive(Debug)]
pub struct SyntaxError {
    pub line: u32,
    pub column: u32,
    pub message: String,
    pub severity: ErrorSeverity,
    pub suggestion: Option<String>,
}

#[derive(Debug)]
pub struct SyntaxWarning {
    pub line: u32,
    pub column: u32,
    pub message: String,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum ErrorSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// JavaScript валидатор с использованием swc
pub struct JavaScriptValidator {
    parser: swc_ecma_parser::Parser<swc_ecma_parser::lexer::Lexer>,
}

#[async_trait]
impl SyntaxValidator for JavaScriptValidator {
    async fn validate_syntax(&self, code: &str) -> Result<SyntaxValidationResult, ValidationError> {
        use swc_ecma_parser::{Parser, StringInput, Syntax};
        use swc_ecma_ast::*;
        
        let syntax = Syntax::default();
        let mut parser = Parser::new(
            syntax,
            StringInput::new(code, swc_common::BytePos(0), swc_common::BytePos(code.len() as u32)),
            None,
        );
        
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        
        match parser.parse_script() {
            Ok(script) => {
                // Дополнительные проверки для workflow контекста
                self.validate_workflow_safety(&script, &mut warnings);
            }
            Err(parse_error) => {
                errors.push(SyntaxError {
                    line: 1, // TODO: извлечь реальную позицию из error
                    column: 1,
                    message: format!("Parse error: {}", parse_error),
                    severity: ErrorSeverity::Error,
                    suggestion: None,
                });
            }
        }
        
        Ok(SyntaxValidationResult {
            is_valid: errors.is_empty(),
            errors,
            warnings,
        })
    }
    
    fn supported_language(&self) -> CodeLanguage {
        CodeLanguage::JavaScript
    }
}

impl JavaScriptValidator {
    /// Проверка безопасности для workflow контекста
    fn validate_workflow_safety(&self, _script: &swc_ecma_ast::Script, warnings: &mut Vec<SyntaxWarning>) {
        // Проверяем на потенциально опасные операции
        // - eval() вызовы
        // - setTimeout/setInterval
        // - document/window доступ
        // - XMLHttpRequest/fetch
        
        // Упрощённая реализация - в продакшене нужен AST visitor
        warnings.push(SyntaxWarning {
            line: 1,
            column: 1,
            message: "Consider using expression syntax instead of JavaScript for better security".to_string(),
            suggestion: Some("Use $nodes.previous.result instead of complex JavaScript".to_string()),
        });
    }
}

/// Форматтер кода
#[async_trait]
pub trait CodeFormatter: Send + Sync {
    async fn format_code(&self, code: &str) -> Result<String, FormattingError>;
    fn supported_language(&self) -> CodeLanguage;
}

/// Prettier-based форматтер для JavaScript/TypeScript
pub struct PrettierFormatter {
    config: PrettierConfig,
}

#[derive(Debug, Clone)]
pub struct PrettierConfig {
    pub tab_width: u8,
    pub use_tabs: bool,
    pub semicolons: bool,
    pub single_quotes: bool,
    pub trailing_comma: bool,
}

#[async_trait]
impl CodeFormatter for PrettierFormatter {
    async fn format_code(&self, code: &str) -> Result<String, FormattingError> {
        // В продакшене можно использовать prettier через WASM или вызов CLI
        // Для простоты - базовое форматирование
        
        let mut formatted = String::new();
        let mut indent_level = 0;
        let mut in_string = false;
        let mut escape_next = false;
        
        for ch in code.chars() {
            if escape_next {
                formatted.push(ch);
                escape_next = false;
                continue;
            }
            
            match ch {
                '\\' if in_string => {
                    formatted.push(ch);
                    escape_next = true;
                }
                '"' | '\'' => {
                    formatted.push(ch);
                    in_string = !in_string;
                }
                '{' if !in_string => {
                    formatted.push(ch);
                    if self.config.semicolons {
                        formatted.push('\n');
                        indent_level += 1;
                        self.add_indent(&mut formatted, indent_level);
                    }
                }
                '}' if !in_string => {
                    if formatted.chars().last() != Some('\n') {
                        formatted.push('\n');
                    }
                    indent_level = indent_level.saturating_sub(1);
                    self.add_indent(&mut formatted, indent_level);
                    formatted.push(ch);
                }
                ';' if !in_string && self.config.semicolons => {
                    formatted.push(ch);
                    formatted.push('\n');
                    self.add_indent(&mut formatted, indent_level);
                }
                _ => {
                    formatted.push(ch);
                }
            }
        }
        
        Ok(formatted)
    }
    
    fn supported_language(&self) -> CodeLanguage {
        CodeLanguage::JavaScript
    }
}

impl PrettierFormatter {
    fn add_indent(&self, formatted: &mut String, level: u32) {
        if self.config.use_tabs {
            for _ in 0..level {
                formatted.push('\t');
            }
        } else {
            for _ in 0..(level * self.config.tab_width as u32) {
                formatted.push(' ');
            }
        }
    }
}
```

### 4. Асинхронная валидация файлов (Gemini)

```rust
/// Улучшенный FileParameter с асинхронной валидацией
pub struct AsyncFileParameter {
    metadata: ParameterMetadata,
    value: Option<FileInfo>,
    ui_options: FileUIOptions,
    
    // Асинхронные валидаторы
    async_validators: Vec<Arc<dyn AsyncFileValidator>>,
    
    // Превью генераторы
    preview_generators: HashMap<String, Arc<dyn FilePreviewGenerator>>,
}

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub name: String,
    pub size: u64,
    pub mime_type: String,
    pub last_modified: Option<SystemTime>,
    pub content_hash: Option<String>, // Для дедупликации
    pub preview: Option<FilePreview>,
}

#[derive(Debug, Clone)]
pub enum FilePreview {
    Image { 
        thumbnail_data: Vec<u8>, 
        width: u32, 
        height: u32 
    },
    Text { 
        preview_content: String, 
        total_lines: u32 
    },
    Pdf { 
        page_count: u32, 
        first_page_thumbnail: Option<Vec<u8>> 
    },
    Video { 
        duration_seconds: f64, 
        thumbnail: Option<Vec<u8>>,
        resolution: Option<(u32, u32)>,
    },
}

/// Трейт для асинхронной валидации файлов
#[async_trait]
pub trait AsyncFileValidator: Send + Sync {
    async fn validate_file(&self, file_info: &FileInfo, content: &[u8]) -> Result<(), FileValidationError>;
    fn supported_mime_types(&self) -> &[&str];
    fn max_file_size(&self) -> Option<u64>;
}

/// Валидатор изображений с проверкой размеров
pub struct ImageValidator {
    min_width: Option<u32>,
    min_height: Option<u32>,
    max_width: Option<u32>,
    max_height: Option<u32>,
    allowed_formats: HashSet<String>,
}

#[async_trait]
impl AsyncFileValidator for ImageValidator {
    async fn validate_file(&self, file_info: &FileInfo, content: &[u8]) -> Result<(), FileValidationError> {
        // Используем image crate для анализа
        let img = image::load_from_memory(content)
            .map_err(|e| FileValidationError::InvalidFormat(format!("Invalid image: {}", e)))?;
        
        let (width, height) = img.dimensions();
        
        // Проверяем размеры
        if let Some(min_w) = self.min_width {
            if width < min_w {
                return Err(FileValidationError::ImageTooSmall {
                    actual_width: width,
                    min_width: min_w,
                });
            }
        }
        
        if let Some(max_w) = self.max_width {
            if width > max_w {
                return Err(FileValidationError::ImageTooLarge {
                    actual_width: width,
                    max_width: max_w,
                });
            }
        }
        
        // Аналогично для высоты...
        
        // Проверяем формат
        if !self.allowed_formats.is_empty() {
            let format = image::guess_format(content)
                .map_err(|_| FileValidationError::UnknownFormat)?;
            
            let format_str = format!("{:?}", format).to_lowercase();
            if !self.allowed_formats.contains(&format_str) {
                return Err(FileValidationError::UnsupportedFormat {
                    actual: format_str,
                    allowed: self.allowed_formats.clone(),
                });
            }
        }
        
        Ok(())
    }
    
    fn supported_mime_types(&self) -> &[&str] {
        &["image/jpeg", "image/png", "image/gif", "image/webp", "image/bmp"]
    }
    
    fn max_file_size(&self) -> Option<u64> {
        Some(10 * 1024 * 1024) // 10MB
    }
}

/// Генератор превью файлов
#[async_trait]
pub trait FilePreviewGenerator: Send + Sync {
    async fn generate_preview(&self, file_info: &FileInfo, content: &[u8]) -> Result<FilePreview, PreviewError>;
    fn supported_mime_types(&self) -> &[&str];
}

/// Генератор превью изображений
pub struct ImagePreviewGenerator {
    thumbnail_size: u32,
    quality: u8,
}

#[async_trait]
impl FilePreviewGenerator for ImagePreviewGenerator {
    async fn generate_preview(&self, _file_info: &FileInfo, content: &[u8]) -> Result<FilePreview, PreviewError> {
        let img = image::load_from_memory(content)
            .map_err(|e| PreviewError::ProcessingFailed(e.to_string()))?;
        
        let (original_width, original_height) = img.dimensions();
        
        // Создаём thumbnail с сохранением пропорций
        let thumbnail = img.thumbnail(self.thumbnail_size, self.thumbnail_size);
        
        // Конвертируем в JPEG для экономии места
        let mut thumbnail_data = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut thumbnail_data);
        
        thumbnail.write_to(&mut cursor, image::ImageOutputFormat::Jpeg(self.quality))
            .map_err(|e| PreviewError::EncodingFailed(e.to_string()))?;
        
        Ok(FilePreview::Image {
            thumbnail_data,
            width: original_width,
            height: original_height,
        })
    }
    
    fn supported_mime_types(&self) -> &[&str] {
        &["image/jpeg", "image/png", "image/gif", "image/webp"]
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FileValidationError {
    #[error("Invalid file format: {0}")]
    InvalidFormat(String),
    
    #[error("Image too small: {actual_width}x{actual_height}, minimum: {min_width}x{min_height}")]
    ImageTooSmall {
        actual_width: u32,
        actual_height: u32,
        min_width: u32,
        min_height: u32,
    },
    
    #[error("Image too large: {actual_width}px wide, maximum: {max_width}px")]
    ImageTooLarge {
        actual_width: u32,
        max_width: u32,
    },
    
    #[error("Unknown file format")]
    UnknownFormat,
    
    #[error("Unsupported format '{actual}', allowed: {allowed:?}")]
    UnsupportedFormat {
        actual: String,
        allowed: HashSet<String>,
    },
}
```

---

## 🟡 Phase 2: Функциональные улучшения

### 5. Система локализации (Gemini)

```rust
/// Система локализации для параметров
pub struct LocalizationManager {
    current_locale: String,
    translations: HashMap<String, HashMap<String, String>>, // locale -> key -> translation
    fallback_locale: String,
}

impl LocalizationManager {
    pub fn new(default_locale: &str) -> Self {
        Self {
            current_locale: default_locale.to_string(),
            translations: HashMap::new(),
            fallback_locale: "en".to_string(),
        }
    }
    
    pub fn add_translations(&mut self, locale: &str, translations: HashMap<String, String>) {
        self.translations.insert(locale.to_string(), translations);
    }
    
    pub fn translate(&self, key: &str) -> String {
        // Ищем в текущей локали
        if let Some(locale_translations) = self.translations.get(&self.current_locale) {
            if let Some(translation) = locale_translations.get(key) {
                return translation.clone();
            }
        }
        
        // Fallback на дефолтную локаль
        if let Some(fallback_translations) = self.translations.get(&self.fallback_locale) {
            if let Some(translation) = fallback_translations.get(key) {
                return translation.clone();
            }
        }
        
        // Последний fallback - сам ключ
        key.to_string()
    }
    
    pub fn set_locale(&mut self, locale: &str) {
        self.current_locale = locale.to_string();
    }
}

/// Локализуемые метаданные параметра
#[derive(Debug, Clone)]
pub struct LocalizableParameterMetadata {
    pub key: ParameterKey,
    pub name_key: String,        // Ключ для локализации имени
    pub description_key: Option<String>, // Ключ для описания
    pub placeholder_key: Option<String>, // Ключ для placeholder
    pub hint_key: Option<String>,        // Ключ для подсказки
    pub required: bool,
    pub group_key: Option<String>,       // Ключ для группы
    pub order: Option<u32>,
}

impl LocalizableParameterMetadata {
    /// Создать локализованные метаданные
    pub fn localize(&self, localization: &LocalizationManager) -> ParameterMetadata {
        ParameterMetadata {
            key: self.key.clone(),
            name: localization.translate(&self.name_key).into(),
            description: self.description_key.as_ref()
                .map(|key| localization.translate(key).into()),
            placeholder: self.placeholder_key.as_ref()
                .map(|key| localization.translate(key).into()),
            hint: self.hint_key.as_ref()
                .map(|key| localization.translate(key).into()),
            required: self.required,
            group: self.group_key.as_ref()
                .map(|key| localization.translate(key).into()),
            order: self.order,
        }
    }
}

// Пример локализации
fn setup_localization() -> LocalizationManager {
    let mut loc = LocalizationManager::new("en");
    
    // Английские переводы
    loc.add_translations("en", hashmap! {
        "param.username.name" => "Username".to_string(),
        "param.username.description" => "Your account username".to_string(),
        "param.username.placeholder" => "Enter username".to_string(),
        
        "param.password.name" => "Password".to_string(),
        "param.password.description" => "Your account password".to_string(),
        
        "group.auth.name" => "Authentication".to_string(),
        "group.auth.description" => "Login credentials".to_string(),
    });
    
    // Русские переводы
    loc.add_translations("ru", hashmap! {
        "param.username.name" => "Имя пользователя".to_string(),
        "param.username.description" => "Имя пользователя вашей учётной записи".to_string(),
        "param.username.placeholder" => "Введите имя пользователя".to_string(),
        
        "param.password.name" => "Пароль".to_string(),
        "param.password.description" => "Пароль вашей учётной записи".to_string(),
        
        "group.auth.name" => "Аутентификация".to_string(),
        "group.auth.description" => "Данные для входа".to_string(),
    });
    
    loc
}
```

### 6. Версионирование и миграции параметров (Gemini)

```rust
/// Система версионирования параметров
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterSchema {
    pub version: SchemaVersion,
    pub parameters: Vec<ParameterDefinition>,
    pub migration_path: Option<Vec<SchemaMigration>>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SchemaVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl std::fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Описание миграции между версиями схемы
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaMigration {
    pub from_version: SchemaVersion,
    pub to_version: SchemaVersion,
    pub operations: Vec<MigrationOperation>,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MigrationOperation {
    /// Переименование параметра
    RenameParameter {
        old_key: String,
        new_key: String,
    },
    
    /// Удаление параметра
    RemoveParameter {
        key: String,
        reason: String,
    },
    
    /// Добавление нового параметра с дефолтным значением
    AddParameter {
        key: String,
        parameter_type: ParameterType,
        default_value: Value,
    },
    
    /// Изменение типа параметра с конвертацией
    ChangeParameterType {
        key: String,
        old_type: ParameterType,
        new_type: ParameterType,
        converter: String, // Expression для конвертации
    },
    
    /// Разделение одного параметра на несколько
    SplitParameter {
        source_key: String,
        target_keys: Vec<String>,
        splitter: String, // Expression для разделения
    },
    
    /// Объединение нескольких параметров в один
    MergeParameters {
        source_keys: Vec<String>,
        target_key: String,
        merger: String, // Expression для объединения
    },
    
    /// Изменение валидационных правил
    UpdateValidation {
        key: String,
        old_rules: Vec<ValidationRule>,
        new_rules: Vec<ValidationRule>,
    },
}

/// Движок миграций
pub struct ParameterMigrationEngine {
    expression_engine: Arc<dyn ExpressionEngine>,
}

impl ParameterMigrationEngine {
    pub async fn migrate_values(
        &self,
        values: HashMap<String, Value>,
        migration: &SchemaMigration,
    ) -> Result<HashMap<String, Value>, MigrationError> {
        let mut migrated_values = values.clone();
        
        for operation in &migration.operations {
            migrated_values = self.apply_operation(migrated_values, operation).await?;
        }
        
        Ok(migrated_values)
    }
    
    async fn apply_operation(
        &self,
        mut values: HashMap<String, Value>,
        operation: &MigrationOperation,
    ) -> Result<HashMap<String, Value>, MigrationError> {
        match operation {
            MigrationOperation::RenameParameter { old_key, new_key } => {
                if let Some(value) = values.remove(old_key) {
                    values.insert(new_key.clone(), value);
                }
            }
            
            MigrationOperation::RemoveParameter { key, .. } => {
                values.remove(key);
            }
            
            MigrationOperation::AddParameter { key, default_value, .. } => {
                if !values.contains_key(key) {
                    values.insert(key.clone(), default_value.clone());
                }
            }
            
            MigrationOperation::ChangeParameterType { key, converter, .. } => {
                if let Some(old_value) = values.get(key) {
                    // Создаём контекст для expression
                    let context = MigrationContext {
                        old_value: old_value.clone(),
                        all_values: &values,
                    };
                    
                    let new_value = self.expression_engine
                        .evaluate_with_context(converter, &context)
                        .await?;
                    
                    values.insert(key.clone(), new_value);
                }
            }
            
            MigrationOperation::SplitParameter { source_key, target_keys, splitter } => {
                if let Some(source_value) = values.remove(source_key) {
                    let context = MigrationContext {
                        old_value: source_value,
                        all_values: &values,
                    };
                    
                    let split_result = self.expression_engine
                        .evaluate_with_context(splitter, &context)
                        .await?;
                    
                    // Ожидаем массив значений
                    if let Value::Array(split_values) = split_result {
                        for (i, target_key) in target_keys.iter().enumerate() {
                            if let Some(value) = split_values.get(i) {
                                values.insert(target_key.clone(), value.clone());
                            }
                        }
                    }
                }
            }
            
            MigrationOperation::MergeParameters { source_keys, target_key, merger } => {
                let source_values: Vec<Value> = source_keys.iter()
                    .filter_map(|key| values.remove(key))
                    .collect();
                
                if !source_values.is_empty() {
                    let context = MigrationContext {
                        old_value: Value::Array(source_values),
                        all_values: &values,
                    };
                    
                    let merged_value = self.expression_engine
                        .evaluate_with_context(merger, &context)
                        .await?;
                    
                    values.insert(target_key.clone(), merged_value);
                }
            }
            
            MigrationOperation::UpdateValidation { .. } => {
                // Валидационные правила не влияют на данные
            }
        }
        
        Ok(values)
    }
}

struct MigrationContext<'a> {
    old_value: Value,
    all_values: &'a HashMap<String, Value>,
}

// Пример использования миграций
fn example_migration() -> SchemaMigration {
    SchemaMigration {
        from_version: SchemaVersion { major: 1, minor: 0, patch: 0 },
        to_version: SchemaVersion { major: 1, minor: 1, patch: 0 },
        description: "Split full_name into first_name and last_name".to_string(),
        operations: vec![
            MigrationOperation::SplitParameter {
                source_key: "full_name".to_string(),
                target_keys: vec!["first_name".to_string(), "last_name".to_string()],
                splitter: "old_value.split(' ', 2)".to_string(), // Expression
            },
            MigrationOperation::AddParameter {
                key: "middle_name".to_string(),
                parameter_type: ParameterType::String {
                    min_length: None,
                    max_length: Some(50),
                    pattern: None,
                },
                default_value: Value::String("".to_string()),
            },
        ],
    }
}
```

### 7. Сложные примеры использования (Gemini)

```rust
/// Библиотека сложных примеров и шаблонов
pub mod examples {
    use super::*;
    
    /// Сложная форма с динамическими полями и условной валидацией
    pub fn database_connection_advanced() -> Result<ParameterCollection, ParameterError> {
        let mut collection = ParameterCollection::new();
        
        // Тип базы данных определяет остальные поля
        let db_type = SelectParameter::builder()
            .metadata(ParameterMetadata::required("db_type", "Database Type")?)
            .options(vec![
                SelectOption::builder()
                    .value("postgresql")
                    .label("PostgreSQL")
                    .description("PostgreSQL database")
                    .icon("postgresql")
                    .build(),
                SelectOption::builder()
                    .value("mysql")
                    .label("MySQL")
                    .description("MySQL/MariaDB database")
                    .icon("mysql")
                    .build(),
                SelectOption::builder()
                    .value("sqlite")
                    .label("SQLite")
                    .description("SQLite file database")
                    .icon("sqlite")
                    .build(),
                SelectOption::builder()
                    .value("mongodb")
                    .label("MongoDB")
                    .description("MongoDB NoSQL database")
                    .icon("mongodb")
                    .build(),
            ])
            .build()?;
        
        // Хост - скрыт для SQLite
        let host = TextParameter::builder()
            .metadata(ParameterMetadata::required("host", "Host")?)
            .default_value("localhost".to_string())
            .validation(vec![
                ValidationRule::Custom {
                    validator: Arc::new(|value| {
                        let host = value.as_str().ok_or("Expected string")?;
                        
                        // Валидируем как IP или hostname
                        if host.parse::<std::net::IpAddr>().is_ok() {
                            return Ok(());
                        }
                        
                        // Базовая валидация hostname
                        if host.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-') {
                            Ok(())
                        } else {
                            Err("Invalid hostname or IP address".to_string())
                        }
                    }),
                    message: "Please enter a valid hostname or IP address".into(),
                }
            ])
            .display(ParameterDisplay::builder()
                .show_when(DisplayCondition::field("db_type").not_equals("sqlite"))
                .build())
            .build()?;
        
        // Порт с разными дефолтами для разных БД
        let port = NumberParameter::builder()
            .metadata(ParameterMetadata::required("port", "Port")?)
            .ui_options(NumberUIOptions {
                format: NumberFormat::Integer,
                min: Some(1.0),
                max: Some(65535.0),
                ..Default::default()
            })
            .display(ParameterDisplay::builder()
                .show_when(DisplayCondition::field("db_type").not_equals("sqlite"))
                .build())
            .build()?;
        
        // Сложный объект с условными полями
        let connection_options = ObjectParameter::builder()
            .metadata(ParameterMetadata::optional("options", "Connection Options")?)
            
            // SSL настройки (только для PostgreSQL/MySQL)
            .add_field("ssl_enabled", BooleanParameter::builder()
                .metadata(ParameterMetadata::optional("ssl_enabled", "Enable SSL")?)
                .default_value(false)
                .display(ParameterDisplay::builder()
                    .show_when(DisplayCondition::Or(vec![
                        DisplayCondition::field("db_type").equals("postgresql"),
                        DisplayCondition::field("db_type").equals("mysql"),
                    ]))
                    .build())
                .build()?)
            
            // SSL сертификат (показывается только если SSL включён)
            .add_field("ssl_cert", FileParameter::builder()
                .metadata(ParameterMetadata::optional("ssl_cert", "SSL Certificate")?)
                .ui_options(FileUIOptions {
                    accept: vec!["application/x-x509-ca-cert".into(), ".pem".into(), ".crt".into()],
                    max_size: Some(1024 * 1024), // 1MB
                    preview: false,
                    ..Default::default()
                })
                .display(ParameterDisplay::builder()
                    .show_when(DisplayCondition::And(vec![
                        DisplayCondition::field("ssl_enabled").equals(true),
                        DisplayCondition::Or(vec![
                            DisplayCondition::field("db_type").equals("postgresql"),
                            DisplayCondition::field("db_type").equals("mysql"),
                        ]),
                    ]))
                    .build())
                .build()?)
            
            // MongoDB специфичные опции
            .add_field("replica_set", TextParameter::builder()
                .metadata(ParameterMetadata::optional("replica_set", "Replica Set")?)
                .display(ParameterDisplay::builder()
                    .show_when(DisplayCondition::field("db_type").equals("mongodb"))
                    .build())
                .build()?)
            
            .build()?;
        
        // Список дополнительных параметров подключения
        let extra_params = ListParameter::builder()
            .metadata(ParameterMetadata::optional("extra_params", "Extra Parameters")?)
            .item_template(
                ObjectParameter::builder()
                    .metadata(ParameterMetadata::required("param", "Parameter")?)
                    .add_field("name", TextParameter::builder()
                        .metadata(ParameterMetadata::required("name", "Parameter Name")?)
                        .validation(vec![
                            ValidationRule::Pattern(r"^[a-zA-Z_][a-zA-Z0-9_]*$".into())
                        ])
                        .build()?)
                    .add_field("value", TextParameter::builder()
                        .metadata(ParameterMetadata::required("value", "Parameter Value")?)
                        .build()?)
                    .add_field("description", TextParameter::builder()
                        .metadata(ParameterMetadata::optional("description", "Description")?)
                        .ui_options(TextUIOptions {
                            multiline: false,
                            ..Default::default()
                        })
                        .build()?)
                    .build()?
            )
            .min_items(0)
            .max_items(10)
            .ui_options(ListUIOptions {
                add_button_text: Some("Add Parameter".into()),
                empty_text: Some("No extra parameters".into()),
                reorderable: true,
                ..Default::default()
            })
            .build()?;
        
        // Кросс-параметрическая валидация
        let cross_validation = CrossParameterValidation {
            rules: vec![
                // Порт должен соответствовать типу БД
                CrossParameterRule::builder()
                    .name("port_matches_db_type")
                    .parameters(vec!["db_type", "port"])
                    .validator(Arc::new(|values| {
                        let db_type = values.get("db_type")
                            .and_then(|v| v.as_str()).unwrap_or("");
                        let port = values.get("port")
                            .and_then(|v| v.as_f64()).unwrap_or(0.0) as u16;
                        
                        let expected_port = match db_type {
                            "postgresql" => 5432,
                            "mysql" => 3306,
                            "mongodb" => 27017,
                            _ => return Ok(()), // Для других типов проверка не нужна
                        };
                        
                        if port != 0 && port != expected_port {
                            return Err(format!(
                                "Port {} is unusual for {}. Standard port is {}",
                                port, db_type, expected_port
                            ));
                        }
                        
                        Ok(())
                    }))
                    .severity(ValidationSeverity::Warning) // Предупреждение, не ошибка
                    .build(),
                
                // SSL сертификат должен быть валидным если SSL включён
                CrossParameterRule::builder()
                    .name("ssl_cert_required")
                    .parameters(vec!["ssl_enabled", "ssl_cert"])
                    .validator(Arc::new(|values| {
                        let ssl_enabled = values.get("ssl_enabled")
                            .and_then(|v| v.as_bool()).unwrap_or(false);
                        let ssl_cert = values.get("ssl_cert");
                        
                        if ssl_enabled && ssl_cert.is_none() {
                            return Err("SSL certificate is required when SSL is enabled".to_string());
                        }
                        
                        Ok(())
                    }))
                    .severity(ValidationSeverity::Error)
                    .build(),
            ],
        };
        
        collection.add_parameter(Parameter::Select(db_type))?;
        collection.add_parameter(Parameter::Text(host))?;
        collection.add_parameter(Parameter::Number(port))?;
        collection.add_parameter(Parameter::Object(connection_options))?;
        collection.add_parameter(Parameter::List(extra_params))?;
        collection.set_cross_validation(cross_validation)?;
        
        Ok(collection)
    }
    
    /// Сложная форма с вложенными списками и условными полями
    pub fn api_endpoint_configuration() -> Result<ParameterCollection, ParameterError> {
        let mut collection = ParameterCollection::new();
        
        // Основная конфигурация
        let base_url = TextParameter::url("base_url", "Base URL")?;
        
        let auth_type = SelectParameter::builder()
            .metadata(ParameterMetadata::required("auth_type", "Authentication Type")?)
            .options(vec![
                SelectOption::new("none", "No Authentication"),
                SelectOption::new("basic", "Basic Auth"),
                SelectOption::new("bearer", "Bearer Token"),
                SelectOption::new("oauth2", "OAuth 2.0"),
                SelectOption::new("api_key", "API Key"),
            ])
            .build()?;
        
        // Список endpoints с вложенными объектами
        let endpoints = ListParameter::builder()
            .metadata(ParameterMetadata::required("endpoints", "API Endpoints")?)
            .item_template(
                ObjectParameter::builder()
                    .metadata(ParameterMetadata::required("endpoint", "Endpoint")?)
                    
                    // Базовая информация об endpoint
                    .add_field("name", TextParameter::simple_required("name", "Name")?)
                    .add_field("path", TextParameter::builder()
                        .metadata(ParameterMetadata::required("path", "Path")?)
                        .validation(vec![
                            ValidationRule::Pattern(r"^/.*".into())
                        ])
                        .placeholder("/api/v1/users")
                        .build()?)
                    .add_field("method", SelectParameter::builder()
                        .metadata(ParameterMetadata::required("method", "HTTP Method")?)
                        .options(vec![
                            SelectOption::new("GET", "GET"),
                            SelectOption::new("POST", "POST"),
                            SelectOption::new("PUT", "PUT"),
                            SelectOption::new("DELETE", "DELETE"),
                            SelectOption::new("PATCH", "PATCH"),
                        ])
                        .build()?)
                    
                    // Параметры запроса (показываются для GET)
                    .add_field("query_params", ListParameter::builder()
                        .metadata(ParameterMetadata::optional("query_params", "Query Parameters")?)
                        .item_template(
                            ObjectParameter::builder()
                                .metadata(ParameterMetadata::required("query_param", "Query Parameter")?)
                                .add_field("name", TextParameter::simple_required("name", "Name")?)
                                .add_field("value", TextParameter::simple_required("value", "Value")?)
                                .add_field("required", BooleanParameter::builder()
                                    .metadata(ParameterMetadata::optional("required", "Required")?)
                                    .default_value(false)
                                    .build()?)
                                .build()?
                        )
                        .display(ParameterDisplay::builder()
                            .show_when(DisplayCondition::field("method").equals("GET"))
                            .build())
                        .build()?)
                    
                    // Тело запроса (показывается для POST/PUT/PATCH)
                    .add_field("request_body", CodeParameter::builder()
                        .metadata(ParameterMetadata::optional("request_body", "Request Body")?)
                        .ui_options(CodeUIOptions {
                            language: CodeLanguage::JSON,
                            show_line_numbers: true,
                            auto_format_on_save: true,
                            ..Default::default()
                        })
                        .validation(vec![validators::json()])
                        .display(ParameterDisplay::builder()
                            .show_when(DisplayCondition::Or(vec![
                                DisplayCondition::field("method").equals("POST"),
                                DisplayCondition::field("method").equals("PUT"),
                                DisplayCondition::field("method").equals("PATCH"),
                            ]))
                            .build())
                        .build()?)
                    
                    // Заголовки специфичные для endpoint'а
                    .add_field("headers", ListParameter::builder()
                        .metadata(ParameterMetadata::optional("headers", "Custom Headers")?)
                        .item_template(
                            ObjectParameter::builder()
                                .metadata(ParameterMetadata::required("header", "Header")?)
                                .add_field("name", TextParameter::builder()
                                    .metadata(ParameterMetadata::required("name", "Header Name")?)
                                    .validation(vec![
                                        ValidationRule::Pattern(r"^[a-zA-Z0-9\-]+$".into())
                                    ])
                                    .build()?)
                                .add_field("value", TextParameter::simple_required("value", "Header Value")?)
                                .add_field("condition", SelectParameter::builder()
                                    .metadata(ParameterMetadata::optional("condition", "When to send")?)
                                    .options(vec![
                                        SelectOption::new("always", "Always"),
                                        SelectOption::new("success_only", "On Success Only"),
                                        SelectOption::new("error_only", "On Error Only"),
                                    ])
                                    .default_value("always")
                                    .build()?)
                                .build()?
                        )
                        .build()?)
                    
                    // Retry настройки для каждого endpoint
                    .add_field("retry_config", ObjectParameter::builder()
                        .metadata(ParameterMetadata::optional("retry_config", "Retry Configuration")?)
                        .add_field("enabled", BooleanParameter::builder()
                            .metadata(ParameterMetadata::optional("enabled", "Enable Retry")?)
                            .default_value(true)
                            .build()?)
                        .add_field("max_attempts", NumberParameter::builder()
                            .metadata(ParameterMetadata::optional("max_attempts", "Max Attempts")?)
                            .ui_options(NumberUIOptions {
                                format: NumberFormat::Integer,
                                min: Some(1.0),
                                max: Some(10.0),
                                ..Default::default()
                            })
                            .default_value(3.0)
                            .display(ParameterDisplay::builder()
                                .show_when(DisplayCondition::field("enabled").equals(true))
                                .build())
                            .build()?)
                        .add_field("backoff_ms", NumberParameter::builder()
                            .metadata(ParameterMetadata::optional("backoff_ms", "Backoff (ms)")?)
                            .ui_options(NumberUIOptions {
                                format: NumberFormat::Integer,
                                min: Some(100.0),
                                max: Some(30000.0),
                                unit: Some("ms".into()),
                                ..Default::default()
                            })
                            .default_value(1000.0)
                            .display(ParameterDisplay::builder()
                                .show_when(DisplayCondition::field("enabled").equals(true))
                                .build())
                            .build()?)
                        .build()?)
                    
                    .build()?
            )
            .min_items(1)
            .max_items(20)
            .ui_options(ListUIOptions {
                add_button_text: Some("Add Endpoint".into()),
                empty_text: Some("No endpoints configured".into()),
                reorderable: true,
                collapsible_items: true,
                ..Default::default()
            })
            .build()?;
        
        collection.add_parameter(Parameter::Text(base_url))?;
        collection.add_parameter(Parameter::Select(auth_type))?;
        collection.add_parameter(Parameter::List(endpoints))?;
        
        Ok(collection)
    }
}
```

---

## 🔴 Phase 1: Детальный API справочник (Gemini)

### Comprehensive API Documentation

```rust
/// Полный справочник API для nebula-parameter
/// 
/// # Обзор
/// 
/// Система параметров nebula-parameter предоставляет типобезопасную систему
/// для определения, валидации и управления параметрами workflow узлов.
/// 
/// ## Основные концепции
/// 
/// - **Parameter**: Отдельный параметр с типом, метаданными и валидацией
/// - **ParameterCollection**: Коллекция параметров с зависимостями  
/// - **ValidationRule**: Правило валидации значения параметра
/// - **DisplayCondition**: Условие отображения параметра в UI
/// - **ParameterMetadata**: Метаданные параметра (имя, описание, группа)
/// 
/// # Быстрый старт
/// 
/// ```rust
/// use nebula_parameter::*;
/// 
/// // Создание простого текстового параметра
/// let username = TextParameter::simple_required("username", "Username")?;
/// 
/// // Создание коллекции
/// let mut collection = ParameterCollection::new();
/// collection.add_parameter(Parameter::Text(username))?;
/// 
/// // Установка значения
/// collection.set_value(&ParameterKey::new("username"), "john_doe".into())?;
/// 
/// // Валидация
/// let result = collection.validate()?;
/// assert!(result.is_valid);
/// ```
/// 
/// # Типы параметров
/// 
/// | Тип | Описание | Пример использования |
/// |-----|----------|---------------------|
/// | `TextParameter` | Текстовый ввод | Имена, описания, URLs |
/// | `SecretParameter` | Конфиденциальные данные | Пароли, API ключи |
/// | `NumberParameter` | Числовые значения | Таймауты, проценты, цены |
/// | `BooleanParameter` | Логические значения | Флаги включения/отключения |
/// | `SelectParameter` | Выбор из списка | HTTP методы, режимы работы |
/// | `MultiSelectParameter` | Множественный выбор | Права доступа, теги |
/// | `DateTimeParameter` | Дата и время | Расписание, дедлайны |
/// | `CodeParameter` | Ввод кода | JavaScript, SQL, JSON |
/// | `FileParameter` | Загрузка файлов | Изображения, документы |
/// | `ObjectParameter` | Структурированные данные | Конфигурации, настройки |
/// | `ListParameter` | Динамические списки | HTTP заголовки, параметры |
/// | `ResourceParameter` | Динамическая загрузка | Списки из API |
/// 
/// # Система валидации
/// 
/// ## Встроенные валидаторы
/// 
/// ```rust
/// use nebula_parameter::validators;
/// 
/// // Email валидация
/// let email_rules = vec![validators::email()];
/// 
/// // URL валидация
/// let url_rules = vec![validators::url()];
/// 
/// // Сильный пароль
/// let password_rules = validators::password_strong();
/// 
/// // Композитная валидация
/// let username_rules = vec![
///     ValidationRule::MinLength(3),
///     ValidationRule::MaxLength(20),
///     ValidationRule::Pattern(r"^[a-zA-Z0-9_]+$".into()),
/// ];
/// ```
/// 
/// ## Пользовательская валидация
/// 
/// ```rust
/// let custom_validation = ValidationRule::Custom {
///     validator: Arc::new(|value| {
///         let string_value = value.as_str().ok_or("Expected string")?;
///         
///         if string_value.contains("admin") && !user.is_admin() {
///             return Err("Only admins can use 'admin' in usernames".to_string());
///         }
///         
///         Ok(())
///     }),
///     message: "Invalid username for your access level".into(),
/// };
/// ```
/// 
/// # Условное отображение
/// 
/// ## Простые условия
/// 
/// ```rust
/// let display = ParameterDisplay::builder()
///     .show_when(DisplayCondition::field("mode").equals("advanced"))
///     .hide_when(DisplayCondition::field("environment").equals("production"))
///     .build();
/// ```
/// 
/// ## Сложные условия
/// 
/// ```rust
/// let complex_display = ParameterDisplay::builder()
///     .show_when(DisplayCondition::And(vec![
///         DisplayCondition::field("feature_enabled").equals(true),
///         DisplayCondition::Or(vec![
///             DisplayCondition::field("user_role").equals("admin"),
///             DisplayCondition::field("user_level").greater_than(10),
///         ]),
///     ]))
///     .build();
/// ```
/// 
/// # Производительность
/// 
/// ## Инкрементальная валидация
/// 
/// Система автоматически отслеживает изменения и валидирует только
/// затронутые параметры:
/// 
/// ```rust
/// // Первая валидация - проверяет все параметры
/// let result1 = collection.validate_incremental()?; // ~50ms для 1000 параметров
/// 
/// // Повторная валидация без изменений - мгновенная
/// let result2 = collection.validate_incremental()?; // ~0.1ms
/// 
/// // Изменение одного параметра - валидирует только зависимые
/// collection.set_value(&ParameterKey::new("timeout"), 60.0.into())?;
/// let result3 = collection.validate_incremental()?; // ~1-5ms
/// ```
/// 
/// ## Кэширование валидации
/// 
/// Дорогие валидации автоматически кэшируются:
/// 
/// ```rust
/// // Дорогая валидация (например, проверка API ключа)
/// let expensive_validation = ValidationRule::Custom {
///     validator: Arc::new(|value| {
///         // Эта валидация займёт >1ms и будет закэширована
///         check_api_key_validity(value.as_str().unwrap())
///     }),
///     message: "Invalid API key".into(),
/// };
/// ```
/// 
/// # Безопасность
/// 
/// ## Секретные параметры
/// 
/// ```rust
/// let api_key = SecretParameter::builder()
///     .metadata(ParameterMetadata::required("api_key", "API Key")?)
///     .build()?;
/// 
/// // Безопасный доступ к значению
/// if let Some(secret_value) = api_key.get_value() {
///     let guard = secret_value.access(); // Аудит доступа
///     let key_str = guard.as_str();
///     // Используем ключ
/// } // Автоматическая очистка памяти при drop
/// ```
/// 
/// ## Защита от DoS
/// 
/// Все пользовательские валидаторы защищены от DoS:
/// 
/// ```rust
/// let safe_validator = SafeValidator::new("complex_check".to_string(), |value| {
///     // Этот код будет выполняться с timeout и memory limits
///     perform_complex_validation(value)
/// })
/// .with_timeout(Duration::from_millis(100))
/// .with_memory_limit(10); // 10MB limit
/// ```
/// 
/// # Интеграция с Expression System
/// 
/// ## Статические и динамические значения
/// 
/// ```rust
/// // Статическое значение
/// let static_param = TextParameter::builder()
///     .metadata(ParameterMetadata::required("static", "Static Value")?)
///     .static_value("Hello World")
///     .build()?;
/// 
/// // Динамическое значение из предыдущего узла
/// let dynamic_param = TextParameter::builder()
///     .metadata(ParameterMetadata::required("dynamic", "Dynamic Value")?)
///     .expression_value("$nodes.previous.result.message")?
///     .build()?;
/// 
/// // Условное значение
/// let conditional_param = TextParameter::builder()
///     .metadata(ParameterMetadata::required("conditional", "Conditional Value")?)
///     .expression_value("if $nodes.check.result.success then 'Success' else 'Failed'")?
///     .build()?;
/// ```
/// 
/// # Тестирование
/// 
/// ## Test Utilities
/// 
/// ```rust
/// use nebula_parameter::testing::*;
/// 
/// #[test]
/// fn test_parameter_validation() {
///     let collection = TestParameterBuilder::new()
///         .text_param("username", "john_doe")
///         .number_param("timeout", 30.0)
///         .bool_param("enabled", true)
///         .build_collection()
///         .unwrap();
///     
///     assert_validation_passes(&collection);
/// }
/// 
/// #[test]
/// fn test_conditional_display() {
///     let collection = http_request_fixture();
///     let context = DisplayContext::builder()
///         .parameter_value("mode", "advanced".into())
///         .build();
///     
///     assert_parameter_visible(&collection, "advanced_options", &context);
/// }
/// ```
/// 
/// ## Fixtures
/// 
/// Предопределённые коллекции для тестов:
/// 
/// ```rust
/// // HTTP запрос
/// let http_collection = testing::http_request_fixture();
/// 
/// // Подключение к БД
/// let db_collection = testing::database_connection_fixture();
/// 
/// // Большая коллекция для тестов производительности
/// let large_collection = testing::large_parameter_collection_fixture(1000);
/// ```
/// 
/// # Мониторинг и метрики
/// 
/// ## Prometheus интеграция
/// 
/// ```rust
/// use prometheus::Registry;
/// 
/// let registry = Registry::new();
/// let metrics = ParameterMetrics::new(&registry)?;
/// 
/// // Автоматический сбор метрик
/// let collector = StatisticsCollector::start_background_collection(
///     Arc::new(metrics),
///     Arc::new(parameter_collection),
/// );
/// ```
/// 
/// ## Доступные метрики
/// 
/// - `nebula_parameter_validations_total` - Общее количество валидаций
/// - `nebula_parameter_validation_duration_seconds` - Время валидации
/// - `nebula_parameter_cache_hits_total` - Попадания в кэш
/// - `nebula_parameter_cache_size` - Размер кэша валидации
/// - `nebula_parameter_cache_memory_bytes` - Использование памяти кэшем
/// 
/// # Расширенные возможности
/// 
/// ## Версионирование схем
/// 
/// ```rust
/// let schema_v1 = ParameterSchema {
///     version: SchemaVersion { major: 1, minor: 0, patch: 0 },
///     parameters: vec![/* параметры v1.0 */],
///     migration_path: None,
/// };
/// 
/// let schema_v2 = ParameterSchema {
///     version: SchemaVersion { major: 1, minor: 1, patch: 0 },
///     parameters: vec![/* параметры v1.1 */],
///     migration_path: Some(vec![
///         SchemaMigration {
///             from_version: SchemaVersion { major: 1, minor: 0, patch: 0 },
///             to_version: SchemaVersion { major: 1, minor: 1, patch: 0 },
///             operations: vec![
///                 MigrationOperation::RenameParameter {
///                     old_key: "user_name".to_string(),
///                     new_key: "username".to_string(),
///                 },
///             ],
///             description: "Rename user_name to username".to_string(),
///         }
///     ]),
/// };
/// 
/// // Автоматическая миграция значений
/// let migration_engine = ParameterMigrationEngine::new(expression_engine);
/// let migrated_values = migration_engine.migrate_values(
///     old_values, 
///     &schema_v2.migration_path.unwrap()[0]
/// ).await?;
/// ```
/// 
/// ## Локализация
/// 
/// ```rust
/// // Настройка локализации
/// let mut localization = LocalizationManager::new("en");
/// localization.add_translations("ru", hashmap! {
///     "param.username.name" => "Имя пользователя".to_string(),
///     "param.username.description" => "Ваше имя пользователя".to_string(),
/// });
/// 
/// // Создание локализуемых параметров
/// let localizable_metadata = LocalizableParameterMetadata {
///     key: ParameterKey::new("username"),
///     name_key: "param.username.name".to_string(),
///     description_key: Some("param.username.description".to_string()),
///     required: true,
///     ..Default::default()
/// };
/// 
/// // Получение локализованных метаданных
/// localization.set_locale("ru");
/// let localized = localizable_metadata.localize(&localization);
/// assert_eq!(localized.name.as_ref(), "Имя пользователя");
/// ```
/// 
/// # Troubleshooting
/// 
/// ## Общие проблемы
/// 
/// ### Циклические зависимости
/// 
/// ```rust
/// // ❌ Проблема: параметры зависят друг от друга
/// let param_a = TextParameter::builder()
///     .display(ParameterDisplay::show_when("param_b", condition))
///     .build()?;
/// let param_b = TextParameter::builder()
///     .display(ParameterDisplay::show_when("param_a", condition))
///     .build()?;
/// 
/// // ✅ Решение: используйте общий контролирующий параметр
/// let mode = SelectParameter::builder()
///     .options(vec![SelectOption::new("simple", "Simple"), SelectOption::new("advanced", "Advanced")])
///     .build()?;
/// let param_a = TextParameter::builder()
///     .display(ParameterDisplay::show_when("mode", condition))
///     .build()?;
/// let param_b = TextParameter::builder()
///     .display(ParameterDisplay::show_when("mode", condition))
///     .build()?;
/// ```
/// 
/// ### Медленная валидация
/// 
/// ```rust
/// // ❌ Проблема: дорогая валидация на каждое изменение
/// let expensive_validation = ValidationRule::Custom {
///     validator: Arc::new(|value| {
///         // Медленная операция (например, API вызов)
///         expensive_api_call(value)
///     }),
///     message: "Invalid value".into(),
/// };
/// 
/// // ✅ Решение: используйте AsyncValidatable для дорогих операций
/// #[async_trait]
/// impl AsyncValidatable for MyParameter {
///     async fn validate_async(&self, value: &String) -> Result<(), ValidationError> {
///         // Асинхронная валидация с кэшированием
///         cached_expensive_validation(value).await
///     }
/// }
/// ```
/// 
/// ### Утечки памяти в секретных параметрах
/// 
/// ```rust
/// // ❌ Проблема: секреты остаются в памяти
/// let secret = "my-secret-key".to_string();
/// // secret остаётся в памяти до GC
/// 
/// // ✅ Решение: используйте SecretString
/// let secret = SecretString::new("my-secret-key".to_string());
/// {
///     let guard = secret.access();
///     use_secret(guard.as_str());
/// } // Автоматическая очистка при drop
/// ```
/// 
/// ## Debug и диагностика
/// 
/// ### Включение детального логирования
/// 
/// ```rust
/// // В Cargo.toml
/// tracing = "0.1"
/// tracing-subscriber = "0.3"
/// 
/// // В коде
/// tracing_subscriber::fmt()
///     .with_env_filter("nebula_parameter=debug")
///     .init();
/// 
/// // Теперь все операции с параметрами будут логироваться
/// collection.validate_incremental()?; // Логирует детали валидации
/// ```
/// 
/// ### Анализ производительности
/// 
/// ```rust
/// // Получение статистики
/// let metrics = collection.get_metrics().await;
/// 
/// println!("Cache hit rate: {:.2}%", metrics.validation_cache_stats.hit_rate * 100.0);
/// println!("Average validation time: {:?}", metrics.parameter_collection_stats.average_validation_time);
/// println!("Dependency graph depth: {}", metrics.dependency_graph_stats.max_dependency_depth);
/// 
/// // Анализ "горячих" параметров
/// let hot_parameters = collection.get_most_validated_parameters(10).await;
/// for (param_key, validation_count) in hot_parameters {
///     println!("Parameter '{}' validated {} times", param_key, validation_count);
/// }
/// ```
/// 
/// # Лучшие практики
/// 
/// ## Организация параметров
/// 
/// ```rust
/// // ✅ Группируйте связанные параметры
/// let auth_group = ParameterGroup::builder()
///     .metadata(GroupMetadata::new("auth", "Authentication"))
///     .parameters(vec![username, password, api_key])
///     .collapsible(true)
///     .build();
/// 
/// // ✅ Используйте осмысленные ключи
/// let good_keys = vec!["database_host", "retry_count", "enable_ssl"];
/// 
/// // ❌ Избегайте неясных ключей
/// let bad_keys = vec!["param1", "val", "x"];
/// ```
/// 
/// ## Валидация
/// 
/// ```rust
/// // ✅ Предоставляйте понятные сообщения об ошибках
/// ValidationRule::Custom {
///     validator: Arc::new(validate_credit_card),
///     message: "Please enter a valid credit card number (16 digits)".into(),
/// }
/// 
/// // ❌ Избегайте технических сообщений
/// ValidationRule::Custom {
///     validator: Arc::new(validate_credit_card),
///     message: "Luhn algorithm validation failed".into(), // Непонятно пользователю
/// }
/// ```
/// 
/// ## Производительность
/// 
/// ```rust
/// // ✅ Используйте подходящие типы для данных
/// let port = NumberParameter::builder()
///     .ui_options(NumberUIOptions {
///         format: NumberFormat::Integer, // Не Float для портов
///         min: Some(1.0),
///         max: Some(65535.0),
///     })
///     .build()?;
/// 
/// // ✅ Кэшируйте дорогие ресурсы
/// let countries = ResourceParameter::builder()
///     .cache_duration(Duration::hours(24)) // Страны не меняются часто
///     .build()?;
/// ```
/// 
/// # Feature flags
/// 
/// Система поддерживает модульную сборку:
/// 
/// ```toml
/// [dependencies]
/// nebula-parameter = { version = "1.0", features = ["core"] } # Только core
/// nebula-parameter = { version = "1.0", features = ["ui"] } # С UI возможностями
/// nebula-parameter = { version = "1.0", features = ["metrics"] } # С Prometheus метриками
/// nebula-parameter = { version = "1.0", features = ["full"] } # Все возможности
/// ```
/// 
/// Доступные features:
/// - `core` - Базовая функциональность (включена по умолчанию)
/// - `ui` - UI компоненты и рендеринг
/// - `metrics` - Prometheus метрики  
/// - `localization` - Поддержка локализации
/// - `async-validation` - Асинхронная валидация
/// - `file-preview` - Генерация превью файлов
/// - `code-editor` - Продвинутый редактор кода
/// - `full` - Все возможности
/// 
/// # Миграция с текущей версии
/// 
/// ## Пошаговый план
/// 
/// ### Шаг 1: Обновление зависимостей
/// 
/// ```toml
/// # Добавьте в Cargo.toml
/// [dependencies]
/// nebula-parameter = { version = "2.0", features = ["core", "ui"] }
/// zeroize = "1.7"
/// prometheus = { version = "0.13", optional = true }
/// ```
/// 
/// ### Шаг 2: Обновление кода параметров
/// 
/// ```rust
/// // Старый код
/// let param = TextParameter {
///     metadata: ParameterMetadata { /* ... */ },
///     value: Some("value".to_string()),
///     validation: vec![/* ... */],
///     display: Some(/* ... */),
///     ui_options: TextUIOptions { /* ... */ },
/// };
/// 
/// // Новый код
/// let param = TextParameter::builder()
///     .metadata(ParameterMetadata::required("key", "Name")?)
///     .static_value("value")
///     .validation(vec![/* ... */])
///     .display(ParameterDisplay::builder()
///         .show_when(DisplayCondition::field("mode").equals("advanced"))
///         .build())
///     .build()?;
/// ```
/// 
/// ### Шаг 3: Обновление валидации
/// 
/// ```rust
/// // Старый код
/// collection.validate()?;
/// 
/// // Новый код (с инкрементальной валидацией)
/// collection.validate_incremental()?;
/// ```
/// 
/// ### Шаг 4: Обновление секретных параметров
/// 
/// ```rust
/// // Старый код
/// let secret_param = SecretParameter {
///     value: Some("secret".to_string()), // Небезопасно
///     /* ... */
/// };
/// 
/// // Новый код
/// let secret_param = SecretParameter::builder()
///     .metadata(ParameterMetadata::required("secret", "Secret")?)
///     .build()?;
/// 
/// // Безопасная установка значения
/// secret_param.set_secure_value(SecretString::new("secret".to_string()))?;
/// ```
/// 
/// # Roadmap
/// 
/// ## Version 2.1 (Q2 2024)
/// - GraphQL integration для ResourceParameter
/// - WebAssembly sandbox для custom валидаторов
/// - Real-time collaboration для параметров
/// - Advanced code editor с LSP
/// 
/// ## Version 2.2 (Q3 2024)  
/// - Machine learning для предложения значений параметров
/// - Visual parameter dependency editor
/// - Integration с external secret managers (Vault, AWS Secrets)
/// - Performance profiler для параметров
/// 
/// ## Version 3.0 (Q4 2024)
/// - Breaking changes для упрощения API
/// - Native WASM support
/// - Declarative parameter definition via YAML/JSON
/// - Built-in A/B testing для параметров