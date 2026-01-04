# Architecture

Core architectural decisions and design philosophy of `paramdef` — a standalone, type-safe parameter definition library.

---

## Vision

Create the **"serde of parameter schemas"** - a library so well-designed that it becomes the natural choice for:
- Workflow engines (n8n, Temporal, Airflow alternatives)
- Visual programming tools
- No-code platforms
- Game engines and 3D tools
- CLI tools and form builders
- Data processing pipelines

**Inspiration:**
- **Blender RNA** - Property system architecture
- **Unreal Engine UPROPERTY** - Metadata and flags
- **Qt Property System** - Signals and observers
- **Houdini Parameters** - Node-based workflows

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

| Feature | Enables |
|---------|---------|
| (none) | Core types, Node traits, Value enum |
| `display` | `Displayable` trait, `DisplayConfig`, visibility conditions |
| `validation` | `Validatable` trait, `Validator` trait, `ValidationError` |
| `serde` | Serialize/Deserialize + JSON conversions (From, FromStr, Display) |
| `events` | Event system with tokio broadcast channels |
| `i18n` | Fluent localization, `Localizable` trait |
| `chrono` | Chrono type conversions for Date/Time/Expirable |

**Typical combinations:**

| Use Case | Features |
|----------|----------|
| Minimal core | `default = []` |
| Static UI | `display` |
| Forms + validation | `display, validation` |
| Server-side | `validation, serde` |
| Full async app | `full` |

---

## Node Hierarchy

The system is built on a trait-based hierarchy with five categories:

```
Node (base trait)
├── Group: Node               // Root aggregator, NO own Value, HAS ValueAccess
│   └── Group                 // Can contain everything
│
├── Layout: Node              // UI organization, NO own Value, HAS ValueAccess
│   └── Panel                 // Tabs/sections
│
├── Decoration: Node          // Display-only, NO Value, NO children
│   └── Notice                // Info/warning/error/success messages
│
├── Container: Node           // WITH own Value, HAS ValueAccess
│   ├── Object                // Named fields → Value::Object
│   ├── List                  // Dynamic array → Value::Array
│   ├── Mode                  // Discriminated union → Value::Object
│   ├── Routing               // Connection wrapper → Value::Object
│   └── Expirable             // TTL wrapper → Value::Object
│
└── Leaf: Node                // WITH own Value (terminal), NO children
    ├── Text                  // String values → Value::Text
    ├── Number                // Numeric values → Value::Int/Float
    ├── Boolean               // True/false → Value::Bool
    ├── Vector                // Fixed arrays → Value::Array
    └── Select (unified)      // Selections → Value::Text/Array

// Total: 1 Group + 1 Layout + 1 Decoration + 5 Container + 5 Leaf = 13 types
```

### Nesting Rules

```
Group      → Layout, Decoration, Container, Leaf   (root, can contain everything)
Layout     → Decoration, Container, Leaf           (NOT Layout, NOT Group)
Container  → Decoration, Container, Leaf           (NOT Layout, NOT Group)
Decoration → nothing                               (terminal, display-only)
Leaf       → nothing                               (terminal, with value)
```

### Trait Definitions

