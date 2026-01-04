# Schema vs Context Architecture

Guide to the separation between immutable Schema (definition) and mutable Context (runtime state).

---

## Core Principle

**Never mix schema definition with runtime state!**

```
Schema (Definition)     →  Immutable, shareable
Context (State)         →  Mutable, per-instance
```

This separation is a fundamental design pattern used by:
- React (Component definition vs State)
- Blender (PropertyGroup vs PropertyValue)
- Unity (ScriptableObject vs Instance)
- SQL (Table schema vs Row data)
- GraphQL (Schema vs Resolver)
- TypeScript (Interface vs Object)

---

## Architecture Layers

```
┌─────────────────────────────────────────────────────────┐
│  SCHEMA LAYER (Immutable)                               │
│  - Parameter definitions                                │
│  - Metadata (key, label, description)                   │
│  - Flags (REQUIRED, READONLY, etc.)                     │
│  - Validators & Transformers                            │
│  - Default values                                       │
│  - Can be shared via Arc                                │
└─────────────────────────────────────────────────────────┘
                          │
                          │ Used by (Arc<Schema>)
                          ▼
┌─────────────────────────────────────────────────────────┐
│  CONTEXT LAYER (Mutable)                                │
│  - Current values                                       │
│  - State flags (dirty, touched, valid)                  │
│  - Validation errors                                    │
│  - Per-instance, per-context                            │
└─────────────────────────────────────────────────────────┘
```

---

## Implementation

### Schema Layer (Immutable)

```rust
/// Text parameter definition (schema).
/// Immutable and can be shared between multiple contexts.
#[derive(Debug, Clone)]
pub struct TextParameter {
    /// Parameter metadata
    metadata: Arc<Metadata>,
    
    /// Configuration flags (REQUIRED, READONLY, etc.)
    flags: Flags,
    
    /// Subtype (semantic meaning)
    subtype: TextSubtype,
    
    /// Constraints
    min_length: Option<usize>,
    max_length: Option<usize>,
    pattern: Option<Arc<regex::Regex>>,
    
    /// Default value (schema-level)
    default: Option<SmartString<LazyCompact>>,
    
    /// Value transformers
    transformers: Vec<Arc<dyn Transformer<String>>>,
    
    /// Validators
    validators: Vec<Arc<dyn Validator<String>>>,
    
    // NO runtime state here!
    // NO current_value, is_dirty, errors, etc.
}

/// Schema - collection of parameters
#[derive(Debug, Clone)]
pub struct Schema {
    parameters: Vec<Arc<dyn Parameter>>,
    // Immutable after construction
    // Can be shared via Arc<Schema>
}
```

### Context Layer (Mutable)

```rust
/// Runtime context - holds mutable state
#[derive(Debug)]
pub struct Context {
    /// Schema (shared, immutable)
    schema: Arc<Schema>,
    
    /// Current values (mutable)
    values: HashMap<Key, Value>,
    
    /// Runtime state (mutable)
    states: HashMap<Key, ParameterState>,
    
    /// Event bus for change notifications
    event_bus: EventBus,
    
    /// Undo/redo history
    history: HistoryManager,
}

/// Runtime state for a single parameter
#[derive(Debug, Clone, Default)]
pub struct ParameterState {
    /// Value has been modified
    dirty: bool,
    
    /// User has interacted with field
    touched: bool,
    
    /// Current validation errors
    errors: Vec<ValidationError>,
    
    /// Last validation timestamp
    validated_at: Option<Instant>,
}
```

---

## Why This Separation?

### 1. Schema Reusability

One schema, multiple contexts:

```rust
// Define schema once
let schema = Arc::new(Schema::new()
    .with_parameter(TextParameter::builder("name")
        .required()
        .build())
);

// Context 1 (User A)
let mut context_a = Context::new(schema.clone());
context_a.set_value("name", "Alice".into())?;

// Context 2 (User B)
let mut context_b = Context::new(schema.clone());
context_b.set_value("name", "Bob".into())?;

// Same schema, different runtime state!
assert_eq!(context_a.get_string("name"), Some("Alice"));
assert_eq!(context_b.get_string("name"), Some("Bob"));
```

### 2. Thread Safety

Schema can be shared across threads:

```rust
let schema = Arc::new(create_schema());

// Spawn multiple threads, each with own context
for i in 0..4 {
    let schema_clone = schema.clone();
    thread::spawn(move || {
        let mut context = Context::new((*schema_clone).clone());
        context.set_value("worker_id", i.to_string().into()).unwrap();
        // Each thread has independent state
    });
}
```

### 3. Undo/Redo Support

Snapshots only capture values, not schema:

```rust
impl Context {
    /// Save current state for undo
    pub fn save_snapshot(&mut self) {
        let snapshot = self.values.clone();
        self.history.push(snapshot);
    }
    
    /// Restore previous state
    pub fn undo(&mut self) -> Option<()> {
        let snapshot = self.history.pop()?;
        self.values = snapshot;
        Some(())
    }
}

// Usage
context.set_value("name", "Alice".into())?;
context.save_snapshot();

context.set_value("name", "Bob".into())?;
// name = "Bob"

context.undo();
// name = "Alice" (restored)
```

