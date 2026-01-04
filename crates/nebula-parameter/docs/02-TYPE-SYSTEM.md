# Type System

Complete reference for all node types and the value system in `paramdef`.

---

## Feature Flags

```toml
[package]
name = "paramdef"

[features]
default = []
display = []
validation = []
serde = ["dep:serde", "dep:serde_json"]
events = ["dep:tokio"]
i18n = ["dep:fluent"]
chrono = ["dep:chrono"]
full = ["display", "validation", "serde", "events", "i18n", "chrono"]
```

| Feature | What It Adds |
|---------|-------------|
| (none) | Core struct fields only |
| `display` | `Displayable` trait, `DisplayConfig` |
| `validation` | `Validatable` trait, `Validator` trait |
| `serde` | Serialize/Deserialize + JSON (From, FromStr, Display) |
| `events` | Event system (tokio channels) |
| `i18n` | Fluent localization support |
| `chrono` | Chrono type conversions |

**Example with features:**

```rust
pub struct Text {
    pub metadata: Metadata,
    pub subtype: TextSubtype,
    pub value: Option<String>,
    
    #[cfg(feature = "display")]
    pub display: Option<DisplayConfig>,
    
    #[cfg(feature = "validation")]
    pub validation: Option<ValidationConfig>,
}

#[cfg(feature = "serde")]
impl Serialize for Text { /* ... */ }

#[cfg(feature = "chrono")]
impl Text {
    pub fn as_naive_date(&self) -> Option<chrono::NaiveDate> { /* ... */ }
}
```

---

## Value Enum

The unified runtime representation for all parameter values:

```rust
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(SmartString<LazyCompact>),
    Array(Arc<[Value]>),
    Object(Arc<HashMap<SmartString, Value>>),
    Binary(Arc<[u8]>),
    Expression {
        template: SmartString<LazyCompact>,
        compiled: Option<Arc<CompiledExpression>>,
    },
}
```

### JSON Conversions (Feature: `serde`)

Idiomatic Rust trait implementations for JSON interop:

```rust
#[cfg(feature = "serde")]
impl From<Value> for serde_json::Value {
    fn from(value: Value) -> Self { /* ... */ }
}

#[cfg(feature = "serde")]
impl From<serde_json::Value> for Value {
    fn from(json: serde_json::Value) -> Self { /* ... */ }
}

#[cfg(feature = "serde")]
impl FromStr for Value {
    type Err = serde_json::Error;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s)
    }
}

#[cfg(feature = "serde")]
impl Display for Value {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            write!(f, "{}", serde_json::to_string_pretty(self).unwrap())
        } else {
            write!(f, "{}", serde_json::to_string(self).unwrap())
        }
    }
}
```

**Usage:**

```rust
// From/Into
let json: serde_json::Value = value.into();
let value: Value = json.into();

// FromStr (parse)
let value: Value = r#"{"name": "test"}"#.parse()?;

// Display
println!("{}", value);    // compact: {"name":"test"}
println!("{:#}", value);  // pretty-printed
```

---

## Node Hierarchy

```
Node (base trait)
│
├── Group: Node                    // Root aggregator
│   └── Group                      // NO own Value, HAS ValueAccess
│
├── Layout: Node                   // UI organization
│   └── Panel                      // NO own Value, HAS ValueAccess
│
├── Decoration: Node               // Display-only
│   └── Notice                     // NO Value, NO children
│
├── Container: Node                // Data structures
│   ├── Object    -> Value::Object // Named fields
│   ├── List      -> Value::Array  // Dynamic array
│   ├── Mode      -> Value::Object // Discriminated union
│   ├── Routing   -> Value::Object // Connection wrapper (workflow)
│   └── Expirable -> Value::Object // TTL wrapper (caching)
│
└── Leaf: Node                     // Terminal values
    ├── Text    -> Value::Text     // HAS own Value, NO children
    ├── Number  -> Value::Int/Float
    ├── Boolean -> Value::Bool
    ├── Vector  -> Value::Array (fixed)
    └── Select  -> Value::Text/Array (unified)

// Total: 1 Group + 1 Layout + 1 Decoration + 5 Container + 5 Leaf = 13 types
```

### Categories Summary

| Category | Types | Own Value | ValueAccess | Can Contain |
|----------|-------|-----------|-------------|-------------|
| Group | Group | NO | YES | Layout, Decoration, Container, Leaf |
| Layout | Panel | NO | YES | Decoration, Container, Leaf |
| Decoration | Notice | NO | NO | nothing |
| Container | Object, List, Mode, Routing, Expirable | YES | YES | Decoration, Container, Leaf |
| Leaf | Text, Number, Boolean, Vector, Select | YES | NO | nothing |

---

## Group Type

### Group

Root aggregator that can contain everything (Layout, Container, Leaf).

```rust
pub struct Group {
    pub metadata: Metadata,
    pub children: Vec<Arc<dyn Node>>,  // Layout | Container | Leaf
    pub layout: GroupLayout,
    pub collapsed: bool,
}
```

**Key Features:**
- Only type that can contain Layout (Panel)
- Provides ValueAccess API to access all values in the tree
- Used as root container for complex configurations