```rust
/// Base trait for all nodes
pub trait Node: Send + Sync {
    fn metadata(&self) -> &Metadata;
    fn kind(&self) -> NodeKind;
    fn key(&self) -> &Key;
}

/// Common API for accessing values in nodes with children
/// Implemented by Group, Layout, and Container (NOT Leaf)
pub trait ValueAccess {
    fn collect_values(&self) -> HashMap<Key, Value>;
    fn set_values(&mut self, values: HashMap<Key, Value>) -> Result<(), Error>;
    fn get_value(&self, key: &Key) -> Option<&Value>;
    fn set_value(&mut self, key: Key, value: Value) -> Result<(), Error>;
}

/// Root aggregator (Group) - NO own Value, HAS ValueAccess
pub trait GroupNode: Node + ValueAccess {
    fn children(&self) -> &[Arc<dyn Node>];
}

/// UI-only organization (Panel) - NO own Value, HAS ValueAccess
pub trait Layout: Node + ValueAccess {
    fn children(&self) -> &[Arc<dyn Node>];
    fn ui_state(&self) -> &dyn Any;
}

/// Data containers (Object, List, Mode) - WITH own Value, HAS ValueAccess
pub trait Container: Node + ValueAccess {
    fn to_value(&self) -> Value;
    fn from_value(&mut self, value: Value) -> Result<(), Error>;
    fn validate(&self) -> ValidationResult;
    fn children(&self) -> Vec<Arc<dyn Node>>;
}

/// Display-only decorations (Notice) - NO Value, NO children, NO ValueAccess
pub trait Decoration: Node {
    fn decoration_type(&self) -> DecorationType;
    fn is_dismissible(&self) -> bool;
}

/// Leaf values (Text, Number, Boolean, Vector, Select) - WITH own Value, NO children
pub trait Leaf: Node {
    type ValueType: Clone + PartialEq + Debug + 'static;
    
    fn get_value(&self) -> Option<&Self::ValueType>;
    fn set_value(&mut self, value: Self::ValueType) -> Result<(), Error>;
    fn to_value(&self) -> Option<Value>;
    fn from_value(&mut self, value: Value) -> Result<(), Error>;
    fn validate(&self) -> ValidationResult;
}
```

---

## Component Hierarchy

```
Context
├── EventBus (broadcast channel)
├── HistoryManager (undo/redo)
├── DisplayObserver (formatting)
└── Nodes (tree structure)
    └── Group (Group) ← root aggregator
        ├── Panel "General" (Layout)
        │   ├── Notice (Decoration) ← info message
        │   ├── Text (Leaf)
        │   └── Number (Leaf)
        ├── Panel "Advanced" (Layout)
        │   ├── Notice (Decoration) ← warning
        │   └── Object (Container)
        │       ├── Text (Leaf)
        │       └── Select (Leaf)
        └── List (Container)
            └── Object (template)
```

### Group as Root Aggregator

Group is the root container that can hold multiple Panels:

```rust
let config = Group::builder("settings")
    .child(Panel::builder("general")
        .child(Text::new("name"))
        .child(Number::new("port"))
        .build())
    .child(Panel::builder("advanced")
        .child(Object::builder("options")
            .field("timeout", Number::new("timeout"))
            .field("retries", Number::new("retries"))
            .build())
        .build())
    .build();

// Get all values from the entire tree
let values = config.collect_values();

// Set a specific value (Group finds the right node)
config.set_value("port".into(), Value::Int(8080))?;
```

### RuntimeNode<T>

The generic pattern that wraps any node type with runtime state:

```rust
pub struct RuntimeNode<T: Node> {
    /// Immutable node definition (shared via Arc)
    node: Arc<T>,
    
    /// State flags (bitflags)
    state: StateFlags,
    
    /// Validation errors (if any)
    errors: Vec<ValidationError>,
}

impl<T: Leaf> RuntimeNode<T> {
    pub fn set_value(&mut self, new_value: T::ValueType) -> Result<(), ValidationError> {
        // 1. Transform (clamp, round, normalize)
        let transformed = self.node.transform(new_value)?;
        
        // 2. Validate
        self.node.validate(&transformed)?;
        
        // 3. Store
        self.node.set_value(transformed)?;
        self.state.insert(StateFlags::DIRTY);
        
        // 4. Notify (via EventBus)
        Ok(())
    }
}
```

### StateFlags

Runtime state tracked per parameter (distinct from schema-level Flags):

```rust
bitflags! {
    pub struct StateFlags: u8 {
        const DIRTY   = 0b0000_0001;  // Value changed since last save
        const TOUCHED = 0b0000_0010;  // User interacted with field
        const VALID   = 0b0000_0100;  // Passed validation
        const VISIBLE = 0b0000_1000;  // Currently visible
        const ENABLED = 0b0001_0000;  // Currently enabled
        const READONLY= 0b0010_0000;  // Currently readonly
    }
}
```

