# Nebula Parameters - Subtype Selection Guide

**How to choose the right subtype for your use case**

---

## Table of Contents

1. [Decision Framework](#decision-framework)
2. [Text Subtypes](#text-subtypes)
3. [Number Subtypes](#number-subtypes)
4. [Vector Subtypes](#vector-subtypes)
5. [Common Scenarios](#common-scenarios)
6. [Anti-Patterns](#anti-patterns)

---

## Decision Framework

### Step 1: Identify the Base Type

```
┌─ Is it text/string? ────────→ TextParameter
│
├─ Is it numeric? ─────────────→ NumberParameter
│
├─ Is it true/false? ──────────→ BoolParameter
│
├─ Is it one of a fixed set? ──→ ChoiceParameter
│
└─ Is it fixed-size numeric? ──→ VectorParameter
```

### Step 2: Ask Three Questions

**Question 1: "What IS this value fundamentally?"**
- Email address → `TextSubtype::Email`
- Temperature → `NumberSubtype::Temperature`
- RGB color → `VectorSubtype::ColorRgb`

**Question 2: "What validation does it need?"**
- Email format → `TextSubtype::Email` (auto-validates)
- 0-100 range → `NumberSubtype::Percentage`
- Normalized vector → `VectorSubtype::Normal`

**Question 3: "What unit does it have?"**
- Temperature in Celsius → `Unit::Temperature(TemperatureUnit::Celsius)`
- Distance in meters → `Unit::Distance(DistanceUnit::Meters)`
- File size → `Unit::DataSize(DataSizeUnit::Bytes)`

---

## Text Subtypes

### Generic Text

```rust
// Use Generic for:
TextSubtype::Generic        // Any text without specific format
TextSubtype::SingleLine     // Names, titles (no newlines)
TextSubtype::MultiLine      // Descriptions, comments
TextSubtype::RichText       // Formatted content (HTML/Markdown)
```

**When to use:**
- ✅ No specific format required
- ✅ Free-form text input
- ✅ User-generated content

**Examples:**
```rust
// Name (single line)
TextParameter::builder("name")
    .subtype(TextSubtype::SingleLine)

// Description (multi-line)
TextParameter::builder("description")
    .subtype(TextSubtype::MultiLine)

// Blog post (rich text)
TextParameter::builder("content")
    .subtype(TextSubtype::RichText)
```

---

### Code and Structured Data

```rust
// Use these for:
TextSubtype::Code                           // Generic code
TextSubtype::CodeWithLanguage(Rust)        // Language-specific
TextSubtype::Json                          // JSON data
TextSubtype::Xml                           // XML documents
TextSubtype::Yaml                          // YAML config
TextSubtype::SqlQuery                      // SQL queries
```

**Decision Tree:**

```
Is it code or data?
├─ Generic code? ───────────→ Code
├─ Specific language? ──────→ CodeWithLanguage(lang)
├─ JSON/API data? ──────────→ Json
├─ Config file? ────────────→ Yaml / Toml
└─ Database query? ─────────→ SqlQuery
```

**Examples:**
```rust
// Python script
TextParameter::builder("script")
    .subtype(TextSubtype::CodeWithLanguage(CodeLanguage::Python))

// API request body
TextParameter::builder("request")
    .subtype(TextSubtype::Json)

// Kubernetes manifest
TextParameter::builder("manifest")
    .subtype(TextSubtype::Yaml)
```

---

### Web and Network

```rust
// Use these for:
TextSubtype::Email          // Email addresses
TextSubtype::Url            // Any URL
TextSubtype::UrlAbsolute    // Must have scheme (https://)
TextSubtype::Hostname       // Domain/hostname
TextSubtype::IpAddress      // IP (v4 or v6)
```

**Decision Tree:**

```
Is it a web/network identifier?
├─ Email address? ──────────→ Email
├─ Full URL? ───────────────→ Url or UrlAbsolute
├─ Just domain? ────────────→ Hostname
└─ IP address? ─────────────→ IpAddress / IpV4Address
```

**Examples:**
```rust
// User email
TextParameter::email("email")
    // Automatic email validation!

// Website URL
TextParameter::url("website")
    // Validates URL format

// Server hostname
TextParameter::builder("server")
    .subtype(TextSubtype::Hostname)

// IP address
TextParameter::builder("ip")
    .subtype(TextSubtype::IpV4Address)
```

---

### Files and Paths

```rust
// Use these for:
TextSubtype::FilePath           // Any file path
TextSubtype::FilePathAbsolute   // Absolute path
TextSubtype::FilePathRelative   // Relative path
TextSubtype::DirectoryPath      // Directory only
```

**Decision Tree:**

```
Is it a file system path?
├─ File path? ──────────────→ FilePath
├─ Must be absolute? ───────→ FilePathAbsolute
├─ Must be relative? ───────→ FilePathRelative
└─ Directory only? ─────────→ DirectoryPath
```

**Examples:**
```rust
// Config file
TextParameter::builder("config")
    .subtype(TextSubtype::FilePath)

// Output directory
TextParameter::builder("output_dir")
    .subtype(TextSubtype::DirectoryPath)

// Project root (absolute)
TextParameter::builder("project_root")
    .subtype(TextSubtype::FilePathAbsolute)
```

---

### Identifiers

```rust
// Use these for:
TextSubtype::Uuid           // Unique IDs
TextSubtype::Slug           // URL-friendly names
TextSubtype::Username       // User accounts
TextSubtype::Secret         // Passwords, keys
```

**Decision Tree:**

```
Is it an identifier?
├─ Random unique ID? ───────→ Uuid
├─ URL-safe name? ──────────→ Slug
├─ Username? ───────────────→ Username
└─ Secret/password? ────────→ Secret
```

**Examples:**
```rust
// Record ID
TextParameter::builder("id")
    .subtype(TextSubtype::Uuid)

// Blog post slug
TextParameter::builder("slug")
    .subtype(TextSubtype::Slug)
    // Validates lowercase, numbers, hyphens

// Password
TextParameter::password("password")
    // Uses Secret subtype automatically
```

---

### Date and Time

```rust
// Use these for:
TextSubtype::Date           // Date only (YYYY-MM-DD)
TextSubtype::Time           // Time only (HH:MM:SS)
TextSubtype::DateTime       // Full timestamp
TextSubtype::Duration       // Time span
TextSubtype::Timezone       // IANA timezone
```

**Decision Tree:**

```
Is it temporal data?
├─ Date only? ──────────────→ Date
├─ Time only? ──────────────→ Time
├─ Date + time? ────────────→ DateTime
├─ Duration/span? ──────────→ Duration
└─ Timezone? ───────────────→ Timezone
```

**Examples:**
```rust
// Birth date
TextParameter::builder("birth_date")
    .subtype(TextSubtype::Date)

// Appointment time
TextParameter::builder("appointment")
    .subtype(TextSubtype::DateTime)

// Session timeout
TextParameter::builder("timeout")
    .subtype(TextSubtype::Duration)
```

---

## Number Subtypes

### Generic Numbers

```rust
// Use these for:
NumberSubtype::Integer      // Whole numbers
NumberSubtype::Float        // Decimal numbers
NumberSubtype::Percentage   // 0-100 (or 0.0-1.0)
```

**Decision Tree:**

```
Is it a number?
├─ Whole numbers only? ─────→ Integer
├─ Decimals allowed? ───────→ Float
└─ Percentage (0-100)? ─────→ Percentage
```

**Examples:**
```rust
// Count
NumberParameter::integer("count")

// Ratio
NumberParameter::float("ratio")

// Opacity
NumberParameter::percentage("opacity")
    .min(0.0)
    .max(100.0)
```

---

### Physical Measurements

```rust
// Use these for:
NumberSubtype::Temperature  // Heat
NumberSubtype::Distance     // Length
NumberSubtype::Weight       // Mass
NumberSubtype::Speed        // Velocity
NumberSubtype::Pressure     // Force per area
```

**Decision Tree:**

```
Is it a physical quantity?
├─ Temperature? ────────────→ Temperature + TemperatureUnit
├─ Length/distance? ────────→ Distance + DistanceUnit
├─ Mass/weight? ────────────→ Weight + WeightUnit
├─ Speed? ──────────────────→ Speed + SpeedUnit
└─ Other physical? ─────────→ (see full list)
```

**Examples:**
```rust
// Room temperature
NumberParameter::builder("temperature")
    .subtype(NumberSubtype::Temperature)
    .unit(Unit::Temperature(TemperatureUnit::Celsius))

// Travel distance
NumberParameter::builder("distance")
    .subtype(NumberSubtype::Distance)
    .unit(Unit::Distance(DistanceUnit::Kilometers))

// Package weight
NumberParameter::builder("weight")
    .subtype(NumberSubtype::Weight)
    .unit(Unit::Weight(WeightUnit::Kilograms))
```

---

### Financial

```rust
// Use these for:
NumberSubtype::Currency     // Money amounts
NumberSubtype::Price        // Product prices
NumberSubtype::Tax          // Tax amounts/rates
NumberSubtype::Discount     // Discounts
```

**Decision Tree:**

```
Is it money-related?
├─ Money amount? ───────────→ Currency + CurrencyCode
├─ Product price? ──────────→ Price + CurrencyCode
├─ Tax? ────────────────────→ Tax (as % or amount)
└─ Discount? ───────────────→ Discount (as % or amount)
```

**Examples:**
```rust
// Product price
NumberParameter::builder("price")
    .subtype(NumberSubtype::Price)
    .unit(Unit::Currency(CurrencyCode::USD))
    .min(0.0)

// Sales tax rate
NumberParameter::builder("tax_rate")
    .subtype(NumberSubtype::Tax)
    .min(0.0)
    .max(100.0)  // Percentage

// Discount amount
NumberParameter::builder("discount")
    .subtype(NumberSubtype::Discount)
```

---

### Data and Network

```rust
// Use these for:
NumberSubtype::FileSize     // File sizes
NumberSubtype::Bandwidth    // Data transfer rate
NumberSubtype::Port         // Network ports
NumberSubtype::Latency      // Network delay
```

**Decision Tree:**

```
Is it computer/network related?
├─ File size? ──────────────→ FileSize + DataSizeUnit
├─ Network speed? ──────────→ Bandwidth
├─ Port number? ────────────→ Port (0-65535)
└─ Network delay? ──────────→ Latency (milliseconds)
```

**Examples:**
```rust
// Upload limit
NumberParameter::builder("max_upload")
    .subtype(NumberSubtype::FileSize)
    .unit(Unit::DataSize(DataSizeUnit::Megabytes))

// Server port
NumberParameter::builder("port")
    .subtype(NumberSubtype::Port)
    .min(1024)
    .max(65535)

// Connection timeout
NumberParameter::builder("timeout")
    .subtype(NumberSubtype::Latency)
    .min(100)
    .max(30000)
```

---

### Geographic

```rust
// Use these for:
NumberSubtype::Latitude     // -90 to +90
NumberSubtype::Longitude    // -180 to +180
NumberSubtype::Altitude     // Height above sea level
```

**Always use these for GPS coordinates!**

**Examples:**
```rust
// GPS location
NumberParameter::builder("latitude")
    .subtype(NumberSubtype::Latitude)
    .min(-90.0)
    .max(90.0)

NumberParameter::builder("longitude")
    .subtype(NumberSubtype::Longitude)
    .min(-180.0)
    .max(180.0)
```

---

## Vector Subtypes

### Geometric Vectors

```rust
// Use these for:
VectorSubtype::Vector2      // 2D vectors
VectorSubtype::Vector3      // 3D vectors
VectorSubtype::Vector4      // 4D vectors
```

**Decision Tree:**

```
Is it a vector?
├─ 2D (x, y)? ──────────────→ Vector2
├─ 3D (x, y, z)? ───────────→ Vector3
└─ 4D (x, y, z, w)? ────────→ Vector4
```

**Examples:**
```rust
// 2D point
VectorParameter::vector2("point")
    .default_vec2([0.0, 0.0])

// 3D position
VectorParameter::vector3("position")
    .default_vec3([0.0, 0.0, 0.0])
```

---

### Positions vs Directions vs Normals

```rust
// Semantic distinction:
VectorSubtype::Position3D   // Point in space
VectorSubtype::Direction3D  // Unit direction vector
VectorSubtype::Normal       // Surface normal (normalized)
```

**When to use which:**

| Subtype | Use Case | Normalized? | Example |
|---------|----------|-------------|---------|
| `Position3D` | Object location | No | [10, 20, 5] |
| `Direction3D` | Movement direction | Yes | [0.707, 0.707, 0] |
| `Normal` | Surface normal | Yes | [0, 1, 0] |

**Examples:**
```rust
// Object position (any values)
VectorParameter::builder("position")
    .subtype(VectorSubtype::Position3D)

// Movement direction (normalized)
VectorParameter::builder("direction")
    .subtype(VectorSubtype::Direction3D)
    // Will validate that length ≈ 1

// Surface normal (normalized)
VectorParameter::builder("normal")
    .subtype(VectorSubtype::Normal)
    // Must be unit vector
```

---

### Colors

```rust
// Use these for:
VectorSubtype::ColorRgb     // RGB (3 components)
VectorSubtype::ColorRgba    // RGBA with alpha (4)
VectorSubtype::ColorHsv     // HSV color space
VectorSubtype::ColorHsl     // HSL color space
```

**Decision Tree:**

```
Is it a color?
├─ RGB without alpha? ──────→ ColorRgb
├─ RGB with alpha? ─────────→ ColorRgba
├─ HSV color space? ────────→ ColorHsv
└─ HSL color space? ────────→ ColorHsl
```

**Examples:**
```rust
// Tint color with transparency
VectorParameter::color_rgba("tint")
    .default_vec4([1.0, 1.0, 1.0, 1.0])

// Background color (no alpha)
VectorParameter::color_rgb("background")
    .default_vec3([0.2, 0.2, 0.2])
```

---

### Rotations

```rust
// Use these for:
VectorSubtype::EulerAngles  // Pitch, yaw, roll (degrees)
VectorSubtype::Quaternion   // x, y, z, w (normalized)
VectorSubtype::AxisAngle    // Axis + angle
```

**Decision Tree:**

```
Is it a rotation?
├─ Euler angles (degrees)? ─→ EulerAngles
├─ Quaternion? ─────────────→ Quaternion (no gimbal lock)
└─ Axis + angle? ───────────→ AxisAngle
```

**When to use which:**

| Subtype | Pros | Cons | Use Case |
|---------|------|------|----------|
| `EulerAngles` | Intuitive | Gimbal lock | UI, simple rotations |
| `Quaternion` | No gimbal lock | Less intuitive | Animation, interpolation |
| `AxisAngle` | Clear meaning | Conversion needed | Rotation specification |

**Examples:**
```rust
// Camera rotation (Euler)
VectorParameter::builder("rotation")
    .subtype(VectorSubtype::EulerAngles)
    .default_vec3([0.0, 0.0, 0.0])  // pitch, yaw, roll

// Bone rotation (Quaternion)
VectorParameter::builder("bone_rotation")
    .subtype(VectorSubtype::Quaternion)
    .default_vec4([0.0, 0.0, 0.0, 1.0])  // identity
```

---

### Matrices

```rust
// Use these for:
VectorSubtype::Matrix2x2    // 2D transforms
VectorSubtype::Matrix3x3    // 2D homogeneous
VectorSubtype::Matrix4x4    // 3D transforms
```

**Examples:**
```rust
// 3D transformation matrix
VectorParameter::builder("transform")
    .subtype(VectorSubtype::Matrix4x4)
    // 16 components
```

---

## Common Scenarios

### Scenario 1: User Profile Form

```rust
Schema::new()
    // Name (single line)
    .with_parameter(
        TextParameter::builder("name")
            .subtype(TextSubtype::SingleLine)
            .required()
    )
    
    // Email (validated)
    .with_parameter(
        TextParameter::email("email")
            .required()
    )
    
    // Bio (multi-line)
    .with_parameter(
        TextParameter::builder("bio")
            .subtype(TextSubtype::MultiLine)
    )
    
    // Avatar URL
    .with_parameter(
        TextParameter::url("avatar")
    )
    
    // Age (integer)
    .with_parameter(
        NumberParameter::integer("age")
            .subtype(NumberSubtype::Age)
            .min(13)
    )
```

---

### Scenario 2: 3D Game Object

```rust
Schema::new()
    // Position in world
    .with_parameter(
        VectorParameter::builder("position")
            .subtype(VectorSubtype::Position3D)
    )
    
    // Rotation (Euler angles for UI)
    .with_parameter(
        VectorParameter::builder("rotation")
            .subtype(VectorSubtype::EulerAngles)
    )
    
    // Scale (uniform or non-uniform)
    .with_parameter(
        VectorParameter::builder("scale")
            .subtype(VectorSubtype::Scale3D)
            .default_vec3([1.0, 1.0, 1.0])
    )
    
    // Color with transparency
    .with_parameter(
        VectorParameter::color_rgba("color")
            .default_vec4([1.0, 1.0, 1.0, 1.0])
    )
```

---

### Scenario 3: API Configuration

```rust
Schema::new()
    // Endpoint URL
    .with_parameter(
        TextParameter::url("endpoint")
            .subtype(TextSubtype::UrlAbsolute)
            .required()
    )
    
    // API Key (secret)
    .with_parameter(
        TextParameter::builder("api_key")
            .subtype(TextSubtype::Secret)
            .required()
    )
    
    // Timeout (milliseconds)
    .with_parameter(
        NumberParameter::integer("timeout")
            .subtype(NumberSubtype::DurationMillis)
            .default_value(30000)
    )
    
    // Max retries
    .with_parameter(
        NumberParameter::integer("max_retries")
            .subtype(NumberSubtype::Count)
            .min(0)
            .max(10)
    )
```

---

## Anti-Patterns

### ❌ DON'T: Use Generic when specific exists

```rust
// ❌ BAD
TextParameter::builder("email")
    .subtype(TextSubtype::Generic)
    .pattern(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$")

// ✅ GOOD
TextParameter::email("email")
    // Built-in validation!
```

---

### ❌ DON'T: Confuse position with direction

```rust
// ❌ BAD
VectorParameter::vector3("velocity")
    // velocity is a direction, not position!

// ✅ GOOD
VectorParameter::builder("velocity")
    .subtype(VectorSubtype::Direction3D)
```

---

### ❌ DON'T: Forget units for physical quantities

```rust
// ❌ BAD
NumberParameter::float("temperature")
    .subtype(NumberSubtype::Temperature)
    // What unit? Celsius? Fahrenheit?

// ✅ GOOD
NumberParameter::builder("temperature")
    .subtype(NumberSubtype::Temperature)
    .unit(Unit::Temperature(TemperatureUnit::Celsius))
```

---

### ❌ DON'T: Use Secret for non-secrets

```rust
// ❌ BAD
TextParameter::builder("username")
    .subtype(TextSubtype::Secret)
    // Username is not secret!

// ✅ GOOD
TextParameter::builder("username")
    .subtype(TextSubtype::Username)
```

---

## Quick Reference Table

### Text Subtypes

| Use Case | Subtype | Example |
|----------|---------|---------|
| Any text | `Generic` | Notes |
| Name, title | `SingleLine` | "John Doe" |
| Description | `MultiLine` | "Long text..." |
| Email | `Email` | user@example.com |
| URL | `Url` | https://example.com |
| File path | `FilePath` | /path/to/file |
| Password | `Secret` | ******* |
| Code | `Code` or `CodeWithLanguage` | fn main() {...} |
| JSON | `Json` | {"key": "value"} |

### Number Subtypes

| Use Case | Subtype | Unit |
|----------|---------|------|
| Count | `Integer` | - |
| Percentage | `Percentage` | % |
| Money | `Currency` | USD |
| Temperature | `Temperature` | °C |
| Distance | `Distance` | m |
| File size | `FileSize` | bytes |
| Port | `Port` | - |
| Latitude | `Latitude` | ° |

### Vector Subtypes

| Use Case | Subtype | Size |
|----------|---------|------|
| 2D point | `Vector2` | 2 |
| 3D point | `Vector3` | 3 |
| Position | `Position3D` | 3 |
| Direction | `Direction3D` | 3 |
| RGB color | `ColorRgb` | 3 |
| RGBA color | `ColorRgba` | 4 |
| Rotation | `EulerAngles` | 3 |
| Quaternion | `Quaternion` | 4 |

---

## Summary

**Decision Process:**
1. Choose base type (Text/Number/Vector)
2. Identify semantic meaning
3. Select appropriate subtype
4. Add unit if applicable
5. Validate choice against anti-patterns

**Remember:**
- Subtypes = semantic meaning
- Units = measurement system
- Validation = automatic when subtype chosen
- UI hints = derived from subtype

**When in doubt, use the more specific subtype!** 🎯