**Example:**
```rust
let config = Group::builder("settings")
    .child(Panel::builder("general")
        .child(Text::new("name"))
        .child(Number::new("port"))
        .build())
    .child(Panel::builder("advanced")
        .child(Object::builder("options")
            .field("timeout", Number::new("timeout"))
            .build())
        .build())
    .build();

// Get all values from entire tree
let values = config.collect_values();

// Set a specific value
config.set_value("port".into(), Value::Int(8080))?;
```

---

## Layout Type

### Panel

UI organization for tabs and sections. Cannot contain other Layout or Group.

```rust
pub struct Panel {
    pub metadata: Metadata,
    pub children: Vec<Arc<dyn Node>>,  // Container | Leaf only
    pub display_type: PanelDisplayType,
    pub selected_tab: Option<usize>,
}
```

**Example:**
```rust
Panel::builder("database")
    .label("Database Settings")
    .child(Text::builder("host").required().build())
    .child(Number::builder("port").default(5432).build())
    .child(Text::builder("database").required().build())
    .build()
```

---

## Decoration Type

### Notice

Display-only messages for info, warnings, errors, or success feedback.

```rust
pub struct Notice {
    pub metadata: Metadata,
    pub notice_type: NoticeType,
    pub dismissible: bool,
}

pub enum NoticeType {
    Info,
    Warning,
    Error,
    Success,
}
```

**Key Features:**
- No Value (does not participate in data collection)
- No children (terminal node)
- Pure UI element for displaying messages
- Can be placed in Group, Layout, or Container

**Example:**
```rust
// Info message
Notice::builder("api_notice")
    .label("API Configuration")
    .notice_type(NoticeType::Info)
    .description("Configure your API settings below. Changes take effect immediately.")
    .build()

// Warning message
Notice::builder("deprecation_warning")
    .label("Deprecated Feature")
    .notice_type(NoticeType::Warning)
    .description("This feature will be removed in v2.0. Please migrate to the new API.")
    .dismissible(true)
    .build()

// Error message
Notice::builder("connection_error")
    .label("Connection Failed")
    .notice_type(NoticeType::Error)
    .description("Unable to connect to the database. Check your credentials.")
    .build()

// Success message
Notice::builder("save_success")
    .label("Settings Saved")
    .notice_type(NoticeType::Success)
    .description("Your configuration has been saved successfully.")
    .dismissible(true)
    .build()
```

**Usage in Forms:**
```rust
Panel::builder("settings")
    .child(Notice::builder("info")
        .notice_type(NoticeType::Info)
        .description("Fill out all required fields.")
        .build())
    .child(Text::builder("name").required().build())
    .child(Text::builder("email").required().build())
    .build()
```

---

## Leaf Types

### Text

Single or multi-line text input with semantic subtypes.

```rust
pub struct Text {
    pub metadata: Metadata,
    pub subtype: TextSubtype,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub pattern: Option<Regex>,
    pub default: Option<String>,
}
```

**Common Subtypes:**
- `Plain` - Generic text
- `Email` - Email address with validation
- `Url` - URL with protocol validation
- `FilePath` - File system path
- `Secret` - Masked input (sensitive)
- `Code(Language)` - Syntax-highlighted editor
- `Json`, `Yaml`, `Xml` - Structured data
- `DateTime`, `Date`, `Time` - Temporal values
- `Expression` - Template expressions

**Example:**
```rust
Text::builder("email")
    .label("Email Address")
    .subtype(TextSubtype::Email)
    .required()
    .placeholder("user@example.com")
    .build()
```

**Output:** `Value::Text("user@example.com")`

---

### Number

Numeric values with type-safe generics.

```rust
pub struct Number<T: Numeric> {
    pub metadata: Metadata,
    pub subtype: NumberSubtype,
    pub unit: NumberUnit,
    pub hard_min: Option<T>,
    pub hard_max: Option<T>,
    pub soft_min: Option<T>,
    pub soft_max: Option<T>,
    pub step: Option<T>,
    pub default: Option<T>,
}
```

**Hard vs Soft Constraints:**
- `hard_min/hard_max` - Validation fails if exceeded
- `soft_min/soft_max` - UI slider range (user can type beyond)

**Example:**
```rust
// Opacity slider 0-100, but validation allows full range
Number::builder::<f64>("opacity")
    .label("Opacity")
    .subtype(NumberSubtype::Factor)
    .hard_min(0.0)
    .hard_max(1.0)
    .soft_min(0.0)
    .soft_max(1.0)
    .step(0.01)
    .default(1.0)
    .build()
```

**Output:** `Value::Float(0.75)`

---

### Boolean

Boolean toggle/checkbox.

```rust
pub struct Boolean {
    pub metadata: Metadata,
    pub default: bool,
}
```

**Example:**
```rust
Boolean::builder("enabled")
    .label("Enable Feature")
    .default(true)
    .build()
```

**Output:** `Value::Bool(true)`

---

### Vector

Fixed-size numeric arrays with const generics.

