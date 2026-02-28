# Complete Type Guide

> Reference for value types and conversion: используется **serde_json::Value** и serde (crate nebula-value не используется)

## Type Hierarchy Overview

```
TypedValue<T>              // Zero-cost wrapper
├── Text                   // String data
├── Bool                   // Boolean values
├── Numeric Types
│   ├── Integer            // i64-based (recommended)
│   ├── UInteger           // u64-based (recommended)
│   ├── Float              // f64-based (recommended)
│   ├── Decimal            // High-precision decimal
│   ├── Fixed-Size Integers (feature-gated)
│   │   ├── I8, I16, I32, I64
│   │   └── U8, U16, U32, U64
│   └── Fixed-Size Floats (feature-gated)
│       └── F32, F64
├── Collections
│   ├── Array              // Dynamic arrays
│   └── Map                // Key-value maps
├── Binary Data
│   └── Bytes              // Binary data with encoding
├── Pattern Matching
│   └── Pattern            // Regex patterns
└── Temporal (feature-gated)
    ├── DateTime           // Date and time
    ├── Date               // Date only
    ├── Time               // Time only
    └── Duration           // Time spans
```

## Text Type

**Purpose**: Safe string handling with validation support.

### Basic Usage

```rust
use nebula_value::prelude::*;

// Creation
let text = Text::new("Hello, World!");
let empty = Text::empty();
let from_string = Text::from(String::from("Owned string"));

// Access (zero-cost)
assert_eq!(text.as_ref().len(), 13);
assert_eq!(text.as_ref().chars().count(), 13);
assert_eq!(text.as_ref().is_empty(), false);

// String operations
let uppercase = text.as_ref().to_uppercase();
let lowercase = text.as_ref().to_lowercase();
let trimmed = text.as_ref().trim();

// Conversion
let owned: String = text.into_inner().into();
```

### Common Validation

```rust
use nebula_value::validation::*;

let username = Text::new("user123");

// Length validation
username.validate(&MinLength::new(3))?;
username.validate(&MaxLength::new(20))?;
username.validate(&ExactLength::new(7))?;

// Content validation
username.validate(&AlphanumericOnly)?;
username.validate(&NoWhitespace)?;

// Pattern validation
let email = Text::new("user@example.com");
email.validate(&Email)?;

let phone = Text::new("+1-555-123-4567");
phone.validate(&Phone)?;

let url = Text::new("https://example.com");
url.validate(&Url)?;
```

### Text Methods

```rust
impl Text {
    pub fn new(value: impl Into<String>) -> Self
    pub fn empty() -> Self
    pub fn is_empty(&self) -> bool
    pub fn len(&self) -> usize
    pub fn as_str(&self) -> &str                    // Via as_ref()
    pub fn into_string(self) -> String              // Via into_inner()
}
```

## Numeric Types

### Dynamic Types (Recommended)

These types handle 98% of numeric use cases:

#### Integer (i64-based)

```rust
use nebula_value::prelude::*;

// Creation
let count = Integer::new(42);
let negative = Integer::new(-100);
let from_primitive = Integer::from(42i64);

// Access
assert_eq!(count.as_ref().get(), 42);
assert_eq!(count.as_ref().is_negative(), false);
assert_eq!(count.as_ref().abs(), 42);

// Safe downcasting with validation
let as_u8: u8 = count.as_ref().to_u8()?;           // May fail if out of range
let as_u16: u16 = count.as_ref().to_u16()?;
let as_i32: i32 = count.as_ref().to_i32()?;

// Arithmetic (returns new Integer)
let sum = Integer::new(10) + Integer::new(5);      // Integer(15)
let diff = Integer::new(10) - Integer::new(3);     // Integer(7)
let product = Integer::new(6) * Integer::new(7);   // Integer(42)
```

#### UInteger (u64-based)

```rust
use nebula_value::prelude::*;

// For unsigned values and large numbers
let big_id = UInteger::new(18446744073709551615u64);  // u64::MAX
let count = UInteger::new(1000);
let from_primitive = UInteger::from(42u64);

// Safe downcasting
let byte: u8 = count.as_ref().to_u8()?;
let port: u16 = count.as_ref().to_u16()?;
let id: u32 = count.as_ref().to_u32()?;

// Common use cases
let timestamp_micros = UInteger::new(1640995200000000);
let file_size_bytes = UInteger::new(1073741824);    // 1GB
let user_id = UInteger::new(12345);
```

#### Float (f64-based)

