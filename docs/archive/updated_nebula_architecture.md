# Nebula Value: Final Architecture with Performance & Maintainability Focus

> **⚠️ Актуальное решение:** Отдельный crate nebula-value не используется. В Nebula используются **serde** и **serde_json::Value** для значений и сериализации. Этот документ сохранён как архив прежней концепции.

## 📋 Executive Summary (архив)

**nebula-value** — ранее рассматривавшаяся типобезопасная система значений. Сейчас заменена на serde/serde_json::Value. It achieves true zero-cost abstractions through direct value mapping while maintaining clean APIs, rich error handling, and seamless interoperability.

### 🎯 Key Features

- **Rich Core Types**: Comprehensive types (`Text`, `Integer`, `Bool`, `Array`, `Map`) with built-in validation and operations
- **True Zero-Cost Custom Types**: Direct value mapping without JSON serialization overhead
- **Type-Specific Errors**: Dedicated error types for each value type (TextError, NumberError, etc.)
- **Unified Architecture**: Both core and custom types use the same underlying `TypedValue<T>` system
- **Ergonomic APIs**: Automatic Into conversions with explicit method access (no Deref anti-pattern)
- **Modular Design**: Clean separation between core library and derive macros
- **Feature-Gated**: Minimal core with optional advanced features

## 🏗️ Updated Project Structure