```rust
pub struct Vector<T: Numeric, const N: usize> {
    pub metadata: Metadata,
    pub subtype: VectorSubtype,
    pub component_units: NumberUnit,
    pub hard_min: Option<[T; N]>,
    pub hard_max: Option<[T; N]>,
    pub soft_min: Option<[T; N]>,
    pub soft_max: Option<[T; N]>,
    pub default: Option<[T; N]>,
}
```

**Common Subtypes:**
- `Position` - 3D position (XYZ)
- `Direction` - Normalized direction vector
- `Scale` - Scale factors
- `Euler` - Rotation angles
- `ColorRgb` / `ColorRgba` - RGB(A) colors

**Example:**
```rust
// Position in 3D space
Vector::<f64, 3>::builder("position")
    .label("Position")
    .subtype(VectorSubtype::Position)
    .component_units(NumberUnit::Length)
    .default([0.0, 0.0, 0.0])
    .build()

// RGBA Color
Vector::<f64, 4>::builder("color")
    .label("Color")
    .subtype(VectorSubtype::ColorRgba)
    .hard_min([0.0, 0.0, 0.0, 0.0])
    .hard_max([1.0, 1.0, 1.0, 1.0])
    .default([1.0, 1.0, 1.0, 1.0])
    .build()
```

**Output:** `Value::Array([Value::Float(1.0), Value::Float(0.5), Value::Float(0.0)])`

---

### Select (Unified)

Unified selection type that combines single/multiple selection with static/dynamic options.

```rust
pub struct Select {
    pub metadata: Metadata,
    pub selection_mode: SelectionMode,
    pub option_source: OptionSource,
    pub value: SelectValue,
    pub searchable: bool,
    pub creatable: bool,
}

pub enum SelectionMode {
    Single,
    Multiple { min: Option<usize>, max: Option<usize> },
}

pub enum OptionSource {
    Static { options: Vec<SelectOption> },
    Dynamic { loader: Arc<dyn OptionLoader>, cache: OptionCache },
}

pub enum SelectValue {
    Single(Option<String>),
    Multiple(Vec<String>),
}

pub struct SelectOption {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub group: Option<String>,
}
```

**Four Use Cases:**

| Selection | Source | Value | Example |
|-----------|--------|-------|---------|
| Single | Static | `Value::Text` | Dropdown, radio |
| Multiple | Static | `Value::Array` | Multi-select, checkboxes |
| Single | Dynamic | `Value::Text` | Resource picker |
| Multiple | Dynamic | `Value::Array` | Multi-resource |

**Examples:**

```rust
// Single static (dropdown)
Select::builder("method")
    .label("HTTP Method")
    .single()
    .static_options(vec![
        SelectOption::new("GET", "GET"),
        SelectOption::new("POST", "POST"),
        SelectOption::new("PUT", "PUT"),
    ])
    .default("GET")
    .build()

// Multiple static (checkboxes)
Select::builder("tags")
    .label("Tags")
    .multiple(1, Some(5))  // min 1, max 5
    .static_options(vec![
        SelectOption::new("urgent", "Urgent"),
        SelectOption::new("bug", "Bug"),
        SelectOption::new("feature", "Feature"),
    ])
    .build()

// Single dynamic (resource picker)
Select::builder("database")
    .label("Database")
    .single()
    .dynamic_options(DatabaseListLoader::new(connection))
    .searchable(true)
    .build()

// Multiple dynamic (multi-resource)
Select::builder("tables")
    .label("Tables")
    .multiple(1, None)
    .dynamic_options(TableListLoader::new(connection))
    .searchable(true)
    .build()
```

**Output:** `Value::Text("POST")` or `Value::Array(["urgent", "bug"])`

---

## Container Types

Containers have their own Value AND can contain children (Container or Leaf).

### Object

Nested structure with named fields.

```rust
pub struct Object {
    pub metadata: Metadata,
    pub fields: Vec<(String, Arc<dyn Node>)>,  // Container | Leaf only
}
```

**Example:**
```rust
Object::builder("address")
    .field("street", Text::builder("street").build())
    .field("city", Text::builder("city").build())
    .field("zip", Text::builder("zip").build())
    .build()
```

**Output:**
```json
{
    "address": {
        "street": "123 Main St",
        "city": "Springfield",
        "zip": "12345"
    }
}
```

---

### List

Dynamic array of items from a template.

```rust
pub struct List {
    pub metadata: Metadata,
    pub item_template: Arc<dyn Node>,  // Container | Leaf only
    pub items: Vec<Arc<dyn Node>>,
    pub min_items: Option<usize>,
    pub max_items: Option<usize>,
    pub unique: bool,
    pub sortable: bool,
}
```

**Example:**
```rust
List::builder("headers")
    .item_template(Object::builder("header")
        .field("name", Text::builder("name").build())
        .field("value", Text::builder("value").build())
        .build())
    .min_items(0)
    .max_items(20)
    .sortable(true)
    .build()
```

**Output:**
```json
{
    "headers": [
        { "name": "Content-Type", "value": "application/json" },
        { "name": "Authorization", "value": "Bearer token" }
    ]
}
```

---

### Mode (Discriminated Union)

**The most important container type!** Enables type-safe branching.

