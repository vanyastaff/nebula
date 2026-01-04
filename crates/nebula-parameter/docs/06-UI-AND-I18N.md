# UI Metadata and Localization

Guide to UI concerns and the user-managed i18n architecture.

---

## Feature Flags

UI concerns are separated via Cargo features:

```toml
[features]
default = []
ui = []           # UI metadata (placeholders, tooltips, widgets)
i18n = ["ui"]     # Localization support
```

**Benefits:**
- Core library has zero UI dependencies
- Works headless (servers, CLI, batch processing)
- Pay only for what you use
- Smaller binary without UI features

---

## Architecture Overview

```
┌─────────────────────────────────────────┐
│         CORE (always available)         │
│  - key, label, description              │
│  - validation, constraints              │
│  - NO UI dependencies                   │
└─────────────────────────────────────────┘
                  │
                  │ feature = "ui"
                  ▼
┌─────────────────────────────────────────┐
│         UI LAYER (optional)             │
│  - placeholder, tooltip, icon           │
│  - colors, widgets                      │
│  - localization (i18n)                  │
└─────────────────────────────────────────┘
```

---

## UI Metadata Structure

```rust
pub struct Metadata {
    // Core (always available)
    pub key: SmartString<LazyCompact>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub flags: Flags,
    
    // UI (requires feature = "ui")
    #[cfg(feature = "ui")]
    pub ui: Option<UIMetadata>,
}

#[cfg(feature = "ui")]
pub struct UIMetadata {
    pub hints: UIHints,
    pub i18n: Option<I18nMetadata>,
    pub format: Option<DisplayFormat>,
}

pub struct UIHints {
    pub placeholder: Option<String>,
    pub tooltip: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub widget: Option<WidgetHint>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub compact: bool,
}
```

---

## Widget Hints

Suggest how to render a parameter without prescribing exact implementation:

### Text Widgets

```rust
pub enum TextWidget {
    Input,                           // Single-line input
    TextArea { rows: usize },        // Multi-line
    CodeEditor { language: String }, // Syntax highlighting
    Password { reveal: bool },       // Masked with reveal toggle
    RichText,                        // WYSIWYG editor
    Markdown { preview: bool },      // Markdown with preview
    FilePicker { mode: FileMode },   // File/directory picker
    DatePicker,                      // Date selector
    TimePicker,                      // Time selector
    DateTimePicker { timezone: bool }, // Combined date+time
    ColorPicker { alpha: bool },     // Hex color picker
}
```

### Number Widgets

```rust
pub enum NumberWidget {
    Input,                          // Plain number input
    Slider { show_input: bool },    // Slider + optional input
    Stepper { step: f64 },          // +/- buttons
    Knob { arc: f64 },              // Circular knob (for angles)
    Gauge,                          // Visual gauge
}
```

### Select Widgets

```rust
pub enum SelectWidget {
    Dropdown { searchable: bool },
    RadioButtons { inline: bool },
    Tabs,
    SegmentedControl,
    Combobox,                       // Dropdown with text input
    TagInput,                       // Tag-style multi-select
}
```

### Bool Widgets

```rust
pub enum BoolWidget {
    Checkbox,
    Toggle,
    Switch,
}
```

---

## Display Format

Control how values are displayed:

```rust
pub struct DisplayFormat {
    pub number_format: Option<NumberFormat>,
    pub date_format: Option<String>,
    pub custom_formatter: Option<Arc<dyn Fn(&Value) -> String>>,
}

pub struct NumberFormat {
    pub precision: Option<usize>,
    pub use_grouping: bool,        // 1,000,000 vs 1000000
    pub notation: NumberNotation,  // Standard, Scientific, Compact
}
```

---

## User-Managed i18n Architecture

### Philosophy

**Library provides keys, user provides translations.**

This approach:
- Keeps library lightweight (no embedded translations)
- Gives users full control over languages and text
- Avoids binary bloat from unused translations
- Supports any language or custom terminology

```
┌──────────────────────────────────────┐
│  LIBRARY (nebula-parameters)         │
│  - Provides fluent_id for params     │
│  - Provides helper methods           │
│  - NO embedded translations          │
│  - NO i18n dependencies              │
└──────────────────────────────────────┘
              │
              │ fluent_id = "db-host"
              ▼
┌──────────────────────────────────────┐
│  USER APP                            │
│  - Creates locales/ folder           │
│  - Writes FTL files                  │
│  - Setup FluentLoader                │
│  - Full control!                     │
└──────────────────────────────────────┘
```

---

## Recommended: i18n-embed-fl