```
nebula-value/                    # Main crate
├── Cargo.toml                   # Project configuration
├── README.md                    # User documentation
├── CHANGELOG.md                 # Version history
├── ARCHITECTURE.md              # This document
├── LICENSE-MIT
├── LICENSE-APACHE
├── 
├── src/
│   ├── lib.rs                   # Main entry point with clean exports
│   ├── prelude.rs               # Convenient re-exports for development
│   │
│   ├── types/                   # Rich core type system
│   │   ├── mod.rs               # Clean type exports and re-exports
│   │   │
│   │   ├── text/                # Text type with comprehensive functionality
│   │   │   ├── mod.rs           # Public API and type alias
│   │   │   ├── inner.rs         # Core TextInner struct
│   │   │   ├── error.rs         # TextError - text-specific errors
│   │   │   ├── ops.rs           # String operations and methods
│   │   │   ├── convert.rs       # Conversion implementations
│   │   │   ├── traits.rs        # Standard trait implementations
│   │   │   ├── validation.rs    # Text-specific validation
│   │   │   └── encoding.rs      # Base64, hex encoding (feature: encoding)
│   │   │
│   │   ├── number/              # Numeric types (integer, float, decimal)
│   │   │   ├── mod.rs           # Public API with all numeric types
│   │   │   ├── integer.rs       # Integer types and hybrid sizing
│   │   │   ├── float.rs         # Float types (F32, F64, Float enum)
│   │   │   ├── decimal.rs       # High-precision decimal (feature: decimal)
│   │   │   ├── error.rs         # NumberError - all numeric errors
│   │   │   ├── ops.rs           # Arithmetic operations for all numeric types
│   │   │   ├── convert.rs       # Conversions between numeric types
│   │   │   ├── traits.rs        # Numeric trait implementations
│   │   │   └── validation.rs    # Numeric validation and range checking
│   │   │
│   │   ├── bool/                # Boolean type
│   │   │   ├── mod.rs           # Public API
│   │   │   ├── inner.rs         # Core BoolInner struct
│   │   │   ├── error.rs         # BoolError - boolean-specific errors
│   │   │   ├── ops.rs           # Logical operations
│   │   │   ├── convert.rs       # From<bool>, TryFrom<&str>
│   │   │   ├── traits.rs        # Standard traits
│   │   │   └── validation.rs    # Boolean validation
│   │   │
│   │   ├── array/               # Dynamic array type
│   │   │   ├── mod.rs           # Public API
│   │   │   ├── inner.rs         # Core ArrayInner struct
│   │   │   ├── error.rs         # ArrayError - array-specific errors
│   │   │   ├── ops.rs           # Array operations and methods
│   │   │   ├── iter.rs          # Iterator implementations
│   │   │   ├── convert.rs       # Array conversions
│   │   │   ├── traits.rs        # Collection trait implementations
│   │   │   └── validation.rs    # Array validation
│   │   │
│   │   ├── map/                 # Key-value map type
│   │   │   ├── mod.rs           # Public API
│   │   │   ├── inner.rs         # Core MapInner struct
│   │   │   ├── error.rs         # MapError - map-specific errors
│   │   │   ├── ops.rs           # Map operations
│   │   │   ├── entry.rs         # Entry API implementation
│   │   │   ├── convert.rs       # Map conversions
│   │   │   ├── traits.rs        # Map trait implementations
│   │   │   └── validation.rs    # Map validation
│   │   │
│   │   ├── bytes/               # Binary data type
│   │   │   ├── mod.rs           # Public API
│   │   │   ├── inner.rs         # Core BytesInner struct
│   │   │   ├── error.rs         # BytesError - bytes-specific errors
│   │   │   ├── ops.rs           # Byte operations
│   │   │   ├── convert.rs       # From<Vec<u8>>
│   │   │   ├── traits.rs        # Standard traits
│   │   │   ├── validation.rs    # Bytes validation
│   │   │   └── encoding.rs      # Hex, Base64 encoding
│   │   │
│   │   └── specialized/         # Feature-gated specialized types
│   │       └── temporal/        # Date/time types (feature: temporal)
│   │           ├── mod.rs       # Temporal exports
│   │           ├── date.rs      # Date type with DateError
│   │           ├── time.rs      # Time type with TimeError
│   │           ├── datetime.rs  # DateTime type with DateTimeError
│   │           ├── duration.rs  # Duration type with DurationError
│   │           ├── error.rs     # TemporalError - temporal-specific errors
│   │           └── validation.rs # Temporal validation
│   │
│   ├── value/                   # Universal Value enum
│   │   ├── mod.rs              # Main Value enum definition
│   │   ├── convert.rs          # Direct value mapping implementations
│   │   ├── try_from.rs         # TryFrom implementations
│   │   ├── ops.rs              # Operations on Value enum
│   │   ├── traits.rs           # Display, Debug, PartialEq, Hash
│   │   ├── visitor.rs          # Visitor pattern for traversal
│   │   ├── serde.rs            # Serde support (feature: serde)
│   │   └── schema.rs           # Schema for Value enum
│   │
│   ├── value_type/             # Custom type infrastructure
│   │   ├── mod.rs              # ValueType exports
│   │   ├── traits.rs           # ValueType trait definition
│   │   ├── typed_value.rs      # TypedValue<T> wrapper implementation
│   │   ├── error.rs            # TypeError, base error types
│   │   ├── validation.rs       # Validation support for ValueTypes
│   │   └── serde_support.rs    # Serde integration for TypedValue
│   │
│   ├── validation/             # Enhanced validation framework
│   │   ├── mod.rs             # Main validation exports
│   │   ├── core.rs            # Core validation traits and utilities
│   │   ├── error.rs           # ValidationError and context
│   │   ├── combinators.rs     # Validation combinators (And, Or, Not)
│   │   ├── length.rs          # Length-based validators
│   │   ├── numeric.rs         # Numeric validators
│   │   ├── pattern.rs         # Pattern and regex validators
│   │   ├── presets.rs         # Common validation presets
│   │   └── async_support.rs   # Async validation (feature: async)
│   │
│   ├── schema/                # JSON Schema generation (feature: schema)
│   │   ├── mod.rs            # Schema exports
│   │   ├── generator.rs      # Schema generator
│   │   └── extensions.rs     # Custom schema extensions
│   │
│   └── utils/                 # Internal utilities
│       ├── mod.rs            # Utility exports
│       └── helpers.rs        # Helper functions
│
├── examples/                  # Comprehensive examples
│   ├── basic_usage.rs         # Getting started with core types
│   ├── custom_types.rs        # Creating and using custom types
│   ├── validation.rs          # Validation examples
│   ├── performance.rs         # Performance optimization examples
│   ├── node_package_example.rs # Example node package with custom types
│   └── integration/           # Integration examples
│       ├── serde_example.rs   # Serde integration
│       ├── web_framework.rs   # Web framework integration
│       └── cross_service.rs   # Cross-service type conversion
│
├── benches/                   # Performance benchmarks
│   ├── value_creation.rs      # Value creation benchmarks
│   ├── type_conversion.rs     # Direct mapping vs JSON overhead
│   ├── validation.rs          # Validation performance
│   └── memory_usage.rs        # Memory usage benchmarks
│
└── tests/                     # Integration tests
    ├── integration/           # Integration test suite
    │   ├── core_types.rs      # Core type tests
    │   ├── custom_types.rs    # Custom type system tests
    │   ├── error_handling.rs  # Error type tests
    │   └── performance.rs     # Zero-cost assertion tests
    │
    └── property/              # Property-based tests
        ├── core_properties.rs # Core type property tests
        └── validation_properties.rs # Validation property tests

nebula-value-derive/           # 🎯 Separate derive crate
├── Cargo.toml                 # Proc-macro crate configuration
├── README.md                  # Derive macro documentation
├── src/
│   ├── lib.rs                 # Main proc-macro exports
│   ├── value_type.rs          # #[derive(ValueType)] implementation
│   ├── validate.rs            # #[derive(Validate)] implementation
│   ├── accessor.rs            # Automatic accessor generation
│   └── utils.rs               # Macro utilities
│
├── examples/
│   ├── derive_basic.rs        # Basic derive usage
│   └── derive_advanced.rs     # Advanced derive features
│
└── tests/
    ├── value_type_derive.rs   # ValueType derive tests
    └── validate_derive.rs     # Validate derive tests
```