```rust
pub struct Mode {
    pub metadata: Metadata,
    pub variants: Vec<ModeVariant>,
    pub current_variant: Option<String>,
}

pub struct ModeVariant {
    pub key: String,
    pub label: String,
    pub description: Option<String>,
    pub content: Arc<dyn Node>,  // Container | Leaf only
}
```

**Example: Authentication**
```rust
Mode::builder("auth")
    .label("Authentication")
    
    .variant("none", "No Authentication", Object::empty())
    
    .variant("basic", "Basic Auth", Object::builder("credentials")
        .field("username", Text::builder("username").required().build())
        .field("password", Text::builder("password").subtype(TextSubtype::Secret).required().build())
        .build())
    
    .variant("bearer", "Bearer Token", Object::builder("token_config")
        .field("token", Text::builder("token").required().flags(Flags::SENSITIVE).build())
        .build())
    
    .variant("oauth2", "OAuth 2.0", Object::builder("oauth_config")
        .field("client_id", Text::builder("client_id").required().build())
        .field("client_secret", Text::builder("client_secret").flags(Flags::SENSITIVE).build())
        .field("token_url", Text::builder("token_url").subtype(TextSubtype::Url).required().build())
        .build())
    
    .default_variant("none")
    .build()
```

**Output:**
```json
{
    "auth": {
        "mode": "basic",
        "value": {
            "username": "admin",
            "password": "secret"
        }
    }
}
```

**Real-World Use Cases:**
- Database connection: URI vs individual fields
- File input: Upload vs URL vs path
- Scheduling: Cron vs interval vs manual
- Message format: Text vs HTML vs template

---

### Routing (Connection Wrapper)

Wraps a child parameter with workflow connection capabilities.

```rust
pub struct Routing {
    pub metadata: Metadata,
    pub child: Option<Arc<dyn Node>>,  // Container | Leaf only
    pub options: RoutingOptions,
}

pub struct RoutingOptions {
    pub connection_label: Option<String>,
    pub connection_required: bool,
    pub max_connections: Option<usize>,
}
```

**Example:**
```rust
Routing::builder("input_data")
    .connection_label("Data In")
    .connection_required(true)
    .max_connections(1)
    .child(Object::builder("payload")
        .field("id", Text::new("id"))
        .field("value", Number::new("value"))
        .build())
    .build()
```

**Output:**
```json
{
    "input_data": {
        "connected_node_id": "node-123",
        "connection_name": "Main Input",
        "connection_metadata": {},
        "connected_at": "2024-01-15T10:30:00Z"
    }
}
```

**Use Cases:**
- Workflow node inputs/outputs
- Data pipeline connections
- Event routing

---

### Expirable (TTL Wrapper)

Wraps a child parameter with time-to-live expiration logic.

```rust
pub struct Expirable {
    pub metadata: Metadata,
    pub child: Option<Arc<dyn Node>>,  // Container | Leaf only
    pub options: ExpirableOptions,
}

pub struct ExpirableOptions {
    pub ttl: u64,              // Time-to-live in seconds
    pub auto_refresh: bool,
    pub auto_clear_expired: bool,
    pub warning_threshold: Option<u64>,
}
```

**Example:**
```rust
Expirable::builder("cached_token")
    .ttl_hours(1)
    .auto_refresh(true)
    .warning_threshold(300)  // Warn 5 min before expiry
    .child(Text::builder("token")
        .subtype(TextSubtype::Secret)
        .build())
    .build()
```

**Output:**
```json
{
    "cached_token": {
        "value": "eyJhbGciOiJIUzI1NiIs...",
        "expires_at": "2024-01-15T11:30:00Z",
        "created_at": "2024-01-15T10:30:00Z"
    }
}
```

**Use Cases:**
- OAuth tokens with refresh
- Cached API responses
- Session data
- Rate limit windows

---

## Type-Safe Property Keys

Compile-time type safety using phantom types:

```rust
pub struct PropertyKey<T> {
    id: u32,
    _marker: PhantomData<T>,
}

// Define keys
const USERNAME: PropertyKey<String> = PropertyKey::new(1);
const AGE: PropertyKey<i64> = PropertyKey::new(2);

// Type-safe access
let name: String = context.get(USERNAME)?;  // OK
let age: i64 = context.get(AGE)?;           // OK
let wrong: i64 = context.get(USERNAME)?;    // Compile error!
```

---

## Value Mapping Summary

| Node Type | Category | Value Type | JSON Example |
|-----------|----------|------------|--------------|
| Group | Group | - | (collects from children) |
| Panel | Layout | - | (collects from children) |
| Notice | Decoration | - | (no value) |
| Object | Container | `Value::Object` | `{"key": "value"}` |
| List | Container | `Value::Array` | `[{}, {}]` |
| Mode | Container | `Value::Object` | `{"mode": "...", "value": {...}}` |
| Routing | Container | `Value::Object` | `{"connected_node_id": "...", ...}` |
| Expirable | Container | `Value::Object` | `{"value": ..., "expires_at": "..."}` |
| Text | Leaf | `Value::Text` | `"hello"` |
| Number<i64> | Leaf | `Value::Int` | `42` |
| Number<f64> | Leaf | `Value::Float` | `3.14` |
| Boolean | Leaf | `Value::Bool` | `true` |
| Vector<_, 3> | Leaf | `Value::Array` | `[1.0, 2.0, 3.0]` |
| Select (single) | Leaf | `Value::Text` | `"option1"` |
| Select (multiple) | Leaf | `Value::Array` | `["a", "b"]` |