**Difference from Flags:**
- `Flags` - schema-level, immutable (Required, Secret, ReadOnly)
- `StateFlags` - runtime-level, mutable (Dirty, Touched, Valid)

---

## Built-in Observers

| Observer | Purpose |
|----------|---------|
| `LoggerObserver` | Logs parameter changes for debugging |
| `ValidationObserver` | Triggers re-validation on changes |
| `DependencyObserver` | Updates dependent parameters |
| `UiObserver` | Notifies UI of state changes |
| `DisplayObserver` | Evaluates visibility conditions |

---

## Conditional Visibility (Feature: `display`)

> **Requires:** `features = ["display"]`

**ALL 13 node types** implement the `Displayable` trait for conditional visibility.

```rust
#[cfg(feature = "display")]
pub trait Displayable: Node {
    fn display(&self) -> Option<&DisplayConfig>;
    fn set_display(&mut self, display: Option<DisplayConfig>);
    fn should_display(&self, context: &DisplayContext) -> bool;
    fn dependencies(&self) -> Vec<Key>;
}
```

### DisplayConfig Configuration

```rust
pub struct DisplayConfig {
    show_when: Option<DisplayRuleSet>,  // Conditions to show (AND)
    hide_when: Option<DisplayRuleSet>,  // Conditions to hide (OR, priority)
}
```

### DisplayCondition Variants

| Condition | Description | Example |
|-----------|-------------|---------|
| `Equals(Value)` | Value equals | `auth_type == "api_key"` |
| `NotEquals(Value)` | Value not equals | `status != "disabled"` |
| `IsSet` | Value is not null | Field has value |
| `IsNull` | Value is null | Field is empty |
| `IsEmpty` | Empty string/array/object | `""`, `[]`, `{}` |
| `IsNotEmpty` | Not empty | Has content |
| `IsTrue` | Boolean is true | Checkbox checked |
| `IsFalse` | Boolean is false | Checkbox unchecked |
| `GreaterThan(f64)` | Numeric comparison | `count > 10` |
| `LessThan(f64)` | Numeric comparison | `count < 100` |
| `InRange { min, max }` | Numeric range | `1 <= x <= 100` |
| `Contains(String)` | String contains | `url.contains("api")` |
| `StartsWith(String)` | String prefix | `url.startsWith("https")` |
| `EndsWith(String)` | String suffix | `file.endsWith(".json")` |
| `OneOf(Vec<Value>)` | Value in list | `method in ["GET", "POST"]` |
| `IsValid` | Passed validation | Show after valid input |
| `IsInvalid` | Failed validation | Show error hint |

### Visibility by Node Category

| Category | Typical Use Case |
|----------|------------------|
| Group | Hide entire group in "simple mode" |
| Panel | Show "Advanced" tab for power users |
| Notice | Show warning when validation fails |
| Object | Hide section when checkbox unchecked |
| List | Hide when empty |
| Mode | Usually always visible |
| Routing | Show based on node capabilities |
| Expirable | Show refresh when expiring soon |
| Leaf | Show field based on other field values |

### Example Usage

```rust
// Show API key only when auth type is "api_key"
Text::builder("api_key")
    .display(DisplayConfig::new()
        .show_when_equals("auth_type", Value::text("api_key")))
    .build()

// Show error notice when email is invalid
Notice::builder("email_error")
    .notice_type(NoticeType::Error)
    .display(DisplayConfig::new()
        .show_when_invalid("email"))
    .build()

// Complex: show when enabled AND level > 10
let display = DisplayConfig::new()
    .show_when(DisplayRuleSet::all([
        DisplayRule::when("enabled", DisplayCondition::IsTrue),
        DisplayRule::when("level", DisplayCondition::GreaterThan(10.0)),
    ]));
```

---

## Validation System (Feature: `validation`)

> **Requires:** `features = ["validation"]`

Only **Container and Leaf types** (10 out of 13) implement the `Validatable` trait — nodes that have their own Value.