## 🔧 Updated Cargo.toml Configuration

### Main Crate (nebula-value)
```toml
[package]
name = "nebula-value"
version = "0.1.0"
edition = "2021"
rust-version = "1.70"
authors = ["Your Name <your.email@example.com>"]
description = "Type-safe value system with zero-cost custom types and rich error handling"
documentation = "https://docs.rs/nebula-value"
repository = "https://github.com/your-org/nebula-value"
homepage = "https://github.com/your-org/nebula-value"
license = "MIT OR Apache-2.0"
keywords = ["value", "types", "validation", "json", "schema"]
categories = ["data-structures", "encoding", "parser-implementations", "web-programming"]
readme = "README.md"

[dependencies]
# Core dependencies
serde = { version = "1.0", features = ["derive"] }

# Optional derive support
nebula-value-derive = { version = "0.1", path = "../nebula-value-derive", optional = true }

# Optional dependencies for features
schemars = { version = "0.8", optional = true }
regex = { version = "1.10", optional = true }
once_cell = { version = "1.19", optional = true }
base64 = { version = "0.22", optional = true }
hex = { version = "0.4", optional = true }

# Specialized types (feature-gated)
rust_decimal = { version = "1.35", optional = true, features = ["serde"] }
chrono = { version = "0.4", optional = true, features = ["serde"] }

# Async support
async-trait = { version = "0.1", optional = true }
tokio = { version = "1.0", optional = true, features = ["rt"] }

[dev-dependencies]
tokio = { version = "1.0", features = ["full"] }
criterion = { version = "0.5", features = ["html_reports"] }
proptest = "1.4"
insta = "1.34"
doc-comment = "0.3"

[features]
default = ["derive", "validation", "regex", "presets"]

# ============================================================================
# CORE FEATURES
# ============================================================================
std = []

# ============================================================================
# DERIVE SUPPORT
# ============================================================================
derive = ["nebula-value-derive"]  # Enable derive macros

# ============================================================================
# SPECIALIZED TYPES - Advanced types (opt-in)
# ============================================================================
decimal = ["dep:rust_decimal"]  # Adds Decimal to number/ module
temporal = ["dep:chrono"]

# ============================================================================
# VALIDATION FEATURES
# ============================================================================
validation = ["dep:once_cell", "regex", "presets"]
regex = ["dep:regex"]
presets = []
async = ["dep:async-trait", "dep:tokio"]

# ============================================================================
# SERIALIZATION & SCHEMA
# ============================================================================
schema = ["dep:schemars"]
encoding = ["dep:base64", "dep:hex"]

# ============================================================================
# SPECIAL USE CASES
# ============================================================================
minimal = []  # Only core types, no derive or validation
testing = ["proptest"]

[lib]
proc-macro = false  # Main crate is not a proc-macro crate
```