```rust
use nebula_value::prelude::*;

// Creation
let score = Float::new(98.5);
let pi = Float::new(std::f64::consts::PI);
let from_primitive = Float::from(42.0f64);

// Access
assert_eq!(score.as_ref().get(), 98.5);
assert!(score.as_ref().is_finite());
assert!(!score.as_ref().is_nan());

// Math operations
let celsius = Float::new(20.0);
let fahrenheit = celsius.as_ref().get() * 9.0 / 5.0 + 32.0;
let fahrenheit = Float::new(fahrenheit);

// Rounding
let rounded = score.as_ref().round();               // 99.0
let floor = score.as_ref().floor();                 // 98.0  
let ceil = score.as_ref().ceil();                   // 99.0
```

#### Decimal (High-precision)

```rust
#[cfg(feature = "decimal")]
use nebula_value::prelude::*;

// For financial calculations requiring exact precision
let price = Decimal::new_from_str("19.99")?;
let tax_rate = Decimal::new_from_str("0.08")?;

// Exact arithmetic (no floating-point errors)
let tax = price * tax_rate;                         // Exactly 1.5992
let total = price + tax;                            // Exactly 21.5892

// Precision control
let rounded_total = total.round_dp(2);              // 21.59
let formatted = total.to_string();                  // "21.5892"

// Common financial operations
let discount_percent = Decimal::new_from_str("15.0")?;
let discount_amount = price * (discount_percent / Decimal::new_from_str("100.0")?);
let final_price = price - discount_amount;
```

### Fixed-Size Types (Feature-gated)

Use these only when you specifically need fixed sizes:

```rust
#[cfg(feature = "popular-ints")]
use nebula_value::prelude::*;

// 8-bit unsigned (bytes, flags, RGB values)
let byte = U8::new(255);
let red = U8::new(128);
let flags = U8::new(0b10101010);

// Bit operations
assert_eq!(flags.as_ref().count_ones(), 4);
assert_eq!(flags.as_ref().rotate_left(1), 0b01010101);

// 32-bit integers (common in APIs)
let user_id = I32::new(123456);
let timestamp = U32::new(1640995200);

#[cfg(feature = "popular-floats")]
// 32-bit float (GPU, games, space optimization)
let coordinate = F32::new(1.5);
let vertex_data = vec![F32::new(0.0), F32::new(1.0), F32::new(0.5)];
```

### Numeric Validation

```rust
use nebula_value::validation::*;

let age = Integer::new(25);
let score = Float::new(85.7);
let price = Decimal::new_from_str("29.99")?;

// Range validation
age.validate(&InRange::new(0i64, 120i64))?;
score.validate(&InRange::new(0.0, 100.0))?;

// Sign validation
age.validate(&Positive)?;
age.validate(&NonZero)?;

// Parity validation
let even_number = Integer::new(42);
even_number.validate(&Even)?;

let odd_number = Integer::new(43);
odd_number.validate(&Odd)?;

// Divisibility
let multiple = Integer::new(15);
multiple.validate(&DivisibleBy::new(5))?;

// Float-specific validation
score.validate(&Finite)?;              // Not NaN or infinite
score.validate(&NotNaN)?;
```

## Boolean Type

```rust
use nebula_value::prelude::*;

// Creation
let active = Bool::new(true);
let inactive = Bool::new(false);
let from_str = Bool::from_str("true")?;         // Parses "true"/"false"
let from_int = Bool::from_int(1)?;              // 1=true, 0=false

// Access
assert_eq!(active.as_ref().get(), true);
assert_eq!(active.as_ref().is_true(), true);
assert_eq!(active.as_ref().is_false(), false);

// Logical operations
let result = active.as_ref().get() && inactive.as_ref().get();  // false

// Validation
let consent = Bool::new(true);
consent.validate(&MustBeTrue)?;                 // For consent checkboxes

let optional_flag = Bool::new(false);
optional_flag.validate(&MustBeFalse)?;          // For opt-out scenarios
```

## Collection Types

### Array Type

```rust
use nebula_value::prelude::*;

// Creation
let numbers = Array::from(vec![
    Integer::new(1).into_value(),
    Integer::new(2).into_value(),
    Integer::new(3).into_value(),
]);

let mixed = Array::from(vec![
    Text::new("hello").into_value(),
    Integer::new(42).into_value(),
    Bool::new(true).into_value(),
]);

// Access
assert_eq!(numbers.len(), 3);
assert!(!numbers.is_empty());

// Get elements by index
let first: Integer = numbers.try_get_index(0)?;
let last: Integer = numbers.try_get_index(2)?;

// Iteration
for (index, value) in numbers.iter().enumerate() {
    match value {
        Value::Integer(n) => println!("numbers[{}] = {}", index, n.as_ref()),
        _ => unreachable!(),
    }
}

// Modification
let mut mutable_array = Array::new();
mutable_array.push(Text::new("item1").into_value());
mutable_array.push(Text::new("item2").into_value());

// Validation
numbers.validate(&MinItems::new(1))?;
numbers.validate(&MaxItems::new(10))?;
numbers.validate(&UniqueItems)?;               // All items must be unique
```