```rust
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

### Who Implements Validatable

| Category | Types | Validatable |
|----------|-------|-------------|
| Group | Group | ❌ No own Value |
| Layout | Panel | ❌ No own Value |
| Decoration | Notice | ❌ No Value at all |
| Container | Object, List, Mode, Routing, Expirable | ✅ **Yes** |
| Leaf | Text, Number, Boolean, Vector, Select | ✅ **Yes** |

### ValidationConfig Configuration

```rust
pub struct ValidationConfig {
    /// Synchronous validators (run first, fast)
    sync_validators: Vec<Arc<dyn Validator<Value>>>,
    
    /// Asynchronous validators (run second, may be slow)
    async_validators: Vec<Arc<dyn AsyncValidator<Value>>>,
    
    /// Custom error messages
    error_messages: HashMap<String, String>,
    
    /// Debounce for async validation (ms)
    debounce_ms: Option<u32>,
}
```

### Validation Flow

```
User Input
    │
    ▼
┌──────────────────────────────┐
│  1. expected_kind() check    │  ← Type mismatch early exit
└──────────────────────────────┘
    │
    ▼
┌──────────────────────────────┐
│  2. is_empty() check         │  ← If required and empty → error
└──────────────────────────────┘
    │
    ▼
┌──────────────────────────────┐
│  3. validate_sync()          │  ← All sync validators (fast)
└──────────────────────────────┘
    │
    ▼
┌──────────────────────────────┐
│  4. validate_async()         │  ← All async validators (debounced)
└──────────────────────────────┘
    │
    ▼
  Result<(), Error>
```

### Built-in Validators

| Validator | Applies To | Description |
|-----------|------------|-------------|
| `Required` | All | Value must not be empty |
| `MinLength(n)` | Text | Minimum string length |
| `MaxLength(n)` | Text | Maximum string length |
| `Pattern(regex)` | Text | Regex pattern match |
| `Email` | Text | Valid email format |
| `Url` | Text | Valid URL format |
| `MinValue(n)` | Number | Minimum numeric value |
| `MaxValue(n)` | Number | Maximum numeric value |
| `Range(min, max)` | Number | Value within range |
| `MinItems(n)` | List, Select(multiple) | Minimum array items |
| `MaxItems(n)` | List, Select(multiple) | Maximum array items |
| `UniqueItems` | List, Select(multiple) | All items unique |
| `Custom(fn)` | All | Custom validation function |

### Async Validators

```rust
// Check if username is available (API call)
pub struct UsernameAvailable;

#[async_trait]
impl AsyncValidator<Value> for UsernameAvailable {
    async fn validate(&self, value: &Value) -> Result<(), ValidationError> {
        let username = value.as_text().ok_or(ValidationError::type_mismatch())?;
        
        let available = api::check_username(username).await?;
        if !available {
            return Err(ValidationError::new("username_taken", "Username already taken"));
        }
        Ok(())
    }
}
```

### Example Usage

```rust
// Text with sync validation
Text::builder("email")
    .validation(ValidationConfig::new()
        .required()
        .email()
        .max_length(255))
    .build()

// Number with range
Number::builder("port")
    .validation(ValidationConfig::new()
        .required()
        .range(1, 65535))
    .build()

// Text with async validation
Text::builder("username")
    .validation(ValidationConfig::new()
        .required()
        .min_length(3)
        .max_length(32)
        .pattern(r"^[a-z0-9_]+$")
        .async_validator(UsernameAvailable)
        .debounce_ms(300))
    .build()

// List with item constraints
List::builder("tags")
    .validation(ValidationConfig::new()
        .min_items(1)
        .max_items(10)
        .unique_items())
    .build()