### Derive Crate (nebula-value-derive)
```toml
[package]
name = "nebula-value-derive"
version = "0.1.0"
edition = "2021"
rust-version = "1.70"
authors = ["Your Name <your.email@example.com>"]
description = "Derive macros for nebula-value custom types"
license = "MIT OR Apache-2.0"
keywords = ["value", "derive", "macro", "types"]
categories = ["development-tools::procedural-macro-helpers"]

[lib]
proc-macro = true

[dependencies]
syn = { version = "2.0", features = ["full"] }
quote = "1.0"
proc-macro2 = "1.0"

[dev-dependencies]
nebula-value = { path = "../nebula-value", default-features = false }
```

## 🎯 Core Architecture with Zero-Cost Direct Mapping

### Direct Value Mapping (No JSON Overhead)

```rust
// src/value_type/traits.rs - Zero-cost ValueType trait
pub trait ValueType: Clone + Send + Sync + 'static + Serialize + for<'de> Deserialize<'de> {
    /// Convert to core Value using direct mapping (zero-cost)
    fn into_value(self) -> Value;
    
    /// Convert from core Value using direct matching (zero-cost)
    fn from_value(value: Value) -> Result<Self, TypeError>
    where Self: Sized;
    
    /// Validate the custom type
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
    
    /// Type name for debugging and errors
    fn type_name() -> &'static str 
    where Self: Sized {
        std::any::type_name::<Self>()
    }
}
```

### Zero-Cost TypedValue with Explicit Methods

```rust
// src/value_type/typed_value.rs - Zero-cost wrapper with explicit access
#[repr(transparent)]
#[derive(Debug, Clone, PartialEq)]
pub struct TypedValue<T: ValueType> {
    inner: T,
    _phantom: PhantomData<T>,
}

impl<T: ValueType> TypedValue<T> {
    #[inline(always)]
    pub fn new(inner: T) -> Self {
        Self { inner, _phantom: PhantomData }
    }
    
    /// Immutable access to inner value
    #[inline(always)]
    pub fn inner(&self) -> &T { 
        &self.inner 
    }
    
    /// Mutable access to inner value
    #[inline(always)]
    pub fn inner_mut(&mut self) -> &mut T { 
        &mut self.inner 
    }
    
    /// Move out the inner value
    #[inline(always)]
    pub fn into_inner(self) -> T { 
        self.inner 
    }
    
    /// Convert to core Value using direct mapping
    #[inline(always)]
    pub fn into_value(self) -> Value {
        self.inner.into_value()
    }
    
    /// Create from core Value using direct matching
    #[inline(always)]
    pub fn from_value(value: Value) -> Result<Self, TypeError> {
        Ok(Self::new(T::from_value(value)?))
    }
    
    /// Validate the typed value
    #[inline(always)]
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.inner.validate()
    }
}

// No Deref implementation - explicit methods only
// This prevents confusion and maintains clear API boundaries
```

### Direct Mapping Implementation (Zero JSON Overhead)

```rust
// Generated by derive macro or manual implementation
impl ValueType for Position3DInner {
    fn into_value(self) -> Value {
        // Direct mapping - no JSON serialization!
        use std::collections::HashMap;
        Value::Map(Map::from_inner(MapInner(HashMap::from([
            ("x".to_string(), Value::Float(Float::from_inner(FloatInner::F32(self.x)))),
            ("y".to_string(), Value::Float(Float::from_inner(FloatInner::F32(self.y)))),
            ("z".to_string(), Value::Float(Float::from_inner(FloatInner::F32(self.z)))),
        ]))))
    }
    
    fn from_value(value: Value) -> Result<Self, TypeError> {
        // Direct matching - no JSON deserialization!
        match value {
            Value::Map(map) => {
                let mut inner = map.into_inner().0;
                let x = inner.remove("x")
                    .ok_or(TypeError::MissingField("x"))?
                    .try_into_f32()?;
                let y = inner.remove("y")
                    .ok_or(TypeError::MissingField("y"))?
                    .try_into_f32()?;
                let z = inner.remove("z")
                    .ok_or(TypeError::MissingField("z"))?
                    .try_into_f32()?;
                
                Ok(Position3DInner { x, y, z })
            }
            _ => Err(TypeError::WrongType { 
                expected: "Map", 
                found: value.type_name() 
            })
        }
    }
}
```

## 🎯 Type-Specific Error Handling

### Rich Error Types for Each Core Type