After evaluating multiple i18n approaches, we recommend **i18n-embed-fl** for its simplicity and ergonomics:

### Why i18n-embed-fl?

| Approach | Pros | Cons |
|----------|------|------|
| **i18n-embed-fl** ✅ | Simple `fl!` macro, global loader, no traits | Requires `once_cell` |
| Custom trait | Flexible, no dependencies | More boilerplate |
| gettext | Industry standard | Complex setup |
| fluent-rs raw | Maximum control | Verbose API |

### Quick Setup

1. **Add dependencies:**

```toml
[dependencies]
i18n-embed = { version = "0.14", features = ["fluent-system"] }
i18n-embed-fl = "0.8"
rust-embed = "8.0"
once_cell = "1.18"
unic-langid = "0.9"
```

2. **Create locales folder:**

```
your-app/
└── locales/
    ├── en-US/
    │   └── app.ftl
    └── ru/
        └── app.ftl
```

3. **Write FTL translations:**

```fluent
# locales/en-US/app.ftl
db-host-label = Database Host
db-host-description = The hostname or IP address of the database server
db-host-placeholder = localhost

db-port-label = Port
db-port-description = Database server port
```

```fluent
# locales/ru/app.ftl
db-host-label = Хост базы данных
db-host-description = Имя хоста или IP-адрес сервера базы данных
db-host-placeholder = localhost

db-port-label = Порт
db-port-description = Порт сервера базы данных
```

4. **Setup global loader:**

```rust
use i18n_embed::{
    fluent::{fluent_language_loader, FluentLanguageLoader},
    LanguageLoader,
};
use i18n_embed_fl::fl;
use rust_embed::RustEmbed;
use once_cell::sync::Lazy;

#[derive(RustEmbed)]
#[folder = "locales/"]
struct Localizations;

pub static LANGUAGE_LOADER: Lazy<FluentLanguageLoader> = Lazy::new(|| {
    let loader = fluent_language_loader!();
    loader.load_languages(&Localizations, &[
        "en-US".parse().unwrap(),
        "ru".parse().unwrap(),
    ]).unwrap();
    loader
});

// Usage is simple:
let label = fl!(LANGUAGE_LOADER, "db-host-label");
// Returns: "Хост базы данных" (if Russian selected)
```

---

## Library Responsibility

Provide `fluent_id` for translation lookup:

```rust
pub struct Metadata {
    pub key: SmartString<LazyCompact>,
    pub label: String,                    // English fallback
    pub fluent_id: Option<String>,        // Translation key
}

impl Metadata {
    /// Get fluent key for label: "db-host" -> "db-host-label"
    pub fn fluent_label_key(&self) -> Option<String> {
        self.fluent_id.as_ref().map(|id| format!("{}-label", id))
    }
    
    /// Get fluent key for description: "db-host" -> "db-host-description"
    pub fn fluent_description_key(&self) -> Option<String> {
        self.fluent_id.as_ref().map(|id| format!("{}-description", id))
    }
    
    /// Get fluent key for placeholder: "db-host" -> "db-host-placeholder"
    pub fn fluent_placeholder_key(&self) -> Option<String> {
        self.fluent_id.as_ref().map(|id| format!("{}-placeholder", id))
    }
}
```

---

## User Helper Functions

```rust
use i18n_embed_fl::fl;
use nebula_parameter::Metadata;

pub fn get_localized_label(meta: &Metadata) -> String {
    if let Some(key) = meta.fluent_label_key() {
        let result = fl!(LANGUAGE_LOADER, &key);
        // fl! returns the key itself if translation not found
        if result != key {
            return result;
        }
    }
    meta.label.to_string()  // Fallback to English
}

pub fn get_localized_description(meta: &Metadata) -> Option<String> {
    if let Some(key) = meta.fluent_description_key() {
        let result = fl!(LANGUAGE_LOADER, &key);
        if result != key {
            return Some(result);
        }
    }
    meta.description.clone()
}

pub fn get_localized_placeholder(meta: &Metadata) -> Option<String> {
    if let Some(key) = meta.fluent_placeholder_key() {
        let result = fl!(LANGUAGE_LOADER, &key);
        if result != key {
            return Some(result);
        }
    }
    meta.ui.as_ref()?.hints.placeholder.clone()
}
```

---

## Usage in Parameters

```rust
TextParameter::builder("host")
    .label("Database Host")      // English fallback
    .fluent_id("db-host")        // User will translate this
    .placeholder("localhost")
    .build()
```

---

## Usage in UI

