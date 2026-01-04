# Advanced Patterns

Industry patterns from Qt, Houdini, TouchDesigner, WPF, Node-RED, Airflow, and more.

---

## 1. Reset Functionality (Qt Pattern)

Reset parameters to their default values.

```rust
// Parameter trait includes reset support
pub trait Parameter {
    fn default_value(&self) -> Option<Self::Value>;
    
    fn reset(&self) -> Self::Value {
        self.default_value().expect("Parameter must have default for reset")
    }
}

// Context methods
impl Context {
    /// Reset single parameter to default
    pub fn reset(&mut self, key: &str) -> Result<()> {
        let param = self.schema.get_parameter(key)?;
        let default = param.reset();
        self.set_value(key, default)
    }
    
    /// Reset all parameters
    pub fn reset_all(&mut self) -> Result<()> {
        for param in self.schema.parameters() {
            if let Some(default) = param.default_value() {
                self.set_value(param.metadata().key(), default)?;
            }
        }
        Ok(())
    }
}
```

---

## 2. Change Notifications (Qt/WPF Pattern)

Subscribe to parameter changes with events.

```rust
pub enum ParameterEvent {
    BeforeChange { key: String, old_value: Value, new_value: Value },
    AfterChange { key: String, old_value: Value, new_value: Value },
    ValidationStarted { key: String },
    ValidationPassed { key: String },
    ValidationFailed { key: String, errors: Vec<ValidationError> },
    Touched { key: String },
    Reset { key: String, value: Value },
    VisibilityChanged { key: String, visible: bool },
    EnabledChanged { key: String, enabled: bool },
    ActionTriggered { key: String, timestamp: Instant },
}

// Subscribe to specific parameter
let mut events = context.subscribe_parameter("username");
while let Ok(event) = events.recv().await {
    match event {
        ParameterEvent::AfterChange { key, new_value, .. } => {
            println!("{} changed to {:?}", key, new_value);
        }
        _ => {}
    }
}

// Subscribe to all events
let mut all_events = context.subscribe_all();
```

---

## 3. Conditional Visibility & Enable (Houdini Pattern)

Show/hide and enable/disable based on other parameters.

### DisplayCondition (16 Types)

```rust
/// A condition that determines whether a parameter should be displayed.
pub enum DisplayCondition {
    // === Value Comparisons ===
    Equals(Value),              // Value equals specified value
    NotEquals(Value),           // Value does not equal specified value
    
    // === Null/Set Checks ===
    IsSet,                      // Value is not null
    IsNull,                     // Value is null
    
    // === Emptiness Checks ===
    IsEmpty,                    // Empty string, array, object, or null
    IsNotEmpty,                 // Not empty
    
    // === Boolean Checks ===
    IsTrue,                     // Boolean value is true
    IsFalse,                    // Boolean value is false
    
    // === Numeric Comparisons ===
    GreaterThan(f64),           // Value > threshold
    LessThan(f64),              // Value < threshold
    InRange { min: f64, max: f64 }, // min <= value <= max
    
    // === String Operations ===
    Contains(String),           // String contains substring
    StartsWith(String),         // String starts with prefix
    EndsWith(String),           // String ends with suffix
    
    // === Membership ===
    OneOf(Vec<Value>),          // Value is one of specified values
    
    // === Validation State ===
    IsValid,                    // Field has passed validation
    IsInvalid,                  // Field has failed validation
}
```

### DisplayRule and DisplayRuleSet

```rust
/// A display rule that checks a specific field against a condition
pub struct DisplayRule {
    pub field: Key,
    pub condition: DisplayCondition,
}

impl DisplayRule {
    pub fn when(field: impl Into<Key>, condition: DisplayCondition) -> Self;
}

/// Combine rules with logical operators
pub enum DisplayRuleSet {
    Single(DisplayRule),        // Single rule
    All(Vec<DisplayRuleSet>),   // All must pass (AND)
    Any(Vec<DisplayRuleSet>),   // Any must pass (OR)
    Not(Box<DisplayRuleSet>),   // Invert result (NOT)
}

impl DisplayRuleSet {
    pub fn all(rules: impl IntoIterator<Item = impl Into<DisplayRuleSet>>) -> Self;
    pub fn any(rules: impl IntoIterator<Item = impl Into<DisplayRuleSet>>) -> Self;
    pub fn not(rule: impl Into<DisplayRuleSet>) -> Self;
}
```

### DisplayContext with Validation State

```rust
/// Context for evaluating display conditions
pub struct DisplayContext {
    values: ParameterValues,
    validation: HashMap<Key, bool>,  // true = valid
}

impl DisplayContext {
    pub fn new() -> Self;
    pub fn from_values(values: ParameterValues) -> Self;
    
    // Builder pattern
    pub fn with_value(self, key: impl Into<Key>, value: Value) -> Self;
    pub fn with_validation(self, key: impl Into<Key>, is_valid: bool) -> Self;
    
    // Check validation state
    pub fn is_valid(&self, key: &str) -> bool;
    pub fn is_invalid(&self, key: &str) -> bool;
}
```