---

## Specializations via Base Types

Instead of creating many specialized node types, Nebula uses **10 core types** combined with **subtypes** and **flags** to cover all use cases. This keeps the API simple while providing rich functionality.

### Text-Based Specializations

| Use Case | Implementation |
|----------|----------------|
| Password | `Text` + `subtype: Secret` + `flags: SENSITIVE \| WRITE_ONLY` |
| Textarea | `Text` + `subtype: MultiLine` |
| Code Editor | `Text` + `subtype: Code(Language::Python)` |
| Date Picker | `Text` + `subtype: Date` |
| Time Picker | `Text` + `subtype: Time` |
| DateTime | `Text` + `subtype: DateTime` |
| Color (Hex) | `Text` + `subtype: HexColor` |
| File Path | `Text` + `subtype: FilePath` |
| Email | `Text` + `subtype: Email` |
| URL | `Text` + `subtype: Url` |
| JSON Editor | `Text` + `subtype: Json` |
| SQL Query | `Text` + `subtype: SqlQuery` |
| Regex | `Text` + `subtype: Regex` |
| UUID | `Text` + `subtype: Uuid` |
| Phone | `Text` + `subtype: PhoneNumber` |
| Credit Card | `Text` + `subtype: CreditCard` + `flags: SENSITIVE` |

**Examples:**

```rust
// Password field
Text::builder("password")
    .subtype(TextSubtype::Secret)
    .flags(Flags::REQUIRED | Flags::SENSITIVE | Flags::WRITE_ONLY)
    .build()

// Multi-line description
Text::builder("description")
    .subtype(TextSubtype::MultiLine)
    .build()

// Code editor with syntax highlighting
Text::builder("script")
    .subtype(TextSubtype::Code(CodeLanguage::Python))
    .build()

// Date picker
Text::builder("birthday")
    .subtype(TextSubtype::Date)
    .placeholder("YYYY-MM-DD")
    .build()

// Color picker (hex)
Text::builder("accent_color")
    .subtype(TextSubtype::HexColor)
    .default("#3B82F6")
    .build()
```

### Number-Based Specializations

| Use Case | Implementation |
|----------|----------------|
| Percentage | `Number<f64>` + `subtype: Percentage` + range 0-100 |
| Currency | `Number<f64>` + `subtype: Currency` |
| Temperature | `Number<f64>` + `subtype: Temperature` + `unit: TemperatureCelsius` |
| Distance | `Number<f64>` + `subtype: Distance` + `unit: LengthMetric` |
| Duration (sec) | `Number<f64>` + `subtype: DurationSeconds` |
| Port Number | `Number<i64>` + `subtype: Port` + range 0-65535 |
| Rating | `Number<i64>` + `subtype: Rating` + range 0-5 |
| Latitude | `Number<f64>` + `subtype: Latitude` + range -90 to 90 |
| Longitude | `Number<f64>` + `subtype: Longitude` + range -180 to 180 |

**Examples:**

```rust
// Percentage slider
Number::builder::<f64>("opacity")
    .subtype(NumberSubtype::Percentage)
    .hard_min(0.0)
    .hard_max(100.0)
    .default(100.0)
    .build()

// Temperature with unit conversion
Number::builder::<f64>("temperature")
    .subtype(NumberSubtype::Temperature)
    .unit(NumberUnit::TemperatureCelsius)
    .build()

// Network port
Number::builder::<i64>("port")
    .subtype(NumberSubtype::Port)
    .hard_min(0)
    .hard_max(65535)
    .default(8080)
    .build()

// Star rating
Number::builder::<i64>("rating")
    .subtype(NumberSubtype::Rating)
    .hard_min(0)
    .hard_max(5)
    .build()
```

### Vector-Based Specializations

| Use Case | Implementation |
|----------|----------------|
| Color RGB | `Vector<f64, 3>` + `subtype: ColorRgb` |
| Color RGBA | `Vector<f64, 4>` + `subtype: ColorRgba` |
| Color HSV | `Vector<f64, 3>` + `subtype: ColorHsv` |
| Position 2D | `Vector<f64, 2>` + `subtype: Position2D` |
| Position 3D | `Vector<f64, 3>` + `subtype: Position` |
| Size 2D | `Vector<f64, 2>` + `subtype: Size2D` |
| Euler Angles | `Vector<f64, 3>` + `subtype: EulerAngles` |
| Quaternion | `Vector<f64, 4>` + `subtype: Quaternion` |

**Examples:**

