# Nebula Parameters - Quick Reference Guide

**Fast lookup for common tasks and patterns**

---

## Table of Contents

1. [Core Types](#core-types)
2. [Subtypes Reference](#subtypes-reference)
3. [Units Reference](#units-reference)
4. [Builder Patterns](#builder-patterns)
5. [Validation](#validation)
6. [Events](#events)
7. [Display Conditions](#display-conditions)
8. [Common Recipes](#common-recipes)

---

## Core Types

### Parameter Types

```rust
ParameterType::Text      // String values
ParameterType::Number    // Numeric (int/float)
ParameterType::Boolean   // True/false
ParameterType::Choice    // Enum/select
ParameterType::Vector    // Fixed-size numeric arrays
ParameterType::Array     // Dynamic arrays
ParameterType::Object    // Nested objects
```

### Value Enum

```rust
Value::None
Value::Text(String)
Value::Integer(i64)
Value::Float(f64)
Value::Boolean(bool)
Value::Array(Vec<Value>)
Value::Object(HashMap<String, Value>)
Value::Vector(Vec<f64>)
```

---

## Subtypes Reference

### TextSubtype Quick Lookup

| Use Case | Subtype | Example |
|----------|---------|---------|
| Generic text | `Generic` | Any text |
| Single line | `SingleLine` | Name, title |
| Multiple lines | `MultiLine` | Description |
| Rich text | `RichText` | Blog post |
| **Code** |
| Source code | `Code` | Generic code |
| With language | `CodeWithLanguage(Rust)` | Rust code |
| JSON data | `Json` | API payload |
| XML data | `Xml` | Config |
| YAML | `Yaml` | Docker compose |
| SQL query | `SqlQuery` | SELECT * FROM |
| **Web** |
| Email | `Email` | user@example.com |
| URL | `Url` | https://... |
| Domain | `Hostname` | example.com |
| IP address | `IpAddress` | 192.168.1.1 |
| **Files** |
| File path | `FilePath` | /path/to/file |
| Directory | `DirectoryPath` | /path/to/dir |
| **Identifiers** |
| UUID | `Uuid` | 550e8400-... |
| Slug | `Slug` | my-blog-post |
| Username | `Username` | john_doe |
| Password | `Secret` | ******** |
| **Date/Time** |
| Date | `Date` | 2024-03-15 |
| Time | `Time` | 14:30:00 |
| DateTime | `DateTime` | 2024-03-15T14:30:00Z |
| **DevOps** |
| Semantic version | `SemVer` | 1.2.3 |
| Git ref | `GitRef` | main |
| Docker image | `DockerImage` | nginx:latest |

### NumberSubtype Quick Lookup

| Use Case | Subtype | Unit | Example |
|----------|---------|------|---------|
| **Generic** |
| Integer | `Integer` | - | 42 |
| Float | `Float` | - | 3.14 |
| Percentage | `Percentage` | % | 75.5% |
| **Financial** |
| Money | `Currency` | USD/EUR | $1,234.56 |
| Price | `Price` | USD | $99.99 |
| Tax | `Tax` | % | 8.5% |
| **Physical** |
| Temperature | `Temperature` | °C/°F/K | 25°C |
| Distance | `Distance` | m/km/mi | 100m |
| Weight | `Weight` | kg/lb | 70kg |
| Speed | `Speed` | km/h/mph | 60km/h |
| **Time** |
| Unix time | `UnixTime` | s | 1710000000 |
| Duration | `DurationSeconds` | s | 3600 |
| Year | `Year` | - | 2024 |
| **Data** |
| File size | `FileSize` | bytes | 1.5MB |
| Bandwidth | `Bandwidth` | Mbps | 100Mbps |
| **Geographic** |
| Latitude | `Latitude` | ° | 37.7749 |
| Longitude | `Longitude` | ° | -122.4194 |
| **Network** |
| Port | `Port` | - | 8080 |
| HTTP status | `HttpStatusCode` | - | 200 |
| **Statistics** |
| Probability | `Probability` | - | 0.75 |
| Count | `Count` | - | 42 |
| Rating | `Rating` | - | 4.5 |

### VectorSubtype Quick Lookup

| Use Case | Subtype | Components | Example |
|----------|---------|------------|---------|
| **Geometric** |
| 2D vector | `Vector2` | 2 | [x, y] |
| 3D vector | `Vector3` | 3 | [x, y, z] |
| 4D vector | `Vector4` | 4 | [x, y, z, w] |
| **Positions** |
| 2D position | `Position2D` | 2 | [100, 200] |
| 3D position | `Position3D` | 3 | [1.0, 2.0, 3.0] |
| **Rotations** |
| Euler angles | `EulerAngles` | 3 | [pitch, yaw, roll] |
| Quaternion | `Quaternion` | 4 | [x, y, z, w] |
| **Colors** |
| RGB | `ColorRgb` | 3 | [1.0, 0.0, 0.0] |
| RGBA | `ColorRgba` | 4 | [1.0, 0.0, 0.0, 1.0] |
| HSV | `ColorHsv` | 3 | [0, 1.0, 1.0] |
| **Texture** |
| UV coords | `TexCoord2D` | 2 | [0.5, 0.5] |
| **Matrices** |
| 2×2 matrix | `Matrix2x2` | 4 | [...] |
| 3×3 matrix | `Matrix3x3` | 9 | [...] |
| 4×4 matrix | `Matrix4x4` | 16 | [...] |

---

## Units Reference

### Temperature

```rust
TemperatureUnit::Celsius      // °C
TemperatureUnit::Fahrenheit   // °F
TemperatureUnit::Kelvin       // K

// Conversion
let f = TemperatureUnit::Celsius.convert_to(100.0, TemperatureUnit::Fahrenheit);
// 100°C → 212°F
```

### Distance

```rust
DistanceUnit::Meters          // m
DistanceUnit::Kilometers      // km
DistanceUnit::Miles           // mi
DistanceUnit::Feet            // ft

// Conversion
let mi = DistanceUnit::Kilometers.convert_to(1.0, DistanceUnit::Miles);
// 1km → 0.621mi
```

### Data Size

```rust
DataSizeUnit::Bytes           // B
DataSizeUnit::Kilobytes       // KB (1024 bytes)
DataSizeUnit::Megabytes       // MB
DataSizeUnit::Gigabytes       // GB

// Auto-format
DataSizeUnit::format_bytes(1_048_576.0);  // "1.00 MB"
```

### All Unit Categories

```rust
Unit::Temperature(TemperatureUnit)
Unit::Distance(DistanceUnit)
Unit::Weight(WeightUnit)
Unit::Volume(VolumeUnit)
Unit::Area(AreaUnit)
Unit::Speed(SpeedUnit)
Unit::Duration(DurationUnit)
Unit::DataSize(DataSizeUnit)
Unit::Frequency(FrequencyUnit)
Unit::Angle(AngleUnit)
Unit::Pressure(PressureUnit)
Unit::Energy(EnergyUnit)
Unit::Power(PowerUnit)
Unit::Force(ForceUnit)
Unit::Acceleration(AccelerationUnit)
Unit::Currency(CurrencyCode)
```

---

## Builder Patterns

### TextParameter Examples

```rust
// Email
TextParameter::email("email")
    .label("Email Address")
    .required()
    .placeholder("user@example.com")
    .build()

// URL
TextParameter::url("website")
    .label("Website URL")
    .placeholder("https://example.com")
    .build()

// Password
TextParameter::password("password")
    .label("Password")
    .required()
    .min_length(8)
    .build()

// Code with language
TextParameter::builder("script")
    .subtype(TextSubtype::code_with_language(CodeLanguage::Python))
    .label("Python Script")
    .build()

// File path
TextParameter::builder("config_file")
    .subtype(TextSubtype::FilePath)
    .label("Configuration File")
    .build()
```

### NumberParameter Examples

```rust
// Integer
NumberParameter::integer("count")
    .label("Item Count")
    .min(0)
    .max(100)
    .build()

// Percentage
NumberParameter::percentage("opacity")
    .label("Opacity")
    .min(0.0)
    .max(100.0)
    .default_value(100.0)
    .build()

// Temperature
NumberParameter::builder("temperature")
    .subtype(NumberSubtype::Temperature)
    .unit(Unit::Temperature(TemperatureUnit::Celsius))
    .label("Temperature")
    .build()

// Currency
NumberParameter::builder("price")
    .subtype(NumberSubtype::Currency)
    .unit(Unit::Currency(CurrencyCode::USD))
    .label("Price")
    .min(0.0)
    .build()

// Port number
NumberParameter::builder("port")
    .subtype(NumberSubtype::Port)
    .label("Port")
    .min(0)
    .max(65535)
    .default_value(8080)
    .build()
```

### VectorParameter Examples

```rust
// 3D position
VectorParameter::vector3("position")
    .label("Position")
    .default_vec3([0.0, 0.0, 0.0])
    .build()

// RGB color
VectorParameter::color_rgb("color")
    .label("Color")
    .default_vec3([1.0, 1.0, 1.0])
    .build()

// RGBA color with alpha
VectorParameter::color_rgba("tint")
    .label("Tint Color")
    .default_vec4([1.0, 1.0, 1.0, 1.0])
    .build()

// Quaternion rotation
VectorParameter::builder("rotation")
    .subtype(VectorSubtype::Quaternion)
    .label("Rotation")
    .default_vec4([0.0, 0.0, 0.0, 1.0])
    .build()
```

### BoolParameter Examples

```rust
// Simple checkbox
BoolParameter::builder("enabled")
    .label("Enabled")
    .default_value(true)
    .build()

// Toggle switch
BoolParameter::builder("show_advanced")
    .label("Show Advanced Options")
    .default_value(false)
    .build()
```

### ChoiceParameter Examples

```rust
// Single selection dropdown
ChoiceParameter::single("theme")
    .label("Theme")
    .option("light", "Light Theme")
    .option("dark", "Dark Theme")
    .option("auto", "Auto")
    .default_value("auto")
    .build()

// Multiple selection
ChoiceParameter::multi("tags")
    .label("Tags")
    .option("rust", "Rust")
    .option("web", "Web Development")
    .option("backend", "Backend")
    .build()
```

---

## Validation

### Built-in Validators

```rust
// Required
TextParameter::email("email")
    .required()

// Length constraints
TextParameter::builder("username")
    .min_length(3)
    .max_length(20)

// Pattern matching
TextParameter::builder("slug")
    .pattern(r"^[a-z0-9-]+$")

// Allowed values
TextParameter::builder("status")
    .allowed_values(["draft", "published", "archived"])

// Numeric range
NumberParameter::integer("age")
    .min(0)
    .max(150)

// Custom validator
TextParameter::email("email")
    .with_validator(MyCustomValidator)
```

### Custom Validator

```rust
pub struct MyCustomValidator;

impl Validator for MyCustomValidator {
    fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Value::Text(s) = value {
            if s.contains("bad_word") {
                return Err(ValidationError::custom("Contains prohibited content"));
            }
        }
        Ok(())
    }
}
```

---

## Events

### Event Types

```rust
ParameterEvent::ValueChanging { key, old_value, new_value }
ParameterEvent::ValueChanged { key, old_value, new_value }
ParameterEvent::Validated { key, is_valid, errors }
ParameterEvent::Dirtied { key }
ParameterEvent::Touched { key }
ParameterEvent::VisibilityChanged { key, visible }
ParameterEvent::BatchBegin { description }
ParameterEvent::BatchEnd { description }
```

### Subscribe to Events

```rust
// Callback observer
context.subscribe(LoggerObserver::new());

// Async receiver
let mut rx = context.receiver();
tokio::spawn(async move {
    while let Ok(event) = rx.recv().await {
        match event {
            ParameterEvent::ValueChanged { key, new_value, .. } => {
                println!("{} = {:?}", key, new_value);
            }
            _ => {}
        }
    }
});
```

### Custom Observer

```rust
pub struct MyObserver;

impl Observer for MyObserver {
    fn on_event(&mut self, event: &ParameterEvent) {
        match event {
            ParameterEvent::ValueChanged { key, .. } => {
                println!("Value changed: {}", key);
            }
            _ => {}
        }
    }
    
    fn name(&self) -> &str {
        "MyObserver"
    }
}
```

---

## Display Conditions

### Basic Conditions

```rust
// Show when field equals value
.show_when_equals("auth_type", Value::text("api_key"))

// Show when field is true
.show_when_true("advanced_mode")

// Show when field is set (not null)
.show_when(DisplayRule::when("field", DisplayCondition::IsSet))

// Hide when field equals value
.hide_when_equals("status", Value::text("disabled"))
```

### Complex Conditions

```rust
// AND: All conditions must be true
.show_when(DisplayRuleSet::all([
    DisplayRule::when("enabled", DisplayCondition::IsTrue),
    DisplayRule::when("level", DisplayCondition::GreaterThan(10.0)),
]))

// OR: Any condition must be true
.show_when(DisplayRuleSet::any([
    DisplayRule::when("role", DisplayCondition::Equals(Value::text("admin"))),
    DisplayRule::when("superuser", DisplayCondition::IsTrue),
]))

// NOT: Condition must be false
.show_when(DisplayRuleSet::not(
    DisplayRule::when("disabled", DisplayCondition::IsTrue)
))
```

### Validation-Based Display

```rust
// Show when field is valid
.show_when_valid("password")

// Show error hint when field is invalid
.show_when_invalid("email")

// Show confirm password only when password is valid
TextParameter::password("confirm_password")
    .with_display(
        ParameterDisplay::new()
            .show_when_valid("password")
    )
```

---

## Common Recipes

### Recipe 1: Form with Validation

```rust
let schema = Schema::new()
    .with_parameter(
        TextParameter::builder("name")
            .label("Full Name")
            .required()
            .min_length(2)
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
            .min_length(8)
            .build()
    )
    .build();

let mut context = Context::new(schema);
context.set_value("name", "John Doe".into())?;
context.set_value("email", "john@example.com".into())?;
context.set_value("password", "secret123".into())?;

if context.validate_all() {
    println!("Form is valid!");
}
```

---

### Recipe 2: Conditional Fields

```rust
// API Key field shown only when auth_type = "api_key"
let schema = Schema::new()
    .with_parameter(
        ChoiceParameter::single("auth_type")
            .label("Authentication Type")
            .option("oauth", "OAuth 2.0")
            .option("api_key", "API Key")
            .option("basic", "Basic Auth")
            .build()
    )
    .with_parameter(
        TextParameter::builder("api_key")
            .subtype(TextSubtype::Secret)
            .label("API Key")
            .with_display(
                ParameterDisplay::new()
                    .show_when_equals("auth_type", Value::text("api_key"))
            )
            .build()
    )
    .with_parameter(
        TextParameter::builder("oauth_token")
            .label("OAuth Token")
            .with_display(
                ParameterDisplay::new()
                    .show_when_equals("auth_type", Value::text("oauth"))
            )
            .build()
    )
    .build();
```

---

### Recipe 3: Undo/Redo

```rust
let mut context = Context::new(schema);

// Make changes
context.set_value("name", "Alice".into())?;
context.set_value("name", "Bob".into())?;

// Undo
context.undo()?;  // Back to "Alice"
context.undo()?;  // Back to empty

// Redo
context.redo()?;  // Forward to "Alice"
context.redo()?;  // Forward to "Bob"

// Transactions
context.begin_transaction("Update user");
context.set_value("first_name", "Alice".into())?;
context.set_value("last_name", "Smith".into())?;
context.end_transaction()?;

// One undo reverts both changes
context.undo()?;
```

---

### Recipe 4: 3D Transform

```rust
let schema = Schema::new()
    .with_parameter(
        VectorParameter::vector3("position")
            .label("Position")
            .default_vec3([0.0, 0.0, 0.0])
            .build()
    )
    .with_parameter(
        VectorParameter::builder("rotation")
            .subtype(VectorSubtype::EulerAngles)
            .label("Rotation")
            .default_vec3([0.0, 0.0, 0.0])
            .build()
    )
    .with_parameter(
        VectorParameter::vector3("scale")
            .subtype(VectorSubtype::Scale3D)
            .label("Scale")
            .default_vec3([1.0, 1.0, 1.0])
            .build()
    )
    .build();

let mut context = Context::new(schema);

// Set transform
context.set_vec3("position", [10.0, 20.0, 30.0])?;
context.set_vec3("rotation", [0.0, 90.0, 0.0])?;
context.set_vec3("scale", [2.0, 2.0, 2.0])?;

// Get transform
let pos: [f64; 3] = context.get_vec3("position")?;
let rot: [f64; 3] = context.get_vec3("rotation")?;
let scale: [f64; 3] = context.get_vec3("scale")?;
```

---

### Recipe 5: Reactive UI Updates

```rust
let mut context = Context::new(schema);

// Subscribe to changes
let mut rx = context.receiver();

tokio::spawn(async move {
    while let Ok(event) = rx.recv().await {
        match event {
            ParameterEvent::ValueChanged { key, new_value, .. } => {
                update_ui_field(&key, &new_value);
            }
            ParameterEvent::VisibilityChanged { key, visible } => {
                if visible {
                    show_ui_field(&key);
                } else {
                    hide_ui_field(&key);
                }
            }
            ParameterEvent::Validated { key, is_valid, .. } => {
                if is_valid {
                    clear_error(&key);
                } else {
                    show_error(&key);
                }
            }
            _ => {}
        }
    }
});

// Changes automatically propagate to UI
context.set_value("email", "test@example.com".into())?;
```

---

## Performance Tips

### 1. Use Arc Sharing

```rust
// Schema is Arc-shared automatically
let schema = Schema::new()...;
let context1 = Context::new(schema.clone());  // ✅ Cheap clone (Arc)
let context2 = Context::new(schema.clone());  // ✅ Cheap clone (Arc)
```

### 2. Batch Operations

```rust
// Batch updates to reduce events
context.begin_transaction("Import data");
for item in data {
    context.set_value(&item.key, item.value)?;
}
context.end_transaction()?;
// Only one BatchEnd event, not 100 ValueChanged events
```

### 3. Use Type-Safe Getters

```rust
// ✅ Fast: Direct extraction
let value: [f64; 3] = context.get_vec3("position")?;

// ❌ Slow: Generic then downcast
let value = context.get_value("position")?;
let vec3 = value.as_vec3()?;
```

---

## Quick Debugging

### Enable Logging

```rust
// Subscribe logger observer
context.subscribe(LoggerObserver::new());

// Now all events are logged
context.set_value("email", "test@example.com".into())?;
// [LOG] ValueChanged { key: "email", ... }
```

### Check State

```rust
// Check dirty flag
if context.is_dirty("email") {
    println!("Email was modified");
}

// Check validity
if !context.is_valid("email") {
    let errors = context.get_errors("email");
    println!("Validation errors: {:?}", errors);
}

// Check visibility
if !context.is_visible("api_key") {
    println!("API key field is hidden");
}
```

---

## Summary

**This quick reference covers:**
- ✅ All core types and subtypes
- ✅ Unit system with conversions
- ✅ Builder patterns for all parameter types
- ✅ Validation examples
- ✅ Event subscription patterns
- ✅ Display conditions (simple and complex)
- ✅ Common recipes (forms, 3D, reactive UI)
- ✅ Performance tips
- ✅ Debugging helpers

**For full details, see FINAL_ARCHITECTURE.md** 📚
