# Nebula Parameter System

Type-safe, production-ready parameter system for workflow automation and node-based editors.

---

## Quick Start

### Define Parameters

```rust
use nebula_parameter::prelude::*;

let schema = Schema::new()
    .with(TextParameter::builder("name")
        .label("Name")
        .required()
        .min_length(1)
        .build())
    
    .with(NumberParameter::builder::<f64>("opacity")
        .label("Opacity")
        .subtype(NumberSubtype::Factor)
        .range(0.0, 1.0)
        .default(1.0)
        .build())
    
    .with(EnumParameter::builder("method")
        .label("Method")
        .option("GET", "GET")
        .option("POST", "POST")
        .default("GET")
        .build());
```

### Use Context

```rust
let mut context = Context::new(schema);

// Set values
context.set_value("name", Value::Text("Hello".into()))?;
context.set_value("opacity", Value::Float(0.5))?;

// Get values
let name: String = context.get_string("name")?;
let opacity: f64 = context.get_float("opacity")?;

// Validate
if context.validate_all().await.is_ok() {
    println!("All valid!");
}
```

---

## Documentation

### Core Concepts

| Document | Description |
|----------|-------------|
| [01-ARCHITECTURE](01-ARCHITECTURE.md) | Core design decisions and philosophy |
| [02-TYPE-SYSTEM](02-TYPE-SYSTEM.md) | Parameter types and value system |
| [03-SUBTYPES-AND-UNITS](03-SUBTYPES-AND-UNITS.md) | Complete subtype/unit reference |
| [04-TRANSFORMERS](04-TRANSFORMERS.md) | Value transformation (coercion) |
| [05-VALIDATION](05-VALIDATION.md) | Validation pipeline |

### UI & User Experience

| Document | Description |
|----------|-------------|
| [06-UI-AND-I18N](06-UI-AND-I18N.md) | UI metadata and localization |
| [07-ADVANCED-PATTERNS](07-ADVANCED-PATTERNS.md) | Industry patterns guide |

### Reactive & State Management

| Document | Description |
|----------|-------------|
| [08-EVENT-SYSTEM](08-EVENT-SYSTEM.md) | Reactive patterns and EventBus |
| [09-UNDO-REDO](09-UNDO-REDO.md) | Command pattern for undo/redo |
| [12-SCHEMA-CONTEXT](12-SCHEMA-CONTEXT.md) | Schema vs Context architecture |

### Reference

| Document | Description |
|----------|-------------|
| [10-DIAGRAMS](10-DIAGRAMS.md) | Architecture diagrams and visuals |
| [11-FLAGS-REFERENCE](11-FLAGS-REFERENCE.md) | Flags complete reference |
| [13-INDUSTRY-REFERENCE](13-INDUSTRY-REFERENCE.md) | Analysis of 18 industry systems |

### Practical Guides

| Document | Description |
|----------|-------------|
| [14-API-EXAMPLES](14-API-EXAMPLES.md) | 8 real-world usage examples |
| [15-QUICK-REFERENCE](15-QUICK-REFERENCE.md) | Cheat sheet for common tasks |
| [16-SUBTYPE-GUIDE](16-SUBTYPE-GUIDE.md) | How to choose the right subtype |
| [17-DESIGN-DECISIONS](17-DESIGN-DECISIONS.md) | Key architectural decisions with rationale |
| [18-ROADMAP](18-ROADMAP.md) | Implementation roadmap and milestones |

---

## Key Features

### Type Safety

```rust
const USERNAME: PropertyKey<String> = PropertyKey::new(1);
const AGE: PropertyKey<i64> = PropertyKey::new(2);

let name: String = context.get(USERNAME)?;  // Compile-time checked
let age: i64 = context.get(AGE)?;           // Compile-time checked
```

### 9 Core Types