```rust
// RGBA color picker
Vector::<f64, 4>::builder("background_color")
    .subtype(VectorSubtype::ColorRgba)
    .hard_min([0.0, 0.0, 0.0, 0.0])
    .hard_max([1.0, 1.0, 1.0, 1.0])
    .default([1.0, 1.0, 1.0, 1.0])
    .build()

// 3D position with unit
Vector::<f64, 3>::builder("position")
    .subtype(VectorSubtype::Position)
    .component_units(NumberUnit::LengthMetric)
    .build()

// Rotation angles
Vector::<f64, 3>::builder("rotation")
    .subtype(VectorSubtype::EulerAngles)
    .component_units(NumberUnit::RotationDegrees)
    .build()
```

### Flag-Based Behavior

| Use Case | Implementation |
|----------|----------------|
| Hidden field | Any node + `flags: HIDDEN` |
| Read-only display | Any node + `flags: READONLY` |
| Disabled field | Any node + `flags: DISABLED` |
| Sensitive data | Any node + `flags: SENSITIVE` |
| Don't save | Any node + `flags: SKIP_SAVE` |
| Runtime computed | Any node + `flags: RUNTIME \| READONLY \| SKIP_SAVE` |
| Animatable | Any node + `flags: ANIMATABLE \| REALTIME` |
| Expression support | Any node + `flags: EXPRESSION` |
| Network sync | Any node + `flags: REPLICATED` |
| Deprecated | Any node + `flags: DEPRECATED` |

**Examples:**

```rust
// Hidden session ID
Text::builder("session_id")
    .flags(Flags::HIDDEN | Flags::SKIP_SAVE)
    .build()

// Computed display value (read-only, not saved)
Text::builder("full_name")
    .flags(Flags::computed())  // RUNTIME | READONLY | SKIP_SAVE
    .build()

// Animatable property for 3D editor
Number::builder::<f64>("scale")
    .flags(Flags::animatable())  // ANIMATABLE | REALTIME
    .build()

// API key (sensitive, write-only)
Text::builder("api_key")
    .flags(Flags::sensitive())  // SENSITIVE | WRITE_ONLY | SKIP_SAVE
    .build()

// Deprecated setting with warning
Text::builder("old_setting")
    .flags(Flags::DEPRECATED | Flags::READONLY)
    .description("Deprecated: use 'new_setting' instead")
    .build()
```

### SelectOption for Choices

For Select with rich metadata:

```rust
pub struct SelectOption {
    pub key: String,           // Unique identifier
    pub name: String,          // Display name
    pub value: String,         // Actual value
    pub description: Option<String>,  // Tooltip/help text
    pub icon: Option<String>,  // Icon name
    pub disabled: Option<bool>,// Disabled state
    pub group: Option<String>, // Group for categorization
    pub color: Option<String>, // Color hint
    pub subtitle: Option<String>, // Secondary text
}

// Example with rich options
Select::builder("log_level")
    .single()
    .static_options(vec![
        SelectOption::builder()
            .key("debug")
            .name("Debug")
            .value("debug")
            .description("Verbose logging for development")
            .icon("bug")
            .color("#6B7280")
            .build(),
        SelectOption::builder()
            .key("info")
            .name("Info")
            .value("info")
            .description("Standard informational messages")
            .icon("info")
            .color("#3B82F6")
            .build(),
        SelectOption::builder()
            .key("error")
            .name("Error")
            .value("error")
            .description("Only error messages")
            .icon("alert")
            .color("#EF4444")
            .build(),
    ])
    .default("info")
    .build()
```

### Dynamic Options with Pagination

For large or dynamic option lists:

```rust
pub struct OptionLoadContext<'a> {
    pub parameters: &'a HashMap<String, Value>,  // Current values
    pub search: Option<String>,                   // Search query
    pub pagination: Option<Pagination>,           // Page info
}

pub struct Pagination {
    pub page: usize,
    pub page_size: usize,
    pub cursor: Option<String>,  // For cursor-based pagination
}

pub struct OptionsResponse {
    pub options: Vec<SelectOption>,
    pub total: Option<usize>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

// Example: Load databases from API
Select::builder("database")
    .single()
    .dynamic_options(|ctx| async {
        let dbs = api.list_databases(ctx.search, ctx.pagination).await?;
        Ok(OptionsResponse {
            options: dbs.into_iter().map(|db| SelectOption::new(
                &db.id, &db.name, &db.id
            )).collect(),
            total: Some(100),
            has_more: true,
            next_cursor: Some("cursor123".into()),
        })
    })
    .searchable(true)
    .build()
```

---

## Conditional Visibility (Feature: `display`)

> **Requires:** `features = ["display"]`

**ALL 13 node types** support conditional visibility via the `Displayable` trait.

### Basic Usage

Every node type has an optional `display` field:

```rust
pub struct Text {
    pub metadata: Metadata,
    pub display: Option<DisplayConfig>,  // Conditional visibility
    // ... other fields
}

pub struct Panel {
    pub metadata: Metadata,
    pub display: Option<DisplayConfig>,  // Conditional visibility
    // ... other fields
}

pub struct Notice {
    pub metadata: Metadata,
    pub display: Option<DisplayConfig>,  // Conditional visibility
    // ... other fields
}
```

### Examples by Category