### ParameterDisplay Configuration

```rust
pub struct ParameterDisplay {
    show_when: Option<DisplayRuleSet>,  // Conditions to show
    hide_when: Option<DisplayRuleSet>,  // Conditions to hide (priority!)
}

impl ParameterDisplay {
    pub fn new() -> Self;
    
    // Add conditions
    pub fn show_when(self, rule: impl Into<DisplayRuleSet>) -> Self;
    pub fn hide_when(self, rule: impl Into<DisplayRuleSet>) -> Self;
    
    // Convenience methods
    pub fn show_when_equals(self, field: impl Into<Key>, value: Value) -> Self;
    pub fn show_when_true(self, field: impl Into<Key>) -> Self;
    pub fn hide_when_equals(self, field: impl Into<Key>, value: Value) -> Self;
    pub fn hide_when_true(self, field: impl Into<Key>) -> Self;
    
    // Validation-based visibility
    pub fn show_when_valid(self, field: impl Into<Key>) -> Self;
    pub fn show_when_invalid(self, field: impl Into<Key>) -> Self;
    pub fn hide_when_valid(self, field: impl Into<Key>) -> Self;
    pub fn hide_when_invalid(self, field: impl Into<Key>) -> Self;
    
    // Evaluate
    pub fn should_display(&self, ctx: &DisplayContext) -> bool;
    
    // Get dependencies for reactive updates
    pub fn dependencies(&self) -> Vec<Key>;
}
```

### Usage Examples

```rust
// Show API key field only when auth type is "api_key"
let display = ParameterDisplay::new()
    .show_when_equals(key("auth_type"), Value::text("api_key"));

// Show advanced options when enabled AND level > 10
let display = ParameterDisplay::new()
    .show_when(DisplayRuleSet::all([
        DisplayRule::when(key("advanced"), DisplayCondition::IsTrue),
        DisplayRule::when(key("level"), DisplayCondition::GreaterThan(10.0)),
    ]));

// Show either admin OR superuser
let display = ParameterDisplay::new()
    .show_when(DisplayRuleSet::any([
        DisplayRule::when(key("role"), DisplayCondition::Equals(Value::text("admin"))),
        DisplayRule::when(key("superuser"), DisplayCondition::IsTrue),
    ]));

// Hide when disabled (NOT pattern)
let display = ParameterDisplay::new()
    .hide_when_true(key("disabled"));

// Show error message only when email is invalid
let display = ParameterDisplay::new()
    .show_when_invalid(key("email"));

// Show confirmation field only when password is valid
let display = ParameterDisplay::new()
    .show_when_valid(key("password"));

// Complex: show when valid AND not in maintenance mode
let display = ParameterDisplay::new()
    .show_when(DisplayRule::when(key("email"), DisplayCondition::IsValid))
    .hide_when(DisplayRule::when(key("maintenance"), DisplayCondition::IsTrue));

// Evaluate with context
let ctx = DisplayContext::new()
    .with_value(key("auth_type"), Value::text("api_key"))
    .with_validation(key("email"), true);

if display.should_display(&ctx) {
    render_parameter();
}
```

### Priority: hide_when Takes Precedence

```rust
let display = ParameterDisplay::new()
    .show_when_true(key("enabled"))
    .hide_when_true(key("maintenance"));

let ctx = DisplayContext::new()
    .with_value(key("enabled"), Value::boolean(true))
    .with_value(key("maintenance"), Value::boolean(true));

// hide_when is checked first, so parameter is hidden
// even though show_when condition is met
assert!(!display.should_display(&ctx));
```

---

## 4. Parameter Organization (TouchDesigner/Houdini Pattern)

Organize parameters into pages, groups, and subgroups.

```rust
pub struct ParameterLocation {
    pub page: Option<String>,
    pub group: Option<String>,
    pub subgroup: Option<String>,
    pub order: i32,
}

// Usage
NumberParameter::distance("pos_x")
    .label("X Position")
    .page("Transform")
    .group("Position")
    .order(0)
    .build()

NumberParameter::distance("pos_y")
    .label("Y Position")
    .page("Transform")
    .group("Position")
    .order(1)
    .build()

VectorParameter::color_rgba("color")
    .label("Color")
    .page("Appearance")
    .order(0)
    .build()
```

---

## 5. Storage Flags (Qt Pattern)

Control serialization behavior.