```rust
fn render_form(schema: &Schema) {
    for param in schema.parameters() {
        let meta = param.metadata();
        
        let label = get_localized_label(meta);
        let description = get_localized_description(meta);
        let placeholder = get_localized_placeholder(meta);
        
        // Render with localized text...
        ui.label(&label);
        if let Some(desc) = description {
            ui.tooltip(&desc);
        }
    }
}
```

---

## Fluent Key Convention

Each parameter with `fluent_id("my-param")` expects these FTL keys:

| Key | Purpose |
|-----|---------|
| `my-param-label` | Parameter label |
| `my-param-description` | Help description |
| `my-param-placeholder` | Input placeholder |
| `my-param-tooltip` | Hover tooltip |

---

## Dynamic Language Switching

```rust
use unic_langid::LanguageIdentifier;

pub fn switch_language(lang: &str) {
    let locale: LanguageIdentifier = lang.parse().unwrap();
    LANGUAGE_LOADER.select(&[locale]).unwrap();
}

// Usage
switch_language("ru");     // Switch to Russian
switch_language("en-US");  // Switch to English
```

---

## Why Fluent?

Fluent handles complex localization that simple key-value systems cannot:

### Russian Plural Forms

```fluent
# Russian has 3 plural forms!
items-count = { $count ->
    [one] { $count } элемент
    [few] { $count } элемента
   *[other] { $count } элементов
}

# Examples:
# 1 → "1 элемент"
# 2 → "2 элемента"
# 5 → "5 элементов"
# 21 → "21 элемент"
# 22 → "22 элемента"
```

### Gender Agreement

```fluent
# German gender-aware
deleted-item = { $gender ->
    [masculine] Der { $name } wurde gelöscht
    [feminine] Die { $name } wurde gelöscht
   *[neuter] Das { $name } wurde gelöscht
}
```

### Contextual Translations

```fluent
# Same word, different context
file-save = Сохранить
file-save-as = Сохранить как...
save-changes = Сохранить изменения?
```

---

## Organization Metadata

Parameters can specify organization hints:

```rust
pub struct ParameterLocation {
    pub page: Option<String>,     // Tab name
    pub group: Option<String>,    // Group within tab
    pub subgroup: Option<String>, // Subgroup
    pub order: i32,               // Display order
}

// Usage
NumberParameter::builder::<f64>("pos_x")
    .label("X Position")
    .page("Transform")
    .group("Position")
    .order(0)
    .build()

NumberParameter::builder::<f64>("pos_y")
    .label("Y Position")
    .page("Transform")
    .group("Position")
    .order(1)
    .build()
```

---

## Complete Example

```rust
// Parameter definition
let schema = Schema::new()
    .with_parameter(
        TextParameter::builder("host")
            .label("Database Host")
            .fluent_id("db-host")
            .page("Connection")
            .group("Server")
            .order(0)
            .required()
            .placeholder("localhost")
            .build()
    )
    .with_parameter(
        NumberParameter::builder::<i64>("port")
            .label("Port")
            .fluent_id("db-port")
            .page("Connection")
            .group("Server")
            .order(1)
            .range(1, 65535)
            .default(5432)
            .suffix(":")
            .build()
    );

// Localization files
// locales/en-US/app.ftl:
// db-host-label = Database Host
// db-host-description = Server hostname or IP
// db-host-placeholder = localhost
// db-port-label = Port
// db-port-description = Server port number

// locales/ru/app.ftl:
// db-host-label = Хост базы данных
// db-host-description = Имя хоста или IP сервера
// db-host-placeholder = localhost
// db-port-label = Порт
// db-port-description = Номер порта сервера
```

---

## Benefits Summary

### For Library
- Lightweight core, no embedded translations
- No i18n dependencies in default build
- No maintenance burden for translations

### For Users
- Full control over languages
- Custom terminology support
- Any language supported
- Update translations independently
- Only include needed languages
- Fluent handles complex grammar rules

---

## Quick Reference

### Widget by Parameter Type

| Type | Suggested Widget |
|------|------------------|
| `TextParameter` | Input, TextArea, Password |
| `NumberParameter` | Input, Slider, Stepper |
| `BoolParameter` | Checkbox, Toggle, Switch |
| `EnumParameter` | Dropdown, RadioButtons, Tabs |
| `VectorParameter` | Vector2D, Vector3D, ColorPicker |
| `TextParameter::file()` | FilePicker |
| `TextParameter::datetime()` | DateTimePicker |

### Suffix by Subtype

| Subtype | Suggested Suffix |
|---------|------------------|
| `Percentage` | % |
| `Angle` | ° |
| `Distance` | m, cm, km |
| `Duration` | s, ms, min |
| `Bytes` | B, KB, MB, GB |
| `Currency` | $, €, ₽ |
