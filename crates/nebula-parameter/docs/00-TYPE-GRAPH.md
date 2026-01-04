# Type Graph

Formal type-level view of the `paramdef` system — a standalone, type-safe parameter definition library.

---

## Feature Flags Architecture

```toml
[package]
name = "paramdef"
version = "0.1.0"

[features]
default = []
display = []
validation = []
serde = ["dep:serde", "dep:serde_json"]
events = ["dep:tokio"]
i18n = ["dep:fluent"]
chrono = ["dep:chrono"]
full = ["display", "validation", "serde", "events", "i18n", "chrono"]

[dependencies]
# Core (always)
smartstring = "1.0"
thiserror = "2.0"
bitflags = "2.0"

# Optional
serde = { version = "1.0", optional = true, features = ["derive"] }
serde_json = { version = "1.0", optional = true }
tokio = { version = "1.0", optional = true, features = ["sync"] }
fluent = { version = "0.16", optional = true }
chrono = { version = "0.4", optional = true, default-features = false, features = ["std"] }
```

### What Each Feature Enables

| Feature | Enables |
|---------|---------|
| (none) | Core types, Node traits, Value enum |
| `display` | `Displayable` trait, `DisplayConfig`, visibility conditions |
| `validation` | `Validatable` trait, `Validator` trait, `ValidationError` |
| `serde` | Serialize/Deserialize + JSON conversions |
| `events` | Event system with tokio broadcast channels |
| `i18n` | Fluent localization, `Localizable` trait |
| `chrono` | Chrono type conversions for Date/Time/Expirable |

### Typical Feature Combinations

| Use Case | Features |
|----------|----------|
| Minimal core | `default = []` |
| Static UI | `display` |
| Forms + validation | `display, validation` |
| Server-side | `validation, serde` |
| Full async app | `full` |

### Node Struct with Features

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

#[cfg(feature = "display")]
impl Displayable for Text { /* ... */ }

#[cfg(feature = "validation")]
impl Validatable for Text { /* ... */ }

#[cfg(feature = "chrono")]
impl Text {
    pub fn as_naive_date(&self) -> Option<chrono::NaiveDate> { /* ... */ }
}
```

---

## Node Hierarchy

```
Node (base trait)
├── Group: Node               // Root aggregator, NO own Value, HAS ValueAccess API
│   └── Group
│
├── Layout: Node              // UI organization, NO own Value, HAS ValueAccess API
│   └── Panel
│
├── Decoration: Node          // Display-only, NO Value, NO children
│   └── Notice                // Info, warning, error, success messages
│
├── Container: Node           // WITH own Value, HAS ValueAccess API
│   ├── Object                // Named fields
│   ├── List                  // Dynamic array
│   ├── Mode                  // Discriminated union
│   ├── Routing               // Connection wrapper (workflow)
│   └── Expirable             // TTL wrapper (caching)
│
└── Leaf: Node                // WITH own Value (terminal), NO children
    ├── Text
    ├── Number
    ├── Boolean
    ├── Vector
    └── Select (unified)

// Total: 1 Group + 1 Layout + 1 Decoration + 5 Container + 5 Leaf = 13 types
```

---

## Trait Definitions

### Core Traits (Always Available)

```rust
/// Base trait for all nodes in parameter tree
pub trait Node: Send + Sync {
    fn metadata(&self) -> &Metadata;
    fn kind(&self) -> NodeKind;
    fn key(&self) -> &Key { &self.metadata().key }
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
```

### Feature: `display`

```rust
/// Conditional visibility for all nodes
/// ALL 13 node types implement this trait
#[cfg(feature = "display")]
pub trait Displayable: Node {
    /// Get display configuration (show_when/hide_when rules)
    fn display(&self) -> Option<&DisplayConfig>;
    
    /// Set display configuration
    fn set_display(&mut self, display: Option<DisplayConfig>);
    
    /// Check if node should be visible given current context
    fn should_display(&self, context: &DisplayContext) -> bool {
        self.display().map_or(true, |d| d.should_display(context))
    }
    