| Type | Purpose |
|------|---------|
| `TextParameter` | Strings with 60+ subtypes |
| `NumberParameter<T>` | Numbers with units |
| `BoolParameter` | Boolean toggles |
| `VectorParameter<T, N>` | Fixed-size arrays |
| `EnumParameter` | Static options |
| `DynamicEnumParameter` | Async-loaded options |
| `ObjectParameter` | Nested structures |
| `ListParameter` | Dynamic arrays |
| `ModeParameter` | Discriminated unions |

### Subtype + Unit Pattern

```rust
NumberParameter::builder::<f64>("height")
    .subtype(NumberSubtype::Distance)  // WHAT it is
    .unit(NumberUnit::Length)          // HOW to measure
    .build()

// American user sees: "5.9 ft"
// European user sees: "1.8 m"
// Stored as: 1.8 (meters)
```

### Mode Parameter (Discriminated Unions)

```rust
ModeParameter::builder("auth")
    .variant("none", "No Auth", Schema::empty())
    .variant("basic", "Basic", Schema::new()
        .with(username_param)
        .with(password_param))
    .variant("oauth", "OAuth", Schema::new()
        .with(client_id_param)
        .with(client_secret_param))
    .build()
```

### Transformers

```rust
NumberParameter::builder::<f64>("angle")
    .transformer(RoundTransformer { step: 15.0 })
    .transformer(ModuloTransformer { modulo: 360.0 })
    .build()

// Input: 373.0 -> Round: 375.0 -> Modulo: 15.0
```

### Parameter Flags

```rust
TextParameter::password("api_key")
    .flags(Flags::SENSITIVE | Flags::WRITE_ONLY | Flags::SKIP_SAVE)
    .build()

// Or use convenience methods:
TextParameter::password("api_key")
    .sensitive()
    .write_only()
    .skip_save()
    .build()
```

### Validation

```rust
TextParameter::builder("email")
    .subtype(TextSubtype::Email)  // Built-in validation
    .required()
    .async_validator(Arc::new(EmailExistsValidator::new(db)))
    .build()
```

### Display Conditions

```rust
TextParameter::builder("ssl_cert")
    .display_when(DisplayRule::show_when(
        Condition::Equals {
            key: "protocol".into(),
            value: Value::Text("https".into()),
        }
    ))
    .build()
```

### Schema vs Context

```rust
// Schema is immutable, shareable
let schema = Arc::new(Schema::new()
    .with(TextParameter::builder("name").build())
);

// Context holds mutable state
let mut context_a = Context::new(schema.clone());
let mut context_b = Context::new(schema.clone());

// Same schema, different values!
context_a.set_value("name", "Alice".into())?;
context_b.set_value("name", "Bob".into())?;
```

---

## Feature Flags

```toml
[features]
default = []
ui = []           # UI metadata
i18n = ["ui"]     # Localization
```

---

## Reading Guide

### Quick Start (30 min)
1. This README
2. [01-ARCHITECTURE](01-ARCHITECTURE.md) - Design overview
3. [02-TYPE-SYSTEM](02-TYPE-SYSTEM.md) - Parameter types

### Complete Guide (3-4 hours)
1. [01-ARCHITECTURE](01-ARCHITECTURE.md)
2. [02-TYPE-SYSTEM](02-TYPE-SYSTEM.md)
3. [03-SUBTYPES-AND-UNITS](03-SUBTYPES-AND-UNITS.md)
4. [04-TRANSFORMERS](04-TRANSFORMERS.md)
5. [05-VALIDATION](05-VALIDATION.md)
6. [06-UI-AND-I18N](06-UI-AND-I18N.md)
7. [07-ADVANCED-PATTERNS](07-ADVANCED-PATTERNS.md)
8. [08-EVENT-SYSTEM](08-EVENT-SYSTEM.md)
9. [09-UNDO-REDO](09-UNDO-REDO.md)
10. [10-DIAGRAMS](10-DIAGRAMS.md)
11. [11-FLAGS-REFERENCE](11-FLAGS-REFERENCE.md)
12. [12-SCHEMA-CONTEXT](12-SCHEMA-CONTEXT.md)
13. [13-INDUSTRY-REFERENCE](13-INDUSTRY-REFERENCE.md)
14. [14-API-EXAMPLES](14-API-EXAMPLES.md)
15. [15-QUICK-REFERENCE](15-QUICK-REFERENCE.md)
16. [16-SUBTYPE-GUIDE](16-SUBTYPE-GUIDE.md)
17. [17-DESIGN-DECISIONS](17-DESIGN-DECISIONS.md)
18. [18-ROADMAP](18-ROADMAP.md)