```

### Validation vs Transform

| Aspect | Transform | Validate |
|--------|-----------|----------|
| Purpose | Coerce value | Check value |
| When | Before validation | After transform |
| Result | Modified value | Pass/Fail |
| Examples | Clamp, trim, normalize | Required, min, max, pattern |

```rust
Number::builder("percentage")
    .transform(|v| v.clamp(0.0, 100.0))  // Transform: coerce to range
    .validation(ValidationConfig::new()
        .range(0.0, 100.0))               // Validate: ensure in range
    .build()
```

---

## Custom Validation Integration

The `Validator` trait allows integration with any validation library.

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

### Integration Examples

```rust
// 1. With nebula-validator (blanket impl)
use nebula_validator::validators::{email, range};

Text::builder("email")
    .validate(email())
    .build()

Number::builder("port")
    .validate(range(1, 65535))
    .build()

// 2. With closure (quick custom)
Text::builder("custom")
    .validate(|v| {
        let text = v.as_text().ok_or(ValidationError::type_mismatch("text"))?;
        if text.len() >= 3 { Ok(()) } 
        else { Err(ValidationError::new("too_short", "Min 3 chars")) }
    })
    .build()

// 3. With garde/validator (user implements wrapper)
// See "Custom Validation Integration" in 00-TYPE-GRAPH.md
```

### Design Rationale

| Approach | Benefit |
|----------|---------|
| No built-in deps | No version conflicts |
| Trait-based | Works with any library |
| Closure support | Quick one-off validation |
| Blanket impl | Seamless nebula-validator integration |

---

## Design Philosophy

### Core Principles

1. **Compile-Time Safety Over Runtime Flexibility**
   - Type-safe property keys with `PropertyKey<T>`
   - Compile-time validation where possible
   - No stringly-typed access patterns

2. **Separation of Concerns**
   - **Schema Layer** - Immutable parameter definitions
   - **Runtime Layer** - Mutable values and state
   - **UI Layer** - Optional presentation concerns

3. **Zero-Cost Abstractions**
   - SmartString for stack-allocated short strings
   - Arc-based sharing for immutable data
   - Const generics for fixed-size vectors
   - Fast path when no transformers/validators present

4. **KISS (Keep It Simple)**
   - 10 core node types, not 50
   - Subtypes for semantic variation
   - Units for measurement systems

---

## Three-Layer Architecture

```
+--------------------------------------------------+
|              SCHEMA LAYER (Immutable)            |
|  - Parameter definitions                         |
|  - Metadata, flags, validators                   |
|  - Shareable via Arc across contexts             |
+--------------------------------------------------+
                        |
                        v
+--------------------------------------------------+
|              RUNTIME LAYER (Mutable)             |
|  - Current values                                |
|  - State flags (dirty, touched, valid)           |
|  - Validation errors                             |
|  - Per-instance, per-context                     |
+--------------------------------------------------+
                        |
                        v