```rust
// src/types/text/error.rs - Text-specific errors
#[derive(Debug, Clone, PartialEq)]
pub enum TextError {
    InvalidEncoding { 
        encoding: String, 
        position: usize 
    },
    TooLong { 
        max_length: usize, 
        actual_length: usize 
    },
    TooShort { 
        min_length: usize, 
        actual_length: usize 
    },
    InvalidPattern { 
        pattern: String, 
        input: String 
    },
    EmptyValue,
    InvalidFormat { 
        expected_format: String 
    },
}

// src/types/number/error.rs - All numeric errors in one place
#[derive(Debug, Clone, PartialEq)]
pub enum NumberError {
    // Integer errors
    Overflow { 
        value: String, 
        max: i64 
    },
    Underflow { 
        value: String, 
        min: i64 
    },
    IntegerOutOfRange { 
        min: i64, 
        max: i64, 
        actual: i64 
    },
    
    // Float errors
    NotFinite { 
        value: f64 
    },
    FloatOutOfRange { 
        min: f64, 
        max: f64, 
        actual: f64 
    },
    
    // Decimal errors (feature: decimal)
    #[cfg(feature = "decimal")]
    DecimalOverflow { 
        value: String 
    },
    #[cfg(feature = "decimal")]
    InvalidPrecision { 
        precision: u32, 
        max_precision: u32 
    },
    #[cfg(feature = "decimal")]
    InvalidScale { 
        scale: u32, 
        max_scale: u32 
    },
    
    // Common numeric errors
    InvalidFormat { 
        input: String, 
        expected: String 
    },
    DivisionByZero,
    InvalidRadix { 
        radix: u32 
    },
    PrecisionLoss { 
        original: String, 
        converted: String 
    },
}

// src/types/array/error.rs - Array-specific errors
#[derive(Debug, Clone, PartialEq)]
pub enum ArrayError {
    IndexOutOfBounds { 
        index: usize, 
        len: usize 
    },
    TooManyItems { 
        max: usize, 
        actual: usize 
    },
    TooFewItems { 
        min: usize, 
        actual: usize 
    },
    InvalidItemType { 
        index: usize, 
        expected: String, 
        found: String 
    },
    DuplicateItem { 
        index: usize 
    },
}
```

## 🎯 Derive Macros for Ergonomic APIs

### ValueType Derive with Direct Mapping

```rust
// nebula-value-derive/src/value_type.rs
use nebula_value_derive::ValueType;

#[derive(ValueType)]
pub struct Position3D {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

// Expands to:
struct Position3DInner {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub type Position3D = TypedValue<Position3DInner>;

impl ValueType for Position3DInner {
    fn into_value(self) -> Value {
        // Direct mapping generated by macro
    }
    
    fn from_value(value: Value) -> Result<Self, TypeError> {
        // Direct matching generated by macro
    }
}

impl Position3D {
    // Generated accessor methods (no Deref)
    pub fn x(&self) -> f32 { self.inner().x }
    pub fn y(&self) -> f32 { self.inner().y }
    pub fn z(&self) -> f32 { self.inner().z }
    
    pub fn set_x(&mut self, x: f32) { self.inner_mut().x = x; }
    pub fn set_y(&mut self, y: f32) { self.inner_mut().y = y; }
    pub fn set_z(&mut self, z: f32) { self.inner_mut().z = z; }
    
    // Generated constructor with Into conversions
    pub fn new(x: impl Into<f32>, y: impl Into<f32>, z: impl Into<f32>) -> Self {
        Self::from_inner(Position3DInner {
            x: x.into(),
            y: y.into(),
            z: z.into(),
        })
    }
}
```

### Advanced Derive with Validation

```rust
use nebula_value_derive::{ValueType, Validate};

#[derive(ValueType, Validate)]
pub struct Quaternion {
    #[validate(finite)]
    pub x: f32,
    #[validate(finite)]
    pub y: f32,
    #[validate(finite)]
    pub z: f32,
    #[validate(finite)]
    pub w: f32,
}

#[validate(custom = "validate_normalized")]
impl Quaternion {
    fn validate_normalized(&self) -> Result<(), ValidationError> {
        let magnitude = (self.x() * self.x() + self.y() * self.y() + 
                        self.z() * self.z() + self.w() * self.w()).sqrt();
        if (magnitude - 1.0).abs() > 0.001 {
            Err(ValidationError::new("not_normalized", "Quaternion must be normalized"))
        } else {
            Ok(())
        }
    }
}
```