### By Topic

**Type System:**
- [02-TYPE-SYSTEM](02-TYPE-SYSTEM.md)
- [03-SUBTYPES-AND-UNITS](03-SUBTYPES-AND-UNITS.md)

**Data Processing:**
- [04-TRANSFORMERS](04-TRANSFORMERS.md)
- [05-VALIDATION](05-VALIDATION.md)

**UI & i18n:**
- [06-UI-AND-I18N](06-UI-AND-I18N.md)

**Reactive & State:**
- [08-EVENT-SYSTEM](08-EVENT-SYSTEM.md)
- [09-UNDO-REDO](09-UNDO-REDO.md)
- [12-SCHEMA-CONTEXT](12-SCHEMA-CONTEXT.md)

**Advanced:**
- [07-ADVANCED-PATTERNS](07-ADVANCED-PATTERNS.md)

**Reference:**
- [10-DIAGRAMS](10-DIAGRAMS.md)
- [11-FLAGS-REFERENCE](11-FLAGS-REFERENCE.md)
- [13-INDUSTRY-REFERENCE](13-INDUSTRY-REFERENCE.md)

**Practical:**
- [14-API-EXAMPLES](14-API-EXAMPLES.md)
- [15-QUICK-REFERENCE](15-QUICK-REFERENCE.md)
- [16-SUBTYPE-GUIDE](16-SUBTYPE-GUIDE.md)

**Meta:**
- [17-DESIGN-DECISIONS](17-DESIGN-DECISIONS.md)
- [18-ROADMAP](18-ROADMAP.md)

---

## Industry Comparison

| Feature | Blender | Unreal | n8n | Qt | Houdini | Nebula |
|---------|---------|--------|-----|----|---------| -------|
| Type Safety | - | ~ | - | ~ | - | **Yes** |
| Subtype+Unit | Yes | Yes | - | - | ~ | **Yes** |
| Soft/Hard | Yes | Yes | - | - | Yes | **Yes** |
| Mode/Branch | - | - | Yes | - | - | **Yes** |
| Expressions | Yes | ~ | Yes | ~ | Yes | **Yes** |
| Undo/Redo | - | Yes | - | - | Yes | **Yes** |
| Event System | - | Yes | - | Yes | - | **Yes** |
| Flags System | ~ | Yes | - | Yes | ~ | **Yes** |
| Schema/Context | ~ | Yes | - | ~ | ~ | **Yes** |
| Zero-Cost | - | ~ | - | ~ | - | **Yes** |

Nebula combines the best features from across the industry with Rust's type safety and zero-cost abstractions.

---

## Architecture Summary

```
┌─────────────────────────────────────────────────────────┐
│  SCHEMA (Immutable, Arc-shared)                         │
│  - Parameter definitions                                │
│  - Metadata, Flags, Constraints                         │
│  - Validators, Transformers                             │
└─────────────────────────────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────┐
│  CONTEXT (Mutable, per-instance)                        │
│  - Current values                                       │
│  - State (dirty, touched, errors)                       │
│  - Event bus, History manager                           │
└─────────────────────────────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────┐
│  UI LAYER (Optional, feature-gated)                     │
│  - Widget hints                                         │
│  - Localization (i18n)                                  │
│  - Display formatting                                   │
└─────────────────────────────────────────────────────────┘
```