+--------------------------------------------------+
|              VALUE LAYER                         |
|  - Runtime data representation                   |
|  - Serialization target                          |
|  - Expression support                            |
+--------------------------------------------------+
```

### Schema vs Runtime Separation

**Schema (Immutable):**
```rust
struct Text {
    metadata: Arc<Metadata>,
    flags: Flags,
    subtype: TextSubtype,
    validators: Vec<Arc<dyn Validator<String>>>,
    // NO runtime state here!
}
```

**Runtime (Mutable):**
```rust
struct Context {
    schema: Arc<Schema>,
    values: HashMap<Key, Value>,
    states: HashMap<Key, ParameterState>,
}
```

**Benefits:**
- One schema, multiple contexts (form instances)
- Thread-safe schema sharing via Arc
- Clean undo/redo support
- Better testability

---

## Type System Overview

### 13 Core Node Types

The system defines exactly 13 node types across five categories:

#### Group Type (1) - Root Aggregator, NO own Value, HAS ValueAccess

| Type | Purpose | Contains |
|------|---------|----------|
| `Group` | Root aggregator | Layout, Decoration, Container, Leaf |

#### Layout Type (1) - UI Organization, NO own Value, HAS ValueAccess

| Type | Purpose | Contains |
|------|---------|----------|
| `Panel` | Tabs and sections | Decoration, Container, Leaf |

#### Decoration Type (1) - Display-Only, NO Value, NO Children

| Type | Purpose | Value |
|------|---------|-------|
| `Notice` | Info/warning/error/success messages | - |

#### Container Types (5) - WITH own Value, HAS ValueAccess

| Type | Purpose | Value |
|------|---------|-------|
| `Object` | Named fields | `Value::Object` |
| `List` | Dynamic arrays | `Value::Array` |
| `Mode` | Discriminated unions | `Value::Object` |
| `Routing` | Connection wrapper (workflow) | `Value::Object` |
| `Expirable` | TTL wrapper (caching) | `Value::Object` |

#### Leaf Types (5) - WITH own Value, NO children

| Type | Purpose | Value |
|------|---------|-------|
| `Text` | String-based data | `Value::Text` |
| `Number` | Numeric data | `Value::Int` / `Value::Float` |
| `Boolean` | True/false toggles | `Value::Bool` |
| `Vector` | Fixed-size arrays | `Value::Array` |
| `Select` | Single/multiple selection | `Value::Text` / `Value::Array` |

### Unified Select Architecture

Select is a single type that covers 4 use cases via composition:

```rust
pub struct Select {
    selection_mode: SelectionMode,  // Single | Multiple { min, max }
    option_source: OptionSource,    // Static { options } | Dynamic { loader, cache }
    value: SelectValue,             // Single(Option<String>) | Multiple(Vec<String>)
}
```

| Selection Mode | Option Source | Value Type | Use Case |
|----------------|---------------|------------|----------|
| Single | Static | `Value::Text` | Dropdown, radio buttons |
| Multiple | Static | `Value::Array` | Multi-select, checkboxes |
| Single | Dynamic | `Value::Text` | Async search, resource picker |
| Multiple | Dynamic | `Value::Array` | Multi-resource selection |

### Specializations via Composition

All specialized types are built from base types + subtypes + flags:

| Specialization | Base | Subtype | Flags |
|----------------|------|---------|-------|
| Secret | Text | Secret | SENSITIVE, WRITE_ONLY |
| Password | Text | Secret | SENSITIVE, WRITE_ONLY |
| DateTime | Text | DateTime | - |
| Date | Text | Date | - |
| Time | Text | Time | - |
| Code | Text | Code(Language) | - |
| FilePath | Text | FilePath | - |
| Email | Text | Email | - |
| URL | Text | URL | - |
| Color RGB | Vector | ColorRgb | - |
| Color RGBA | Vector | ColorRgba | - |
| Position 2D | Vector | Position2D | - |
| Position 3D | Vector | Position3D | - |

### Value Enum

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
    Expression { template: String, compiled: Option<Arc<CompiledExpr>> },
}
```

---

## Key Architectural Decisions

### 1. Subtype + Unit Pattern (Blender-style)

Separate **semantic meaning** from **measurement system**:

```rust
Number {
    subtype: NumberSubtype::Distance,  // WHAT it is
    unit: NumberUnit::Length,          // HOW to measure
}
```

**Benefits:**
- Automatic unit conversion (m <-> ft)
- User preferences for display
- Localization support
- 20 subtypes x 30 units = 600 combinations

### 2. Soft vs Hard Constraints

```rust
Number {
    hard_min: Some(0.0),    // Validation enforced
    hard_max: Some(100.0),  // Validation enforced
    soft_min: Some(0.0),    // UI slider hint
    soft_max: Some(10.0),   // UI slider hint
}
```

- **Hard**: Value MUST be in range (validation fails otherwise)
- **Soft**: UI slider range (user can type beyond)

### 3. Mode for Branching

Discriminated unions as first-class citizens:

```rust
Mode {
    variants: {
        "none" => [],
        "basic" => [username, password],
        "oauth" => [client_id, client_secret, token_url],
    }
}
```

Output:
```json
{
    "auth": {
        "mode": "basic",
        "value": { "username": "...", "password": "..." }
    }
}
```

### 4. UI Separation via Feature Flags