    /// Get all parameter keys this node's visibility depends on
    fn dependencies(&self) -> Vec<Key> {
        self.display().map_or(Vec::new(), |d| d.dependencies())
    }
}
```

### Feature: `validation`

```rust
/// Validation trait for nodes with values
/// Only Container (5) + Leaf (5) = 10 types implement this
#[cfg(feature = "validation")]
pub trait Validatable: Node {
    fn expected_kind(&self) -> Option<ValueKind>;
    fn validate_sync(&self, value: &Value) -> Result<(), Error>;
    async fn validate_async(&self, value: &Value) -> Result<(), Error>;
    async fn validate(&self, value: &Value) -> Result<(), Error>;
    fn validation(&self) -> Option<&ValidationConfig>;
    fn is_empty(&self, value: &Value) -> bool;
}
```

### Core Traits (continued)

```rust
/// Common API for accessing values in nodes with children
/// Implemented by Group, Layout, and Container (NOT Leaf)
pub trait ValueAccess {
    /// Collect all values from the tree
    fn collect_values(&self) -> HashMap<Key, Value>;
    
    /// Set values throughout the tree
    fn set_values(&mut self, values: HashMap<Key, Value>) -> Result<(), Error>;
    
    /// Get value by key (searches children)
    fn get_value(&self, key: &Key) -> Option<&Value>;
    
    /// Set a single value (finds the right node)
    fn set_value(&mut self, key: Key, value: Value) -> Result<(), Error>;
}

/// Root aggregator (Group)
/// Does NOT have own Value, but HAS ValueAccess API
/// Can contain: Layout, Container, Leaf
pub trait GroupNode: Node + ValueAccess {
    fn children(&self) -> &[Arc<dyn Node>];
}

/// UI-only organization (Panel)
/// Does NOT have own Value, but HAS ValueAccess API
/// Can contain: Container, Leaf (NOT Layout, NOT Group)
pub trait Layout: Node + ValueAccess {
    fn children(&self) -> &[Arc<dyn Node>];
    fn ui_state(&self) -> &dyn Any;
}

/// Data containers (Object, List, Mode)
/// HAS own Value AND ValueAccess API
/// Can contain: Container, Leaf (NOT Layout, NOT Group)
pub trait Container: Node + ValueAccess {
    fn to_value(&self) -> Value;
    fn from_value(&mut self, value: Value) -> Result<(), Error>;
    fn validate(&self) -> ValidationResult;
    fn children(&self) -> Vec<Arc<dyn Node>>;
}

/// Display-only decorations (Notice)
/// NO Value, NO children, NO ValueAccess
/// Pure UI element for showing messages
pub trait Decoration: Node {
    fn decoration_type(&self) -> DecorationType;
    fn is_dismissible(&self) -> bool;
}

/// Leaf values (Text, Number, Boolean, Vector, Select)
/// HAS own Value, NO children, NO ValueAccess
pub trait Leaf: Node {
    type ValueType: Clone + PartialEq + Debug + 'static;
    