```rust
// Group: hide entire settings in simple mode
Group::builder("advanced_settings")
    .display(DisplayConfig::new()
        .hide_when_equals("mode", Value::text("simple")))
    .build()

// Panel: show only for authenticated users
Panel::builder("admin_panel")
    .display(DisplayConfig::new()
        .show_when_equals("role", Value::text("admin")))
    .build()

// Notice: show warning when field invalid
Notice::builder("validation_warning")
    .notice_type(NoticeType::Warning)
    .display(DisplayConfig::new()
        .show_when_invalid("email"))
    .build()

// Object: hide shipping when "pickup" selected
Object::builder("shipping_address")
    .display(DisplayConfig::new()
        .hide_when_equals("delivery_method", Value::text("pickup")))
    .build()

// List: hide when no items
List::builder("items")
    .display(DisplayConfig::new()
        .show_when(DisplayRule::when("has_items", DisplayCondition::IsTrue)))
    .build()

// Text: show based on authentication type
Text::builder("api_key")
    .display(DisplayConfig::new()
        .show_when_equals("auth_type", Value::text("api_key")))
    .build()

// Number: show custom port only when protocol is "custom"
Number::builder("custom_port")
    .display(DisplayConfig::new()
        .show_when_equals("protocol", Value::text("custom")))
    .build()

// Select: show regions based on country
Select::builder("region")
    .display(DisplayConfig::new()
        .show_when(DisplayRule::when("country", DisplayCondition::IsSet)))
    .build()
```

### Evaluation Priority

1. `hide_when` is checked first (OR logic between multiple hide conditions)
2. `show_when` is checked second (AND logic between multiple show conditions)
3. If no conditions, node is always visible

```rust
// Multiple conditions example
let display = DisplayConfig::new()
    // Hide if disabled OR in maintenance mode
    .hide_when(DisplayRule::when("disabled", DisplayCondition::IsTrue))
    .hide_when(DisplayRule::when("maintenance", DisplayCondition::IsTrue))
    // Show only if enabled AND level > 10
    .show_when(DisplayRule::when("enabled", DisplayCondition::IsTrue))
    .show_when(DisplayRule::when("level", DisplayCondition::GreaterThan(10.0)));
```

---

## Validation System (Feature: `validation`)

> **Requires:** `features = ["validation"]`

Only **Container and Leaf types** (10 out of 13) implement the `Validatable` trait — nodes that have their own Value. Group, Layout, and Decoration do not implement `Validatable`.

### Who Implements Validatable

| Category | Types | Validatable | Reason |
|----------|-------|-------------|--------|
| Group | Group | ❌ | No own Value (only delegates) |
| Layout | Panel | ❌ | No own Value (only delegates) |
| Decoration | Notice | ❌ | No Value at all (display-only) |
| Container | Object, List, Mode, Routing, Expirable | ✅ | Has own Value |
| Leaf | Text, Number, Boolean, Vector, Select | ✅ | Has own Value |

### Validatable Trait

```rust
pub trait Validatable: Node {
    /// Expected value kind for type checking
    fn expected_kind(&self) -> Option<ValueKind>;
    
    /// Synchronous validation (fast, blocking)
    fn validate_sync(&self, value: &Value) -> Result<(), Error>;
    
    /// Asynchronous validation (may be slow, non-blocking)
    async fn validate_async(&self, value: &Value) -> Result<(), Error>;
    
    /// Full validation (sync then async)
    async fn validate(&self, value: &Value) -> Result<(), Error>;
    
    /// Get validation configuration
    fn validation(&self) -> Option<&ValidationConfig>;
    
    /// Check if value is considered empty
    fn is_empty(&self, value: &Value) -> bool;
}
```

### Validation Examples by Type

```rust
// Text with email validation
Text::builder("email")
    .validation(ValidationConfig::new()
        .required()
        .email()
        .max_length(255))
    .build()

// Number with range validation
Number::builder("port")
    .validation(ValidationConfig::new()
        .required()
        .range(1, 65535))
    .build()

// Boolean (usually no validation needed)
Boolean::builder("enabled")
    .build()

// Vector with component validation
Vector::<f64, 3>::builder("position")
    .validation(ValidationConfig::new()
        .range(-1000.0, 1000.0))  // Each component
    .build()

// Select with minimum selection
Select::builder("tags")
    .multiple(1, Some(5))
    .validation(ValidationConfig::new()
        .min_items(1)
        .max_items(5))
    .build()

// Object validates all fields
Object::builder("address")
    .field("street", Text::builder("street")
        .validation(ValidationConfig::new().required())
        .build())
    .field("city", Text::builder("city")
        .validation(ValidationConfig::new().required())
        .build())
    .build()

// List validates item count
List::builder("items")
    .validation(ValidationConfig::new()
        .min_items(1)
        .max_items(100)
        .unique_items())
    .build()

// Mode validates active variant
Mode::builder("auth")
    .variant("basic", "Basic Auth", Object::builder("basic")
        .field("username", Text::builder("username")
            .validation(ValidationConfig::new().required())
            .build())
        .build())
    .build()

// Routing validates connection
Routing::builder("input")
    .validation(ValidationConfig::new()
        .required())  // Connection required
    .build()

// Expirable validates TTL and value
Expirable::builder("token")
    .validation(ValidationConfig::new()
        .required())
    .build()
```