```rust
bitflags! {
    pub struct Flags: u32 {
        // ... existing flags ...
        
        const STORED      = 1 << 8;   // Save to file (default: true)
        const DESIGNABLE  = 1 << 9;   // Visible in designer
        const SCRIPTABLE  = 1 << 10;  // Accessible via scripts
        const USER        = 1 << 11;  // Primary user property
        const CONSTANT    = 1 << 12;  // Value never changes
    }
}

// Don't serialize temporary values
TextParameter::builder("cached_value")
    .not_stored()
    .build()

// Session-only identifier
TextParameter::builder("session_id")
    .constant()
    .not_stored()
    .build()

// Serialization respects flags
impl Context {
    pub fn serialize(&self) -> HashMap<String, Value> {
        self.schema.parameters()
            .filter(|p| p.flags().contains(Flags::STORED))
            .map(|p| (p.key(), self.get_value(p.key())))
            .collect()
    }
}
```

---

## 6. Action Parameters (TouchDesigner Pattern)

Trigger actions without storing values.

```rust
pub struct ActionParameter {
    pub metadata: Metadata,
    pub display: Option<DisplayRule>,
}

// Quick constructors
impl ActionParameter {
    pub fn reset(key: &str) -> Self { ... }
    pub fn reload(key: &str) -> Self { ... }
    
    pub fn builder(key: &str) -> ActionParameterBuilder { ... }
}

// Usage
Schema::new()
    .with(ActionParameter::reset("reset_all"))
    .with(ActionParameter::reload("reload_config"))
    .with(ActionParameter::builder("export")
        .label("Export Data")
        .description("Export configuration to file")
        .build())

// Trigger action
context.trigger("reset_all")?;
```

---

## 7. Template Support (Airflow Pattern)

Enable template expressions in parameter values.

```rust
bitflags! {
    pub struct Flags: u32 {
        const TEMPLATABLE = 1 << 13;  // Supports templates
    }
}

pub struct TemplateContext {
    pub variables: HashMap<String, Value>,
    pub functions: HashMap<String, TemplateFn>,
}

// Mark parameter as templatable
TextParameter::builder("message")
    .templatable()
    .default("Hello {{user.name}}!")
    .build()

// Render at runtime
let mut template_ctx = TemplateContext::new();
template_ctx.add_variable("user", user_value);

let rendered = context.get_rendered("message", &template_ctx)?;
// Result: "Hello Alice!"
```

---

## 8. Union Parameters (Node-RED TypedInput Pattern)

Parameter value can be one of multiple types.

```rust
pub struct UnionParameter {
    pub metadata: Metadata,
    pub variants: Vec<UnionVariant>,
    pub default_variant: String,
}

pub struct UnionVariant {
    pub id: String,
    pub label: String,
    pub parameter: Box<dyn ParameterAny>,
}

// Usage: value can be literal OR expression OR reference
UnionParameter::builder("value")
    .label("Value")
    .variant("literal", "Literal", Box::new(
        TextParameter::builder("literal").build()
    ))
    .variant("expression", "Expression", Box::new(
        TextParameter::builder("expression")
            .subtype(TextSubtype::Expression)
            .build()
    ))
    .variant("reference", "Reference", Box::new(
        TextParameter::builder("reference")
            .subtype(TextSubtype::JsonPath)
            .build()
    ))
    .build()

// Serialized as:
// { "type": "expression", "value": "{{$json.data}}" }
```

---

## 9. Dynamic Sources (Argo ValueFrom Pattern)

Load values from external sources.

```rust
pub enum ValueSource {
    Static(Value),
    FilePath(String),
    Expression(String),
    ParameterRef(String),
    Environment(String),
    Provider(Arc<dyn ValueProvider>),
}

pub struct DynamicParameter {
    pub metadata: Metadata,
    pub source: ValueSource,
    pub fallback: Option<Value>,
}

// Value from file
DynamicParameter::from_file("config", "/etc/app/config.json")

// Value from expression
DynamicParameter::from_expression("doubled", "{{count}} * 2")

// Value from other parameter
DynamicParameter::from_parameter("total", "subtotal")

// Value from environment
DynamicParameter::from_env("api_url", "API_URL")
```

---

## 10. Expression Engine

First-class expression support with dependency tracking.

```rust
pub enum Value {
    // ... other variants ...
    Expression {
        template: String,
        compiled: Option<Arc<CompiledExpression>>,
    },
}

pub struct CompiledExpression {
    pub ast: Ast,
    pub dependencies: Vec<String>,  // Auto-tracked
}

// Expression syntax
// {{$json.user.name}}           - Access data
// {{$json.age ?? 18}}           - Default value
// {{len($json.items)}}          - Function call
// "Hello {{$json.name}}!"       - String interpolation

// Built-in functions
// len(), upper(), lower(), trim(), json(), get()

// Lazy compilation
impl ExpressionEngine {
    pub fn evaluate(&self, template: &str, context: &Context) -> Result<Value> {
        let compiled = self.compile(template)?;  // Cached
        self.evaluate_compiled(&compiled, context)
    }
}
```