### Manual Implementation for Complex Cases

```rust
// For complex types, manual implementation provides full control
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComplexObjectInner {
    pub nested_data: Vec<CustomStruct>,
    pub metadata: HashMap<String, DynamicValue>,
}

pub type ComplexObject = TypedValue<ComplexObjectInner>;

impl ValueType for ComplexObjectInner {
    fn into_value(self) -> Value {
        // Custom mapping logic for complex structures
        let mut map = HashMap::new();
        
        // Handle nested_data with custom logic
        let nested_array = self.nested_data.into_iter()
            .map(|item| item.into_value())
            .collect();
        map.insert("nested_data".to_string(), Value::Array(Array::from(nested_array)));
        
        // Handle metadata with dynamic values
        for (key, dynamic_val) in self.metadata {
            map.insert(key, dynamic_val.into_value());
        }
        
        Value::Map(Map::from(map))
    }
    
    fn from_value(value: Value) -> Result<Self, TypeError> {
        // Custom restoration logic
        match value {
            Value::Map(map) => {
                // Custom deserialization logic here
                // ...
            }
            _ => Err(TypeError::WrongType {
                expected: "Map",
                found: value.type_name(),
            })
        }
    }
}
```

## 🚀 Implementation Roadmap

### Phase 1: Zero-Cost Foundation (Weeks 1-4)
**Goal**: True zero-cost abstractions with direct value mapping

#### Week 1-2: Core Architecture
- [ ] Implement TypedValue<T> with #[repr(transparent)]
- [ ] Create direct mapping ValueType trait (no JSON)
- [ ] Build type-specific error systems
- [ ] Verify zero-cost with benchmark tests

#### Week 3-4: Derive Macro Infrastructure
- [ ] Set up nebula-value-derive crate
- [ ] Implement basic ValueType derive
- [ ] Add accessor generation (no Deref)
- [ ] Create validation derive support

**Deliverables**:
- ✅ True zero-cost TypedValue<T> implementation
- ✅ Direct value mapping without JSON overhead
- ✅ Type-specific error handling
- ✅ Basic derive macro support

### Phase 2: Rich Type System (Weeks 5-8)
**Goal**: Complete core type system with validation

#### Week 5-6: Core Types Implementation
- [ ] Implement all core types with specific errors
- [ ] Add comprehensive validation framework
- [ ] Create type conversion utilities
- [ ] Build performance test suite

#### Week 7-8: Advanced Features
- [ ] Advanced derive macro features
- [ ] Validation combinators
- [ ] Schema generation support
- [ ] Integration testing

**Deliverables**:
- ✅ Complete core type system
- ✅ Rich validation framework
- ✅ Advanced derive macros
- ✅ Performance benchmarks

### Phase 3: Ecosystem & Polish (Weeks 9-12)
**Goal**: Production-ready with ecosystem support

#### Week 9-10: Ecosystem Integration
- [ ] Serde integration and testing
- [ ] Web framework integration examples
- [ ] Node package templates
- [ ] Community crate guidelines

#### Week 11-12: Documentation & Release
- [ ] Complete API documentation
- [ ] Performance optimization
- [ ] Migration guides
- [ ] Release preparation

**Deliverables**:
- ✅ Production-ready performance
- ✅ Complete ecosystem integration
- ✅ Comprehensive documentation
- ✅ Release-ready crates

## 📚 Usage Examples

### Zero-Cost Custom Types with Direct Mapping

```rust
use nebula_value::prelude::*;
use nebula_value_derive::ValueType;

// Zero-cost custom type with derive
#[derive(ValueType)]
pub struct Position3D {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

// Usage - compiles to the same assembly as raw struct
let position = Position3D::new(1.0, 2.0, 3.0);  // Zero overhead
let x = position.x();  // Direct field access, inlined
let y = position.y();  // Direct field access, inlined

// Direct mapping conversion (no JSON!)
let value: Value = position.into_value();  // Direct field mapping
let restored: Position3D = value.try_into()?;  // Direct field extraction
```

### Rich Error Handling