```toml
[features]
default = []
ui = []           # UI metadata (placeholders, tooltips)
i18n = ["ui"]     # Localization support
```

- Core library has zero UI dependencies
- UI concerns are optional
- Works headless (servers, CLI)

### 5. User-Managed i18n

Library provides keys, user provides translations:

```rust
// Library
Text::builder("host")
    .label("Host")           // English fallback
    .fluent_id("db-host")    // Translation key
    .build()

// User's locales/ru/app.ftl
db-host-label = Хост базы данных
```

**Benefits:**
- No embedded translations in library
- User controls all languages
- No binary bloat

---

## Processing Pipeline

```
User Input
    |
    v
+------------------+
|  1. TRANSFORM    |  <- Coerce value (clamp, round, normalize)
+------------------+
    |
    v
+------------------+
|  2. VALIDATE     |  <- Check constraints (sync)
+------------------+
    |
    v
+------------------+
|  3. VALIDATE     |  <- External checks (async)
|     ASYNC        |
+------------------+
    |
    v
+------------------+
|  4. SET VALUE    |  <- Store in context
+------------------+
    |
    v
+------------------+
|  5. NOTIFY       |  <- Emit change events
+------------------+
```

---

## Event System

```rust
pub enum ParameterEvent {
    BeforeChange { key, old_value, new_value },
    AfterChange { key, old_value, new_value },
    ValidationStarted { key },
    ValidationPassed { key },
    ValidationFailed { key, errors },
    Touched { key },
    Reset { key, value },
    VisibilityChanged { key, visible },
    EnabledChanged { key, enabled },
    ActionTriggered { key, timestamp },
}
```

---

## Performance Optimizations

| Technique | Benefit |
|-----------|---------|
| `SmartString<LazyCompact>` | Strings <23 bytes on stack |
| `Arc<[Value]>` | Immutable arrays, cheap cloning |
| `Arc<HashMap>` | Immutable objects, shared |
| Const generics | `[f64; 3]` on stack, no heap |
| Thread-local regex cache | Avoid recompilation |
| Lazy expression compilation | Compile on first use |
| Fast path checks | Skip empty transformer/validator lists |

---

## Comparison with Industry Systems

| Feature | Blender | Unreal | n8n | Qt | Houdini | Nebula |
|---------|---------|--------|-----|----|---------| -------|
| Type Safety | - | ~ | - | ~ | - | **Yes** |
| Compile-Time | - | ~ | - | ~ | - | **Yes** |
| Subtype+Unit | Yes | Yes | - | - | ~ | **Yes** |
| Soft/Hard | Yes | Yes | - | - | Yes | **Yes** |
| Mode/Branch | - | - | Yes | - | - | **Yes** |
| Expressions | Yes | ~ | Yes | ~ | Yes | **Yes** |
| Reset | - | - | - | Yes | Yes | **Yes** |
| Zero-Cost | - | ~ | - | ~ | - | **Yes** |

---

## Architectural Invariants

These rules MUST NOT be broken:

1. **Node hierarchy is strict**
   - Group → Layout, Decoration, Container, Leaf (can contain everything)
   - Layout → Decoration, Container, Leaf (NOT Layout, NOT Group)
   - Container → Decoration, Container, Leaf (NOT Layout, NOT Group)
   - Decoration → nothing (terminal, display-only)
   - Leaf → nothing (terminal, with value)
2. **Schema is ALWAYS immutable** - Runtime state in Context
3. **UI hints NEVER affect validation** - Soft constraints are hints only
4. **Mode is structural, not scalar** - Produces `{mode, value}` object
5. **Group and Layout have no own Value** - Only delegate via ValueAccess API
6. **Decoration has no Value and no ValueAccess** - Pure display element
7. **Container and Leaf have own Value** - Container also has ValueAccess
8. **Only Decoration and Leaf have no children** - All others can contain children
9. **No "god object"** - Keep types focused and composable
10. **Clean names** - Types use `Text`, not `TextParameter`