---

## 11. Snapshot/Restore (Undo/Redo)

Support undo/redo operations.

```rust
pub struct Snapshot {
    values: HashMap<String, Value>,
    timestamp: Instant,
}

impl Context {
    /// Take snapshot of current state
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            values: self.values.clone(),
            timestamp: Instant::now(),
        }
    }
    
    /// Restore from snapshot
    pub fn restore(&mut self, snapshot: &Snapshot) {
        self.values = snapshot.values.clone();
        
        // Emit batch update event
        self.emit_event(ParameterEvent::BatchUpdate {
            keys: self.values.keys().cloned().collect(),
        });
    }
}

// Usage
let before = context.snapshot();

// Make changes...
context.set_value("name", new_value)?;

// Undo
context.restore(&before);
```

---

## 12. Complete Example: Database Connection

```rust
fn database_connection_schema() -> Schema {
    Schema::new()
        // Connection mode (discriminated union)
        .with(ModeParameter::builder("connection")
            .label("Connection")
            .page("Connection")
            
            .variant("uri", "Connection String", Schema::new()
                .with(TextParameter::builder("uri")
                    .label("Connection URI")
                    .subtype(TextSubtype::Url)
                    .required()
                    .templatable()
                    .placeholder("postgresql://user:pass@localhost:5432/db")
                    .build())
            )
            
            .variant("details", "Connection Details", Schema::new()
                .with(TextParameter::builder("host")
                    .label("Host")
                    .required()
                    .default("localhost")
                    .page("Connection")
                    .group("Server")
                    .order(0)
                    .build())
                
                .with(NumberParameter::builder::<i64>("port")
                    .label("Port")
                    .range(1, 65535)
                    .default(5432)
                    .page("Connection")
                    .group("Server")
                    .order(1)
                    .build())
                
                .with(TextParameter::builder("database")
                    .label("Database")
                    .required()
                    .page("Connection")
                    .group("Server")
                    .order(2)
                    .build())
                
                .with(TextParameter::builder("username")
                    .label("Username")
                    .required()
                    .page("Connection")
                    .group("Credentials")
                    .order(0)
                    .build())
                
                .with(TextParameter::password("password")
                    .label("Password")
                    .required()
                    .page("Connection")
                    .group("Credentials")
                    .order(1)
                    .build())
            )
            .build())
        
        // SSL options (conditional)
        .with(BoolParameter::builder("use_ssl")
            .label("Use SSL")
            .default(true)
            .page("Security")
            .build())
        
        .with(TextParameter::file("ssl_cert")
            .label("SSL Certificate")
            .page("Security")
            .display_when(DisplayRule::show_when(
                Condition::Equals {
                    key: "use_ssl".into(),
                    value: Value::Bool(true),
                }
            ))
            .build())
        
        // Timeouts
        .with(NumberParameter::duration_seconds("connection_timeout")
            .label("Connection Timeout")
            .page("Advanced")
            .group("Timeouts")
            .range(1.0, 300.0)
            .default(30.0)
            .build())
        
        .with(NumberParameter::duration_seconds("query_timeout")
            .label("Query Timeout")
            .page("Advanced")
            .group("Timeouts")
            .range(1.0, 3600.0)
            .default(300.0)
            .build())
        
        // Pool settings
        .with(NumberParameter::builder::<i64>("pool_size")
            .label("Pool Size")
            .page("Advanced")
            .group("Performance")
            .range(1, 100)
            .default(10)
            .build())
        
        // Actions
        .with(ActionParameter::builder("test_connection")
            .label("Test Connection")
            .page("Actions")
            .build())
        
        .with(ActionParameter::builder("clear_pool")
            .label("Clear Pool")
            .page("Actions")
            .build())
}
```

---

## Industry Comparison

| Pattern | Qt | Houdini | WPF | Airflow | Node-RED | Nebula |
|---------|-----|---------|-----|---------|----------|--------|
| Reset | Yes | Yes | - | - | - | **Yes** |
| Notifications | Yes | Yes | Yes | - | - | **Yes** |
| Visibility | - | Yes | - | - | - | **Yes** |
| Pages/Groups | - | Yes | - | - | - | **Yes** |
| Storage Flags | Yes | - | - | - | - | **Yes** |
| Actions | - | Yes | - | - | - | **Yes** |
| Templates | - | - | - | Yes | - | **Yes** |
| Union Types | - | - | - | - | Yes | **Yes** |
| Dynamic Sources | - | - | - | - | - | **Yes** |
| Expressions | - | Yes | Yes | Yes | Yes | **Yes** |
| Undo/Redo | - | Yes | - | - | - | **Yes** |

Nebula combines ALL patterns from across the industry!