    fn get_value(&self) -> Option<&Self::ValueType>;
    fn set_value(&mut self, value: Self::ValueType) -> Result<(), Error>;
    fn to_value(&self) -> Option<Value>;
    fn from_value(&mut self, value: Value) -> Result<(), Error>;
}


```

---

## NodeKind Enum

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    // Group (root aggregator, no own Value)
    Group,
    
    // Layout (UI only, no own Value)
    Panel,
    
    // Decoration (display-only, no Value, no children)
    Notice,
    
    // Container (with own Value)
    Object,
    List,
    Mode,
    Routing,
    Expirable,
    
    // Leaf (with own Value)
    Text,
    Number,
    Boolean,
    Vector,
    Select,
}

impl NodeKind {
    pub fn is_group(&self) -> bool {
        matches!(self, Self::Group)
    }
    
    pub fn is_layout(&self) -> bool {
        matches!(self, Self::Panel)
    }
    
    pub fn is_decoration(&self) -> bool {
        matches!(self, Self::Notice)
    }
    
    pub fn is_container(&self) -> bool {
        matches!(self, Self::Object | Self::List | Self::Mode | Self::Routing | Self::Expirable)
    }
    
    pub fn is_leaf(&self) -> bool {
        matches!(self, Self::Text | Self::Number | Self::Boolean | Self::Vector | Self::Select)
    }
    
    pub fn has_own_value(&self) -> bool {
        self.is_container() || self.is_leaf()
    }
    
    pub fn has_value_access(&self) -> bool {
        self.is_group() || self.is_layout() || self.is_container()
    }
    
    pub fn can_have_children(&self) -> bool {
        self.is_group() || self.is_layout() || self.is_container()
    }
}
```

---

## Nesting Rules

```
Group      → Layout, Decoration, Container, Leaf   (root, can contain everything)
Layout     → Decoration, Container, Leaf           (NOT Layout, NOT Group)
Container  → Decoration, Container, Leaf           (NOT Layout, NOT Group)
Decoration → nothing                               (terminal, display-only)
Leaf       → nothing                               (terminal, with value)
```

### Invariants

1. **Only Group can contain Layout**
   - Group is the root aggregator that can hold multiple Panels

2. **Layout NEVER contains Layout or Group**
   - Panel cannot contain Panel
   - Panel cannot contain Group

3. **Container NEVER contains Layout or Group**
   - Object, List, Mode cannot contain Panel or Group

4. **Decoration NEVER contains children**
   - Notice is a terminal display-only node
   - Has no Value, no children, no ValueAccess

5. **Leaf NEVER contains children**
   - Text, Number, Boolean, Vector, Select are terminal nodes

6. **Group, Layout, and Decoration do NOT have own Value**
   - Group and Panel are for organization (but have ValueAccess API)
   - Notice is for display only (no ValueAccess API)

7. **Container and Leaf have own Value**
   - Object → `Value::Object`
   - List → `Value::Array`
   - Mode → `Value::Object { mode, value }`
   - Routing → `Value::Object { connected_node_id, ... }`
   - Expirable → `Value::Object { value, expires_at, ... }`
   - Text → `Value::Text`
   - Number → `Value::Int | Value::Float`
   - Boolean → `Value::Bool`
   - Vector → `Value::Array` (fixed size)
   - Select → `Value::Text | Value::Array`

---

## Value Mapping

| Node | Category | Own Value | ValueAccess | Example |
|------|----------|-----------|-------------|---------|
| Group | Group | - | YES | Root aggregator |
| Panel | Layout | - | YES | UI tabs/sections |
| Notice | Decoration | - | NO | Info/warning/error messages |
| Object | Container | `Value::Object` | YES | `{ "name": "John", "age": 30 }` |
| List | Container | `Value::Array` | YES | `[{ "id": 1 }, { "id": 2 }]` |
| Mode | Container | `Value::Object` | YES | `{ "mode": "basic", "value": { "user": "..." } }` |
| Routing | Container | `Value::Object` | YES | `{ "connected_node_id": "node-1", ... }` |
| Expirable | Container | `Value::Object` | YES | `{ "value": ..., "expires_at": "..." }` |
| Text | Leaf | `Value::Text` | NO | `"hello@example.com"` |
| Number | Leaf | `Value::Int/Float` | NO | `42` or `3.14` |
| Boolean | Leaf | `Value::Bool` | NO | `true` |
| Vector | Leaf | `Value::Array` | NO | `[1.0, 0.5, 0.0, 1.0]` (RGBA) |
| Select | Leaf | `Value::Text/Array` | NO | `"option1"` or `["a", "b"]` |

---

## Type Composition

### Group Type

```rust
/// Group - root aggregator
/// Can contain: Layout, Container, Leaf (everything)
pub struct Group {
    metadata: Metadata,
    children: Vec<Arc<dyn Node>>,  // Layout | Container | Leaf
    layout: GroupLayout,
    collapsed: bool,
}

impl ValueAccess for Group {
    fn collect_values(&self) -> HashMap<Key, Value> {
        self.children.iter()
            .flat_map(|child| match child.kind() {
                NodeKind::Group => unreachable!("Group cannot contain Group"),
                _ => child.collect_values(),
            })
            .collect()
    }
    // ... other methods delegate to children
}
```

### Layout Type

```rust
/// Panel - tabs or sections
/// Can contain: Container, Leaf (NOT Layout, NOT Group)
pub struct Panel {
    metadata: Metadata,
    children: Vec<Arc<dyn Node>>,  // Container | Leaf only
    display_type: PanelDisplayType,
    selected_tab: Option<usize>,
}

impl ValueAccess for Panel {
    fn collect_values(&self) -> HashMap<Key, Value> {
        self.children.iter()
            .flat_map(|child| child.collect_values())
            .collect()
    }
    // ... other methods delegate to children
}
```

### Decoration Type

```rust
/// Notice - display-only message
/// Shows info, warning, error, or success messages
pub struct Notice {
    metadata: Metadata,
    notice_type: NoticeType,
    dismissible: bool,
}

pub enum NoticeType {
    Info,
    Warning,
    Error,
    Success,
}
```

### Container Types

```rust
/// Object - structured data with named fields
pub struct Object {
    metadata: Metadata,
    fields: Vec<(String, Arc<dyn Node>)>,  // Container | Leaf only
}

/// List - dynamic array from template
pub struct List {
    metadata: Metadata,
    item_template: Arc<dyn Node>,  // Container | Leaf only
    items: Vec<Arc<dyn Node>>,
    min_items: Option<usize>,
    max_items: Option<usize>,
}

/// Mode - discriminated union
pub struct Mode {
    metadata: Metadata,
    variants: Vec<ModeVariant>,
    current_variant: Option<String>,
}

pub struct ModeVariant {
    pub key: String,
    pub label: String,
    pub content: Arc<dyn Node>,  // Container | Leaf only
}

/// Routing - connection wrapper for workflow nodes
pub struct Routing {
    metadata: Metadata,
    child: Option<Arc<dyn Node>>,  // Container | Leaf only
    options: RoutingOptions,
}

pub struct RoutingOptions {
    pub connection_label: Option<String>,
    pub connection_required: bool,
    pub max_connections: Option<usize>,
}

/// Expirable - TTL wrapper for cached values
pub struct Expirable {
    metadata: Metadata,
    child: Option<Arc<dyn Node>>,  // Container | Leaf only
    options: ExpirableOptions,
}

pub struct ExpirableOptions {
    pub ttl: u64,              // Time-to-live in seconds
    pub auto_refresh: bool,
    pub auto_clear_expired: bool,
    pub warning_threshold: Option<u64>,
}
```

### Leaf Types

```rust
/// Text - string values with subtypes
pub struct Text {
    metadata: Metadata,
    subtype: TextSubtype,
    value: Option<String>,
    validators: Vec<ValidationRule>,
}

/// Number - numeric values with subtypes
pub struct Number {
    metadata: Metadata,
    subtype: NumberSubtype,
    value: Option<f64>,
    validators: Vec<ValidationRule>,
}

/// Boolean - true/false values
pub struct Boolean {
    metadata: Metadata,
    value: Option<bool>,
}

/// Vector - fixed-size numeric arrays with subtypes
pub struct Vector {
    metadata: Metadata,
    subtype: VectorSubtype,
    size: usize,  // 2, 3, or 4
    value: Option<Vec<f64>>,
    validators: Vec<ValidationRule>,
}

/// Select - unified selection (single/multiple × static/dynamic)
pub struct Select {
    metadata: Metadata,
    selection_mode: SelectionMode,
    option_source: OptionSource,
    value: SelectValue,
    validators: Vec<ValidationRule>,
}
```

---

## Unified Select

Select combines 4 combinations into one type:

```rust
pub enum SelectionMode {
    Single,
    Multiple { min: Option<usize>, max: Option<usize> },
}

pub enum OptionSource {
    Static { options: Vec<SelectOption> },
    Dynamic { loader: ResourceLoader, cache: OptionCache },
}

pub enum SelectValue {
    Single(Option<String>),
    Multiple(Vec<String>),
}
```

| Selection Mode | Option Source | Value |
|----------------|---------------|-------|
| Single | Static | `Value::Text` |
| Multiple | Static | `Value::Array` |
| Single | Dynamic | `Value::Text` |
| Multiple | Dynamic | `Value::Array` |

---

## Specializations via Composition

Instead of many types, use **base type + subtype + flags**:

| Specialization | Composition |
|----------------|-------------|
| Secret | `Text` + `subtype: Secret` + `flags: SENSITIVE \| WRITE_ONLY` |
| Password | `Text` + `subtype: Secret` + `flags: SENSITIVE \| WRITE_ONLY` |
| Email | `Text` + `subtype: Email` |
| URL | `Text` + `subtype: Url` |
| DateTime | `Text` + `subtype: DateTime` |
| Date | `Text` + `subtype: Date` |
| Time | `Text` + `subtype: Time` |
| Code | `Text` + `subtype: Code(Language)` |
| Json | `Text` + `subtype: Json` |
| FilePath | `Text` + `subtype: FilePath` |
| Percentage | `Number` + `subtype: Percentage` |
| Currency | `Number` + `subtype: Currency` |
| Port | `Number` + `subtype: Port` |
| Rating | `Number` + `subtype: Rating` |
| ColorRgb | `Vector` + `subtype: ColorRgb` |
| ColorRgba | `Vector` + `subtype: ColorRgba` |
| Position2D | `Vector` + `subtype: Position2D` |
| Position3D | `Vector` + `subtype: Position` |
| Scale | `Vector` + `subtype: Scale` |

---

## Ownership Rules

| Owns | Entity |
|------|--------|
| Own Value | Container, Leaf |
| ValueAccess API | Group, Layout, Container |
| Children | Group, Layout, Container |
| UI State | Group, Layout, Decoration |

### With Feature Flags

| Feature | Trait | Applies To |
|---------|-------|-----------|
| `display` | `Displayable` | ALL 13 types |
| `validation` | `Validatable` | Container + Leaf (10 types) |

---

## Validatable Trait (Feature: `validation`)

> **Requires:** `features = ["validation"]`

**Only nodes with values** implement `Validatable`: Container (5) + Leaf (5) = 10 types.

Group, Layout, and Decoration do NOT implement Validatable (they have no own value).

```rust
pub struct ValidationConfig {
    validate_fn: Option<Arc<dyn Fn(&Value) -> Result<(), ValidationError> + Send + Sync>>,
}

impl ValidationConfig {
    /// Create from any nebula-validator validator
    pub fn from<V, T>(validator: V) -> Self
    where
        V: Validator<Input = T> + Send + Sync + 'static,
        Value: AsValidatable<T>,
    { ... }
    
    /// Validate a value (skips null)
    pub fn validate(&self, value: &Value) -> Result<(), ValidationError>;
}
```

### Validation Flow

```
User Input
    │
    ▼
┌─────────────────┐
│  validate_sync  │  ← Type check, range, regex (fast)
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ validate_async  │  ← DB uniqueness, API calls (slow)
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ ValidationConfig│  ← Custom validators from nebula-validator
└────────┬────────┘
         │
         ▼
    Result<(), Error>
```

### Built-in Validators (from nebula-validator)

| Category | Validators |
|----------|------------|
| String | `min_length`, `max_length`, `email`, `url`, `regex`, `uuid` |
| Numeric | `min`, `max`, `range`, `positive`, `negative` |
| Collection | `min_items`, `max_items`, `unique` |
| Combinators | `and`, `or`, `not`, `optional`, `with_message` |

### Example Usage

```rust
use nebula_validator::validators::string::{min_length, email};
use nebula_validator::combinators::{and, with_message};

// Text with email validation
Text::builder("email")
    .validation(ValidationConfig::from(email()))
    .build()

// Password with custom message
Text::builder("password")
    .validation(ValidationConfig::from(
        with_message(min_length(8), "Password must be at least 8 characters")
    ))
    .build()

// Number with range
Number::builder("age")
    .validation(ValidationConfig::from(
        and(min(0), max(150))
    ))
    .build()

// Async validation (database check)
Text::builder("username")
    .validate_async(|value| async {
        let username = value.as_str().unwrap();
        if db.username_exists(username).await? {
            Err(Error::InvalidValue {
                key: "username".into(),
                reason: "Username already taken".into(),
            })
        } else {
            Ok(())
        }
    })
    .build()
```

### Validatable by Category

| Category | Validatable | Reason |
|----------|-------------|--------|
| Group | ❌ NO | No own value |
| Layout (Panel) | ❌ NO | No own value |
| Decoration (Notice) | ❌ NO | No own value |
| Container (Object, List, Mode, Routing, Expirable) | ✅ YES | Has own value |
| Leaf (Text, Number, Boolean, Vector, Select) | ✅ YES | Has own value |

---

## Custom Validation Integration

The `Validator` trait is designed for easy integration with any validation library.

### Validator Trait

```rust
/// Universal validation trait
pub trait Validator: Send + Sync {
    fn validate(&self, value: &Value) -> Result<(), ValidationError>;
}

/// Closures automatically implement Validator
impl<F> Validator for F 
where 
    F: Fn(&Value) -> Result<(), ValidationError> + Send + Sync
{
    fn validate(&self, value: &Value) -> Result<(), ValidationError> {
        self(value)
    }
}
```

### With nebula-validator

```rust
use nebula_validator::validators::{email, range, min_length};

// Direct usage - blanket impl makes it seamless
Text::builder("email")
    .validate(email())
    .build()

Number::builder("port")
    .validate(range(1, 65535))
    .build()

Text::builder("username")
    .validate(min_length(3).and(max_length(32)))
    .build()
```

### With garde (user implements wrapper)

```rust
use garde::Validate;

// User defines wrapper in their crate
struct GardeValidator<T>(PhantomData<T>);

impl<T: garde::Validate + Default> Validator for GardeValidator<T> {
    fn validate(&self, value: &Value) -> Result<(), ValidationError> {
        // Convert Value to T and validate
        let input: T = value.try_into()?;
        input.validate(&()).map_err(Into::into)
    }
}

// Usage
#[derive(Validate, Default)]
struct EmailInput {
    #[garde(email)]
    value: String,
}

Text::builder("email")
    .validate(GardeValidator::<EmailInput>::default())
    .build()
```

### With closure (quick custom validation)

```rust
Text::builder("custom_field")
    .validate(|value| {
        let text = value.as_text().ok_or(ValidationError::type_mismatch("text"))?;
        if text.starts_with("valid_") {
            Ok(())
        } else {
            Err(ValidationError::new("invalid_prefix", "Must start with 'valid_'"))
        }
    })
    .build()
```

---

## Displayable Trait (Feature: `display`)

> **Requires:** `features = ["display"]`

**ALL 13 node types** implement `Displayable` for conditional visibility:

```rust
pub struct DisplayConfig {
    show_when: Option<DisplayRuleSet>,  // Conditions to show
    hide_when: Option<DisplayRuleSet>,  // Conditions to hide (priority)
}

pub enum DisplayCondition {
    Equals(Value),
    NotEquals(Value),
    IsSet,
    IsNull,
    IsEmpty,
    IsNotEmpty,
    IsTrue,
    IsFalse,
    GreaterThan(f64),
    LessThan(f64),
    InRange { min: f64, max: f64 },
    Contains(String),
    StartsWith(String),
    EndsWith(String),
    OneOf(Vec<Value>),
    IsValid,
    IsInvalid,
}
```

### Use Cases by Category

| Category | Example Use Case |
|----------|------------------|
| Group | Hide entire settings group in "simple mode" |
| Panel | Show "Advanced" tab only for power users |
| Notice | Show warning only when validation fails |
| Object | Hide address fields when "same as billing" checked |
| List | Hide items list when empty |
| Mode | Always visible (user selects variant) |
| Routing | Show connection point based on node type |
| Expirable | Show refresh button when token expiring soon |
| Text | Show API key field when auth_type="api_key" |
| Number | Show port field when protocol="custom" |
| Boolean | Always visible (toggles other fields) |
| Vector | Show color picker when "use custom color" enabled |
| Select | Show region selector based on selected country |

### Example

```rust
// Show API key field only when auth type is "api_key"
Text::builder("api_key")
    .display(DisplayConfig::new()
        .show_when_equals("auth_type", Value::text("api_key")))
    .build()

// Show warning notice when password is invalid
Notice::builder("password_warning")
    .notice_type(NoticeType::Warning)
    .display(DisplayConfig::new()
        .show_when_invalid("password"))
    .build()

// Hide advanced panel in simple mode
Panel::builder("advanced")
    .display(DisplayConfig::new()
        .hide_when_equals("mode", Value::text("simple")))
    .build()
```

---

## Forbidden Relationships

- Group → Group (Group cannot contain Group)
- Layout → Layout (Panel cannot contain Panel)
- Layout → Group (Panel cannot contain Group)
- Container → Layout (Object/List/Mode cannot contain Panel)
- Container → Group (Object/List/Mode cannot contain Group)
- Decoration → children (Notice has no children)
- Leaf → children (Text/Number/Boolean/Vector/Select have no children)

---

## Mental Model

```
Group      = ROOT aggregator (collects everything)
Layout     = what the user SEES (UI organization)
Decoration = what the user READS (display-only messages)
Container  = what the data LOOKS LIKE (structure)
Leaf       = what the data IS (values)
```

**Key distinctions:**
- Group/Layout have **no own Value** but provide **ValueAccess API** to delegate
- Decoration has **no own Value** and **no ValueAccess API** (pure display)
- Container/Leaf have **own Value**
- Only Decoration and Leaf have **no children**

This separation is **non-negotiable**.