### Map Type

```rust
use nebula_value::prelude::*;

// Creation
let mut user = Map::new();
user.insert("name", Text::new("Alice").into_value());
user.insert("age", Integer::new(30).into_value());
user.insert("active", Bool::new(true).into_value());

// Access with type safety
let name: Text = user.try_get("name")?;
let age: Integer = user.try_get("age")?;
let active: Bool = user.try_get("active")?;

// Check existence
assert!(user.contains_key("name"));
assert!(!user.contains_key("email"));

// Nested access with paths
let mut nested = Map::new();
let mut address = Map::new();
address.insert("city", Text::new("New York").into_value());
address.insert("zip", Text::new("10001").into_value());
nested.insert("address", address.into_value());

let city: Text = nested.try_get("address.city")?;

// Iteration
for (key, value) in user.iter() {
    println!("{}: {:?}", key, value);
}

// Validation
user.validate(&RequiredKeys::new(vec!["name", "email"]))?;
user.validate(&MinSize::new(2))?;
user.validate(&MaxSize::new(10))?;
```

## Binary Data Type

```rust
#[cfg(feature = "bytes")]
use nebula_value::prelude::*;

// Creation
let data = Bytes::new(vec![0x48, 0x65, 0x6C, 0x6C, 0x6F]);  // "Hello"
let from_slice = Bytes::from(&[1, 2, 3, 4][..]);
let empty = Bytes::empty();

// Access
assert_eq!(data.len(), 5);
assert_eq!(data.as_ref().as_slice(), &[0x48, 0x65, 0x6C, 0x6C, 0x6F]);

// Encoding (requires "encoding" feature)
#[cfg(feature = "encoding")]
{
    let hex_string = data.to_hex();                 // "48656c6c6f"
    let base64_string = data.to_base64();           // "SGVsbG8="
    
    let from_hex = Bytes::from_hex("48656c6c6f")?;
    let from_base64 = Bytes::from_base64("SGVsbG8=")?;
}

// Validation
data.validate(&MinSize::new(1))?;
data.validate(&MaxSize::new(1024))?;
data.validate(&ValidUtf8)?;                        // If containing text
```

## Pattern Type

```rust
#[cfg(feature = "pattern")]
use nebula_value::prelude::*;

// Creation
let email_pattern = Pattern::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$")?;
let phone_pattern = Pattern::new(r"^\+?1?-?\.?\s?\(?(\d{3})\)?[-\.\s]?(\d{3})[-\.\s]?(\d{4})$")?;

// Matching
let email = "user@example.com";
assert!(email_pattern.is_match(email));
assert!(!phone_pattern.is_match(email));

// Capture groups
let captures = email_pattern.captures(email)?;
if let Some(domain) = captures.get(1) {
    println!("Domain: {}", domain.as_str());
}

// Find all matches
let text = "Contact: john@example.com or jane@test.org";
let matches: Vec<&str> = email_pattern.find_iter(text)
    .map(|m| m.as_str())
    .collect();
assert_eq!(matches, vec!["john@example.com", "jane@test.org"]);
```

## Temporal Types

```rust
#[cfg(feature = "temporal")]
use nebula_value::prelude::*;
use chrono::{Utc, NaiveDate, NaiveTime};

// DateTime
let now = DateTime::now();                          // Current UTC time
let from_timestamp = DateTime::from_timestamp(1640995200)?;
let from_str = DateTime::parse("2024-01-01T12:00:00Z")?;

// Date
let today = Date::today();                          // Current date
let birthday = Date::new(1990, 6, 15)?;
let from_str = Date::parse("2024-01-01")?;

// Time
let noon = Time::new(12, 0, 0)?;
let precise = Time::new_with_nanos(14, 30, 45, 123456789)?;
let from_str = Time::parse("14:30:45")?;

// Duration
let one_hour = Duration::hours(1);
let thirty_minutes = Duration::minutes(30);
let combined = one_hour + thirty_minutes;           // 90 minutes

// Formatting
println!("ISO: {}", now.to_rfc3339());
println!("Local: {}", now.format("%Y-%m-%d %H:%M:%S"));

// Validation
let meeting_time = DateTime::parse("2024-06-15T14:30:00Z")?;
meeting_time.validate(&InFuture)?;
meeting_time.validate(&BusinessHours)?;

let age = Date::new(1990, 1, 1)?;
age.validate(&InPast)?;
age.validate(&MinAge::new(18))?;
```