```rust
use nebula_value::prelude::*;

// Text operations with specific errors
let text = Text::new("hello");
match text.validate(MinLength::<10>::new()) {
    Err(ValidationError::Text(TextError::TooShort { min_length, actual_length })) => {
        println!("Text too short: need {}, got {}", min_length, actual_length);
    }
    Ok(()) => println!("Valid text"),
}

// Number operations with unified errors
let number = Integer::new(999999999999999999i64);
match number.validate(InRange::new(0, 1000)) {
    Err(ValidationError::Number(NumberError::IntegerOutOfRange { min, max, actual })) => {
        println!("Integer {} is outside range {}-{}", actual, min, max);
    }
    Ok(()) => println!("Valid integer"),
}

// Float validation with specific errors
let float_val = Float::new(f64::INFINITY);
match float_val.validate(Finite::new()) {
    Err(ValidationError::Number(NumberError::NotFinite { value })) => {
        println!("Float value {} is not finite", value);
    }
    Ok(()) => println!("Valid float"),
}

// Decimal validation (feature: decimal)
#[cfg(feature = "decimal")]
{
    let decimal = Decimal::new_with_scale(123456, 28, 10);
    match decimal.validate() {
        Err(ValidationError::Number(NumberError::InvalidScale { scale, max_scale })) => {
            println!("Decimal scale {} exceeds maximum {}", scale, max_scale);
        }
        Ok(()) => println!("Valid decimal"),
    }
}
```

### Explicit Method Access (No Deref Anti-pattern)

```rust
use nebula_value_derive::ValueType;

#[derive(ValueType)]
pub struct Player {
    pub name: String,
    pub health: i32,
    pub position: Position3D,
}

let mut player = Player::new("Alice".to_string(), 100, Position3D::new(0.0, 0.0, 0.0));

// Explicit access methods (generated by derive)
let name = player.name();           // &String
let health = player.health();       // i32
let pos = player.position();        // &Position3D

// Explicit mutation methods
player.set_health(90);
player.set_position(Position3D::new(1.0, 0.0, 0.0));

// Domain-specific methods (manual implementation)
impl Player {
    pub fn take_damage(&mut self, amount: i32) -> bool {
        let new_health = self.health().saturating_sub(amount);
        self.set_health(new_health);
        new_health > 0
    }
    
    pub fn move_to(&mut self, target: Position3D) {
        self.set_position(target);
    }
    
    pub fn distance_to(&self, other: &Player) -> f32 {
        self.position().distance_to(other.position())
    }
}
```

### Manual Implementation for Complex Cases

```rust
// Complex nested structure requiring manual implementation
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameWorldInner {
    pub players: Vec<Player>,
    pub npcs: Vec<NPC>,
    pub environment: EnvironmentData,
    pub metadata: HashMap<String, DynamicValue>,
}

pub type GameWorld = TypedValue<GameWorldInner>;

impl ValueType for GameWorldInner {
    fn into_value(self) -> Value {
        // Custom mapping for optimal performance
        let mut map = HashMap::new();
        
        // Convert players array with type preservation
        let players_array = self.players.into_iter()
            .map(|player| player.into_value())
            .collect();
        map.insert("players".to_string(), Value::Array(Array::from(players_array)));
        
        // Convert NPCs with compression for large datasets
        let npcs_array = self.npcs.into_iter()
            .map(|npc| npc.into_compressed_value())  // Custom compression
            .collect();
        map.insert("npcs".to_string(), Value::Array(Array::from(npcs_array)));
        
        // Handle environment with versioning
        map.insert("environment".to_string(), self.environment.into_versioned_value());
        
        // Preserve dynamic metadata
        for (key, dynamic_val) in self.metadata {
            map.insert(format!("meta_{}", key), dynamic_val.into_value());
        }
        
        Value::Map(Map::from(map))
    }
    
    fn from_value(value: Value) -> Result<Self, TypeError> {
        // Custom restoration with error recovery
        match value {
            Value::Map(map) => {
                let inner = map.into_inner();
                
                // Restore players with validation
                let players = inner.get("players")
                    .ok_or(TypeError::MissingField("players"))?
                    .clone()
                    .try_into_array()?
                    .into_iter()
                    .map(Player::from_value)
                    .collect::<Result<Vec<_>, _>>()?;
                
                // Restore NPCs with decompression
                let npcs = inner.get("npcs")
                    .ok_or(TypeError::MissingField("npcs"))?
                    .clone()
                    .try_into_array()?
                    .into_iter()
                    .map(NPC::from_compressed_value)  // Custom decompression
                    .collect::<Result<Vec<_>, _>>()?;
                
                // Restore environment with version migration
                let environment = inner.get("environment")
                    .ok_or(TypeError::MissingField("environment"))?
                    .clone()
                    .try_into_environment_data()?;
                
                // Restore metadata
                let metadata = inner.iter()
                    .filter(|(key, _)| key.starts_with("meta_"))
                    .map(|(key, value)| {
                        let clean_key = key.strip_prefix("meta_").unwrap().to_string();
                        let dynamic_val = DynamicValue::from_value(value.clone())?;
                        Ok((clean_key, dynamic_val))
                    })
                    .collect::<Result<HashMap<_, _>, TypeError>>()?;
                
                Ok(GameWorldInner {
                    players,
                    npcs,
                    environment,
                    metadata,
                })
            }
            _ => Err(TypeError::WrongType {
                expected: "Map",
                found: value.type_name(),
            })
        }
    }
}
```