### 4. Multiple Forms from Same Schema

```rust
// Database connection schema
let db_schema = Arc::new(create_db_connection_schema());

// Production form
let mut prod = Context::new(db_schema.clone());
prod.set_value("host", "prod.db.com".into())?;

// Staging form
let mut staging = Context::new(db_schema.clone());
staging.set_value("host", "staging.db.com".into())?;

// Development form
let mut dev = Context::new(db_schema.clone());
dev.set_value("host", "localhost".into())?;

// Three different contexts, one schema definition
```

### 5. Efficient Serialization

```rust
// Schema: Serialize once (or not at all if static)
// Usually embedded in code, not serialized

// Values: Serialize frequently (user data)
let user_data = serde_json::to_string(&context.values)?;
save_to_file("user_settings.json", &user_data)?;

// State: Usually transient (not serialized)
// Or serialize separately for form state recovery
```

---

## Context API

### Value Management

```rust
impl Context {
    /// Get current value
    pub fn get_value(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }
    
    /// Get value as specific type
    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.values.get(key)?.as_string()
    }
    
    /// Set value (with transform + validate)
    pub fn set_value(&mut self, key: &str, value: Value) -> Result<()> {
        let param = self.schema.get_parameter(key)?;
        
        // Transform
        let transformed = param.transform(value)?;
        
        // Validate
        param.validate(&transformed)?;
        
        // Store
        let old_value = self.values.insert(key.into(), transformed.clone());
        
        // Update state
        if let Some(state) = self.states.get_mut(key) {
            state.mark_dirty();
            state.mark_touched();
            state.clear_errors();
        }
        
        // Emit event
        self.event_bus.emit(ParameterEvent::ValueChanged {
            key: key.into(),
            old_value,
            new_value: transformed,
        });
        
        Ok(())
    }
    
    /// Reset to default values
    pub fn reset(&mut self) {
        for param in self.schema.parameters() {
            if let Some(default) = param.default_value() {
                self.values.insert(param.key().clone(), default);
            }
        }
        
        // Clear all state
        for state in self.states.values_mut() {
            *state = ParameterState::default();
        }
    }
    
    /// Reset single parameter to default
    pub fn reset_parameter(&mut self, key: &str) -> Result<()> {
        let param = self.schema.get_parameter(key)?;
        
        if let Some(default) = param.default_value() {
            self.values.insert(key.into(), default);
        } else {
            self.values.remove(key);
        }
        
        if let Some(state) = self.states.get_mut(key) {
            *state = ParameterState::default();
        }
        
        Ok(())
    }
}
```

### State Management

```rust
impl Context {
    /// Check if parameter value has changed
    pub fn is_dirty(&self, key: &str) -> bool {
        self.states.get(key).map(|s| s.dirty).unwrap_or(false)
    }
    
    /// Check if any parameter has changes
    pub fn has_changes(&self) -> bool {
        self.states.values().any(|s| s.dirty)
    }
    
    /// Get changed parameter keys
    pub fn dirty_keys(&self) -> Vec<&str> {
        self.states.iter()
            .filter(|(_, s)| s.dirty)
            .map(|(k, _)| k.as_str())
            .collect()
    }
    
    /// Mark parameter as touched (user interacted)
    pub fn mark_touched(&mut self, key: &str) {
        if let Some(state) = self.states.get_mut(key) {
            state.touched = true;
        }
    }
    
    /// Check if user has interacted with parameter
    pub fn is_touched(&self, key: &str) -> bool {
        self.states.get(key).map(|s| s.touched).unwrap_or(false)
    }
    
    /// Save (marks all as clean)
    pub fn save(&mut self) {
        for state in self.states.values_mut() {
            state.dirty = false;
        }
    }
}
```

### Validation

```rust
impl Context {
    /// Validate single parameter
    pub fn validate(&mut self, key: &str) -> bool {
        let Some(param) = self.schema.get_parameter(key) else {
            return false;
        };
        
        let value = self.values.get(key);
        let result = param.validate(value);
        
        if let Some(state) = self.states.get_mut(key) {
            match result {
                Ok(_) => state.clear_errors(),
                Err(e) => state.set_errors(vec![e]),
            }
        }
        
        result.is_ok()
    }
    
    /// Validate all parameters
    pub fn validate_all(&mut self) -> bool {
        let mut all_valid = true;
        
        for param in self.schema.parameters() {
            let key = param.key();
            let value = self.values.get(key);
            let result = param.validate(value);
            
            if let Some(state) = self.states.get_mut(key) {
                match &result {
                    Ok(_) => state.clear_errors(),
                    Err(e) => state.set_errors(vec![e.clone()]),
                }
            }
            
            if result.is_err() {
                all_valid = false;
            }
        }
        
        all_valid
    }
    
    /// Get validation errors for parameter
    pub fn get_errors(&self, key: &str) -> Option<&[ValidationError]> {
        self.states.get(key).map(|s| s.errors.as_slice())
    }
    
    /// Check if parameter is valid
    pub fn is_valid(&self, key: &str) -> bool {
        self.states.get(key).map(|s| s.errors.is_empty()).unwrap_or(true)
    }
}
```