## Type Conversions

### Between Numeric Types

```rust
use nebula_value::prelude::*;

// Safe conversions (with validation)
let big_int = Integer::new(1000);
let small_int: u8 = big_int.as_ref().to_u8()?;     // Ok: 1000 fits in u8? No -> Error

let uint = UInteger::new(42);
let int: i64 = uint.as_ref().to_i64()?;            // Ok: 42 fits in i64

// Lossy conversions (explicit)
let float_val = Float::new(42.7);
let truncated: i64 = float_val.as_ref().trunc() as i64;  // 42

// Cross-type arithmetic requires explicit conversion
let int_val = Integer::new(10);
let float_val = Float::new(3.14);
let result = int_val.as_ref().get() as f64 * float_val.as_ref().get();
```

### To/From Universal Value

```rust
use nebula_value::prelude::*;

// Any typed value -> Value (zero-cost)
let text = Text::new("hello");
let value: Value = text.into_value();

// Value -> typed value (with validation)
let restored_text = Text::try_from(value)?;

// Pattern matching on Value
match some_value {
    Value::Text(t) => handle_text(t),
    Value::Integer(i) => handle_integer(i),
    Value::Bool(b) => handle_bool(b),
    _ => handle_other(some_value),
}
```

### String Conversions

```rust
use nebula_value::prelude::*;

// From strings
let int_from_str = Integer::from_str("42")?;
let float_from_str = Float::from_str("3.14159")?;
let bool_from_str = Bool::from_str("true")?;
let decimal_from_str = Decimal::new_from_str("19.99")?;

// To strings
let int_string = Integer::new(42).to_string();      // "42"
let float_string = Float::new(3.14159).to_string(); // "3.14159"
let bool_string = Bool::new(true).to_string();      // "true"
```

## Memory Layout & Performance

All types are zero-cost wrappers:

```rust
use nebula_value::*;
use std::mem::size_of;

// Same size as underlying types
assert_eq!(size_of::<Text>(), size_of::<String>());
assert_eq!(size_of::<Integer>(), size_of::<i64>());
assert_eq!(size_of::<UInteger>(), size_of::<u64>());
assert_eq!(size_of::<Bool>(), size_of::<bool>());
assert_eq!(size_of::<Float>(), size_of::<f64>());

// Arrays have minimal overhead
assert_eq!(size_of::<Array>(), size_of::<Vec<Value>>());
assert_eq!(size_of::<Map>(), size_of::<std::collections::HashMap<String, Value>>());
```

## Type Selection Guidelines

### When to Use Each Type

**Text**: Any string data, user input, names, descriptions, URLs, emails
```rust
let name = Text::new("Alice Johnson");
let description = Text::new("A passionate Rust developer");
```

**Integer**: Most numeric data, IDs, counts, ages, years
```rust
let user_id = Integer::new(12345);
let age = Integer::new(28);
let year = Integer::new(2024);
```

**UInteger**: Large positive numbers, timestamps, file sizes, memory addresses
```rust
let timestamp_micros = UInteger::new(1640995200000000);
let file_size = UInteger::new(1073741824);  // 1GB
```

**Float**: Scores, percentages, measurements, scientific data
```rust
let score = Float::new(87.5);
let temperature = Float::new(23.7);
```

**Decimal**: Money, prices, financial calculations, exact arithmetic
```rust
let price = Decimal::new_from_str("19.99")?;
let tax_rate = Decimal::new_from_str("0.08")?;
```

**Bool**: Flags, switches, yes/no values, feature toggles
```rust
let active = Bool::new(true);
let email_verified = Bool::new(false);
```

**Fixed-size types**: Only when interfacing with C APIs, binary protocols, or performance-critical code
```rust
#[cfg(feature = "popular-ints")]
let rgb_r = U8::new(255);  // Byte values
```

### Performance Considerations

- **Use dynamic types first**: Integer, UInteger, Float cover 98% of cases
- **Avoid unnecessary conversions**: Pick the right type upfront
- **Cache validators**: For repeated validation of similar data
- **Use const validation**: For compile-time known values

```rust
// ✅ Efficient - right type from start
let user_id = Integer::new(12345);

// ❌ Inefficient - unnecessary conversion
let user_id = U32::new(12345).as_ref().to_i64();
```

See [Performance Guide](performance.md) for detailed benchmarks and optimization strategies.