## 📈 Performance Targets with Direct Mapping

| Operation | Target | Notes |
|-----------|--------|-------|
| `TypedValue::new(T)` | 0ns | Should be optimized away completely |
| `TypedValue::inner()` | 0ns | Direct field access, no overhead |
| Field access through explicit methods | 0ns | Should inline to direct access |
| **Direct value mapping** | **< 20ns** | **No JSON serialization overhead** |
| **Direct value restoration** | **< 30ns** | **No JSON deserialization overhead** |
| Core type creation with Into | < 10ns | `Text::new("hello")` or `"hello".into()` |
| Custom type creation with derive | < 15ns | `Position3D::new(1.0, 2.0, 3.0)` |
| Type-specific validation | < 5ns | Basic checks like non-empty, range |
| Complex validation with combinators | < 50ns | Multiple validators with error aggregation |
| **Array/Tuple → Custom type** | **< 20ns** | **`[1.0, 2.0, 3.0].into()` → Position3D** |
| **Chained conversions** | **< 25ns** | **Nested automatic conversions** |

## 🎯 Success Metrics

### Technical Goals
- **True Zero Runtime Overhead**: TypedValue<T> compiles to identical assembly as raw types
- **Direct Value Mapping**: No JSON serialization in critical path
- **Type-Specific Errors**: Rich, actionable error messages for each type
- **Explicit APIs**: Clear method boundaries without Deref confusion

### Developer Experience Goals
- **Consistent Naming**: No suffixes, clean type names across all systems
- **2-Minute Custom Types**: From idea to working validated type with derive macros
- **Zero Boilerplate**: Automatic accessor generation and Into conversions
- **Intuitive APIs**: Multiple creation patterns with explicit method access
- **Excellent Errors**: Rich, type-specific error messages with context

### Ecosystem Goals
- **Node Package Integration**: Easy custom types in all node packages
- **Community Adoption**: Popular community crates with domain-specific types
- **Performance Leadership**: Fastest value system in Rust ecosystem
- **Universal Compatibility**: Works with all major Rust libraries

## 📝 Conclusion

This final architecture achieves optimal performance and maintainability through:

1. **True Zero-Cost Abstractions**: Direct value mapping eliminates JSON serialization overhead
2. **Type-Specific Error Handling**: Rich, actionable errors for each type (TextError, NumberError, etc.)
3. **Explicit Method Access**: Clear APIs without Deref anti-pattern confusion
4. **Modular Design**: Separate derive crate prevents compilation slowdown
5. **Flexible Implementation**: Auto-derive for simple cases, manual implementation for complex needs
6. **Rich Ecosystem Support**: Perfect foundation for node packages and community crates

The direct mapping approach ensures that custom types have the same performance characteristics as core types, while the rich error system and explicit method access provide excellent developer experience. The separation of derive macros into a dedicated crate prevents compilation time impact for users who don't need procedural macros, while still providing powerful ergonomic features for those who do.

This architecture provides the ideal foundation for a high-performance, type-safe value system that can scale from simple data structures to complex domain-specific applications while maintaining zero-cost abstractions and excellent error handling throughout.