---

## Usage Examples

### Form Example

```rust
// Define schema once
fn user_form_schema() -> Schema {
    Schema::new()
        .with_parameter(
            TextParameter::builder("username")
                .label("Username")
                .required()
                .min_length(3)
                .build()
        )
        .with_parameter(
            TextParameter::email("email")
                .label("Email")
                .required()
                .build()
        )
        .with_parameter(
            TextParameter::password("password")
                .label("Password")
                .required()
                .build()
        )
        .build()
}

// Use in UI
fn render_form() {
    let schema = user_form_schema();
    let mut context = Context::new(Arc::new(schema));
    
    // User fills form
    context.set_value("username", "alice".into())?;
    context.mark_touched("username");
    
    // Validate on submit
    if context.validate_all() {
        println!("Form valid, saving...");
        context.save();
    } else {
        // Show errors
        for (key, state) in context.states() {
            for error in &state.errors {
                println!("{}: {}", key, error);
            }
        }
    }
    
    // Check unsaved changes on close
    if context.has_changes() {
        println!("You have unsaved changes!");
    }
}
```

### Workflow Node Example

```rust
// HTTP Request node schema
let http_schema = Arc::new(Schema::new()
    .with_parameter(
        TextParameter::url("url")
            .label("URL")
            .required()
            .expression()  // Supports {{ expressions }}
            .build()
    )
    .with_parameter(
        EnumParameter::builder("method")
            .label("Method")
            .options(["GET", "POST", "PUT", "DELETE"])
            .default("GET")
            .build()
    )
);

// Each node instance has its own context
let mut node_context = Context::new(http_schema.clone());
node_context.set_value("url", "https://api.example.com".into())?;
node_context.set_value("method", "POST".into())?;

// Execute
if node_context.validate_all() {
    let url = node_context.get_string("url").unwrap();
    let method = node_context.get_string("method").unwrap();
    
    // Make HTTP request...
}
```

### 3D Editor Example

```rust
// Transform property schema
let transform_schema = Arc::new(Schema::new()
    .with_parameter(
        NumberParameter::builder::<f64>("position_x")
            .label("X")
            .animatable()
            .realtime()
            .build()
    )
    .with_parameter(
        NumberParameter::builder::<f64>("position_y")
            .label("Y")
            .animatable()
            .realtime()
            .build()
    )
    .with_parameter(
        NumberParameter::builder::<f64>("position_z")
            .label("Z")
            .animatable()
            .realtime()
            .build()
    )
);

// Each object has its own context
let mut cube_context = Context::new(transform_schema.clone());
cube_context.set_value("position_x", 10.0.into())?;

let mut sphere_context = Context::new(transform_schema.clone());
sphere_context.set_value("position_x", 20.0.into())?;

// Same schema, different values!
```

---

## Anti-Patterns to Avoid

### ❌ Storing Runtime in Schema

```rust
// DON'T DO THIS!
struct TextParameter {
    metadata: Metadata,
    flags: Flags,
    
    // ❌ Runtime state in schema
    current_value: String,
    is_dirty: bool,
    is_touched: bool,
    errors: Vec<ValidationError>,
}

// Problems:
// - Can't share schema between contexts
// - Can't have multiple instances
// - Requires mutation for any state change
// - Thread-unsafe
```

### ❌ Mixing Concerns

```rust
// DON'T DO THIS!
impl TextParameter {
    pub fn set_value(&mut self, value: String) { ... }  // ❌ Mutation!
    pub fn mark_dirty(&mut self) { ... }                // ❌ State management!
}

// DO THIS:
impl Context {
    pub fn set_value(&mut self, key: &str, value: Value) { ... }  // ✅
}
```

### ❌ Direct Schema Mutation

```rust
// DON'T DO THIS!
let mut schema = Schema::new();
schema.add_parameter(...);  // ❌ Mutation after creation

// DO THIS:
let schema = Schema::new()
    .with_parameter(...)     // ✅ Builder pattern
    .with_parameter(...)
    .build();                // Immutable after build
```

---

## Summary

### Schema Layer (Immutable)

- Parameter definitions
- Metadata, flags, constraints
- Validators and transformers
- Default values
- Shareable via `Arc<Schema>`

### Context Layer (Mutable)

- Current values
- Dirty/touched state
- Validation errors
- Event emission
- Undo/redo history

### Benefits

| Benefit | Description |
|---------|-------------|
| **Reusability** | One schema, many contexts |
| **Thread Safety** | Arc-based sharing |
| **Undo/Redo** | Snapshot only values |
| **Memory** | Schema shared, not duplicated |
| **Testing** | Test schema and context separately |
| **Serialization** | Save values, not schema |

### Key Rules

1. **Schema is immutable** after construction
2. **Context holds all mutable state**
3. **Schema can be shared** via Arc
4. **Each instance gets its own Context**
5. **Never put runtime state in Schema**