### Async Validation Example

```rust
// Custom async validator for username availability
pub struct UsernameAvailable {
    api: Arc<ApiClient>,
}

#[async_trait]
impl AsyncValidator<Value> for UsernameAvailable {
    async fn validate(&self, value: &Value) -> Result<(), ValidationError> {
        let username = value.as_text()
            .ok_or(ValidationError::type_mismatch("text"))?;
        
        let available = self.api.check_username(username).await
            .map_err(|e| ValidationError::external(e))?;
        
        if !available {
            return Err(ValidationError::new(
                "username_taken",
                "Username is already taken"
            ));
        }
        Ok(())
    }
}

// Usage
Text::builder("username")
    .validation(ValidationConfig::new()
        .required()
        .min_length(3)
        .max_length(32)
        .pattern(r"^[a-z0-9_]+$")
        .async_validator(UsernameAvailable::new(api))
        .debounce_ms(300))
    .build()
```

### Validation vs Displayable

| Trait | Applies To | Purpose |
|-------|-----------|---------|
| `Displayable` | ALL 13 types | Conditional visibility |
| `Validatable` | Container + Leaf (10 types) | Value validation |

```rust
// Notice has Displayable but NOT Validatable
Notice::builder("warning")
    .display(DisplayConfig::new()
        .show_when_invalid("email"))  // Can react to validation
    // .validation(...)              // NOT AVAILABLE - no value to validate
    .build()

// Text has BOTH Displayable AND Validatable
Text::builder("api_key")
    .display(DisplayConfig::new()
        .show_when_equals("auth_type", Value::text("api_key")))
    .validation(ValidationConfig::new()
        .required()
        .min_length(32))
    .build()
```

---

## Custom Validation Integration

The `Validator` trait enables integration with any validation library without adding dependencies.

### Validator Trait

```rust
pub trait Validator: Send + Sync {
    fn validate(&self, value: &Value) -> Result<(), ValidationError>;
}

// Closures implement Validator automatically
impl<F> Validator for F 
where 
    F: Fn(&Value) -> Result<(), ValidationError> + Send + Sync
{ /* ... */ }
```

### Usage Examples

```rust
// With nebula-validator
use nebula_validator::validators::{email, range, min_length};

Text::builder("email")
    .validate(email())
    .build()

Number::builder("port")
    .validate(range(1, 65535))
    .build()

// With closure
Text::builder("custom")
    .validate(|v| {
        let text = v.as_text().ok_or(ValidationError::type_mismatch("text"))?;
        if text.contains("@") { Ok(()) }
        else { Err(ValidationError::new("invalid", "Must contain @")) }
    })
    .build()

// With garde/validator - user implements wrapper in their crate
// See 00-TYPE-GRAPH.md for full example
```

### Why No Built-in Library Integration?

| Reason | Benefit |
|--------|---------|
| No `dep:garde` or `dep:validator` | No version conflicts |
| User controls dependencies | Cleaner `Cargo.lock` |
| Trait-based | Works with any library |
| Less maintenance | Focus on core functionality |

---

## Design Principle: Composition Over Proliferation

**Why 13 types instead of 50?**

1. **Simpler API** - Learn 13 types, not 50
2. **Composable** - Combine subtypes + flags for any use case
3. **Extensible** - Add new subtypes without new node types
4. **Consistent** - All text fields behave the same way
5. **Type-safe** - Subtypes are enums, not strings

**The formula:**
```
Specialized Node = Base Type + Subtype + Flags + Options
```

This gives us:
- 13 base types (1 Group + 1 Layout + 1 Decoration + 5 Container + 5 Leaf)
- 60+ text subtypes
- 50+ number subtypes  
- 35+ vector subtypes
- 14 flags
- = Thousands of combinations with a minimal API!

---

## Complete Example

A full configuration using Group as root:

```rust
let http_node = Group::builder("http_request")
    .child(Panel::builder("request")
        .label("Request")
        .child(Notice::builder("api_info")
            .notice_type(NoticeType::Info)
            .description("Configure your HTTP request parameters.")
            .build())
        .child(Select::builder("method")
            .single()
            .static_options(vec![
                SelectOption::new("GET", "GET"),
                SelectOption::new("POST", "POST"),
            ])
            .default("GET")
            .build())
        .child(Text::builder("url")
            .subtype(TextSubtype::Url)
            .required()
            .build())
        .child(List::builder("headers")
            .item_template(Object::builder("header")
                .field("name", Text::new("name"))
                .field("value", Text::new("value"))
                .build())
            .build())
        .build())
    .child(Panel::builder("auth")
        .label("Authentication")
        .child(Mode::builder("auth_type")
            .variant("none", "None", Object::empty())
            .variant("basic", "Basic", Object::builder("basic_auth")
                .field("username", Text::new("username"))
                .field("password", Text::builder("password")
                    .subtype(TextSubtype::Secret)
                    .build())
                .build())
            .default_variant("none")
            .build())
        .build())
    .build();

// Collect all values
let values = http_node.collect_values();

// Set a specific value
http_node.set_value("url".into(), Value::Text("https://api.example.com".into()))?;
```
