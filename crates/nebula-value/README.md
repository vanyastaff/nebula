# nebula-value

[![Crates.io](https://img.shields.io/crates/v/nebula-value.svg)](https://crates.io/crates/nebula-value)
[![Documentation](https://docs.rs/nebula-value/badge.svg)](https://docs.rs/nebula-value)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)

Production-ready value type system for the Nebula workflow engine.

## Features

- 🚀 **High Performance**: O(log n) operations with persistent data structures
- 🛡️ **Type Safety**: Comprehensive error handling, no panics
- 🔄 **Zero-Copy Cloning**: Efficient Arc-based data sharing
- 🎯 **Workflow-Optimized**: Designed for n8n-like use cases
- 📦 **Persistent Collections**: im::Vector and im::HashMap
- 🔒 **Thread-Safe**: Immutable APIs with Arc-based sharing
- 🛡️ **DoS Protection**: Built-in limits for arrays, objects, strings
- 🔧 **Builder Patterns**: Fluent APIs for collection construction
- ⏰ **Temporal Types**: Date, Time, DateTime, Duration with ISO 8601/RFC 3339
- 🔌 **JSON Integration**: Re-exported `json!` macro for convenience

## Quick Start

```rust
use nebula_value::prelude::*;

// Create scalar values
let number = Value::integer(42);
let text = Value::text("hello");
let flag = Value::boolean(true);

// Create temporal values
let date = Date::from_ymd(2024, 1, 15)?;
let date_val = Value::date(date);

// Operations
let sum = Value::integer(10).add(&Value::integer(5))?;  // 15
let concat = Value::text("Hello ").add(&Value::text("World"))?;

// Collections with builders
use nebula_value::collections::array::ArrayBuilder;
use nebula_value::json;  // Re-exported from serde_json

let array = ArrayBuilder::new()
    .push(json!(1))
    .push(json!(2))
    .push(json!(3))
    .build()?;

// Type conversions
let num: i64 = Value::integer(42).as_integer().unwrap();
let text: String = String::try_from(Value::text("hello"))?;

// Serialization
let json = serde_json::to_string(&Value::integer(42))?;
let value: Value = serde_json::from_str(&json)?;
```

## Value Types

### Scalar Types

| Type | Description | Storage |
|------|-------------|---------|
| `Null` | Absence of value | Unit |
| `Boolean` | True/false | bool |
| `Integer` | 64-bit signed integer | i64 |
| `Float` | 64-bit floating point | f64 |
| `Decimal` | Arbitrary precision | Decimal |
| `Text` | UTF-8 text | Arc\<str\> |
| `Bytes` | Binary data | bytes::Bytes |

### Temporal Types

| Type | Description | Format |
|------|-------------|--------|
| `Date` | Calendar date | ISO 8601 (YYYY-MM-DD) |
| `Time` | Time of day | ISO 8601 (HH:MM:SS) |
| `DateTime` | Date + Time + Timezone | RFC 3339 |
| `Duration` | Time span | Milliseconds |

### Collection Types

| Type | Description | Storage |
|------|-------------|---------|
| `Array` | Ordered collection | im::Vector |
| `Object` | Key-value map | im::HashMap |

## Operations

### Arithmetic
```rust
let a = Value::integer(10);
let b = Value::integer(5);

a.add(&b)?;  // 15
a.sub(&b)?;  // 5
a.mul(&b)?;  // 50
a.div(&b)?;  // 2
a.rem(&b)?;  // 0
```

### Comparison
```rust
let a = Value::integer(10);
let b = Value::integer(5);

a.gt(&b)?;   // true
a.lt(&b)?;   // false
a.ge(&b)?;   // true
a.le(&b)?;   // false
```

### Logical
```rust
let t = Value::boolean(true);
let f = Value::boolean(false);

t.and(&f);   // false
t.or(&f);    // true
t.not();     // false
```

### Merge
```rust
use nebula_value::collections::object::ObjectBuilder;
use nebula_value::json;

let obj1 = ObjectBuilder::new()
    .insert("a", json!(1))
    .build()?;

let obj2 = ObjectBuilder::new()
    .insert("b", json!(2))
    .build()?;

let merged = Value::Object(obj1).merge(&Value::Object(obj2))?;
```

## Builders

### ArrayBuilder
```rust
use nebula_value::collections::array::ArrayBuilder;
use nebula_value::core::limits::ValueLimits;
use nebula_value::json;

let array = ArrayBuilder::new()
    .with_limits(ValueLimits::strict())  // Optional validation
    .push(json!(1))
    .push(json!(2))
    .push(json!(3))
    .build()?;
```

### ObjectBuilder
```rust
use nebula_value::collections::object::ObjectBuilder;
use nebula_value::json;

let object = ObjectBuilder::new()
    .insert("name", json!("Alice"))
    .insert("age", json!(30))
    .insert("active", json!(true))
    .build()?;
```

## Temporal Types

> ✅ **Status**: Temporal types have been **migrated to v2 architecture**. They support:
> - ✅ Unified error handling with `NebulaError`
> - ✅ Optional inclusion via `temporal` feature flag
> - ✅ System time methods conditionally available with `std` feature
> - ✅ no_std compatible (with reduced functionality)
>
> See [PHASE2_COMPLETED.md](PHASE2_COMPLETED.md) for details.

Work with dates, times, and durations:

```rust
use nebula_value::prelude::*;

// Dates
let date = Date::from_ymd(2024, 1, 15)?;
let date_value = Value::date(date);
println!("{}", date_value);  // "2024-01-15"

// Times
let time = Time::from_hms(14, 30, 0)?;
let time_value = Value::time(time);
println!("{}", time_value);  // "14:30:00"

// DateTimes
let now = DateTime::now();  // Requires 'std' feature
let dt_value = Value::datetime(now);
println!("{}", dt_value);  // "2024-01-15T14:30:00Z"

// Durations
let hour = Duration::from_seconds(3600);
let dur_value = Value::duration(hour);

// Arithmetic with dates
let tomorrow = date.add_days(1)?;
let yesterday = date.sub_days(1)?;

// Comparisons
assert!(date < tomorrow);
assert!(date > yesterday);

// Serialization
let json = serde_json::to_string(&date_value)?;  // "2024-01-15"
```

## Limits & Validation

Protect against DoS attacks with configurable limits:

```rust
use nebula_value::core::limits::ValueLimits;

// Strict limits for untrusted input
let limits = ValueLimits::strict();

// Custom limits
let limits = ValueLimits {
    max_array_length: 1000,
    max_object_keys: 100,
    max_string_bytes: 10_000,
    max_bytes_length: 100_000,
    max_nesting_depth: 10,
};
```

## Hashing

Use `HashableValue` for HashMap/HashSet:

```rust
use std::collections::HashMap;
use nebula_value::core::hash::HashableValue;

let mut map = HashMap::new();
map.insert(HashableValue(Value::integer(42)), "answer");

// NaN handling: all NaN values are equal for hashing
let nan1 = HashableValue(Value::float(f64::NAN));
let nan2 = HashableValue(Value::float(f64::NAN));
assert_eq!(nan1, nan2);  // true for HashMap purposes
```

## Serialization

Full serde support with special value handling:

```rust
// Serialize
let value = Value::integer(42);
let json = serde_json::to_string(&value)?;  // "42"

// Deserialize
let value: Value = serde_json::from_str("42")?;

// Special values
Value::float(f64::NAN);       // serializes as null
Value::float(f64::INFINITY);  // serializes as "+Infinity"
```

## Performance

- **Persistent Data Structures**: O(log n) operations with structural sharing
- **Zero-Copy**: Arc-based cloning, no data duplication
- **Small Value Optimization**: Inline storage for small values
- **Thread-Safe**: Lock-free immutable operations

## Examples

See the [examples](examples/) directory:
- [`basic_usage.rs`](examples/basic_usage.rs) - Core functionality
- [`operations.rs`](examples/operations.rs) - Arithmetic, comparison, logical ops
- [`limits_and_validation.rs`](examples/limits_and_validation.rs) - DoS protection
- [`json_reexport.rs`](examples/json_reexport.rs) - Using re-exported `json!` macro

Run examples:
```bash
cargo run --example basic_usage --features serde
cargo run --example operations --features serde
cargo run --example limits_and_validation --features serde
cargo run --example json_reexport --features serde
```

## Testing

```bash
# Run all tests
cargo test --lib

# Run with all features
cargo test --lib --all-features

# Run specific module tests
cargo test --lib core::
cargo test --lib collections::
```

**Current test coverage: 322 tests passing**
- 220 unit tests (scalar, collections, temporal, core)
- 77 property-based tests
- 21 integration tests
- 4 doc tests

## Features

- `default` = `["std", "temporal"]`: Standard library + temporal types (recommended)
- `std`: Standard library support (enables system time methods)
- `temporal`: Date, Time, DateTime, Duration types (requires chrono)
- `serde`: JSON serialization/deserialization
- `full` = `["std", "serde", "temporal"]`: All features enabled

```toml
# Default (includes temporal types)
[dependencies]
nebula-value = "0.1"

# With serde support
[dependencies]
nebula-value = { version = "0.1", features = ["serde"] }

# Without temporal types (smaller binary)
[dependencies]
nebula-value = { version = "0.1", default-features = false, features = ["std"] }

# no_std with temporal types (reduced functionality)
[dependencies]
nebula-value = { version = "0.1", default-features = false, features = ["temporal"] }
```

## Architecture

```
nebula-value/
├── core/           # Core Value type and operations
│   ├── value.rs    # Value enum (13 variants)
│   ├── ops.rs      # Arithmetic, comparison, logical
│   ├── conversions.rs  # Type conversions
│   ├── hash.rs     # HashableValue wrapper
│   ├── serde.rs    # JSON serialization
│   └── ...
├── scalar/         # Scalar types (Integer, Float, Text, Bytes)
├── collections/    # Array, Object with builders
└── temporal/       # Date, Time, DateTime, Duration
    ├── date.rs     # Calendar dates
    ├── time.rs     # Time of day
    ├── datetime.rs # Date + time + timezone
    └── duration.rs # Time spans
```

## Contributing

Contributions are welcome! Please read our [Contributing Guide](../../CONTRIBUTING.md).

## License

Licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](../../LICENSE-APACHE))
- MIT license ([LICENSE-MIT](../../LICENSE-MIT))

at your option.

## See Also

- [nebula-error](../nebula-error) - Error handling
- [nebula-log](../nebula-log) - Logging utilities
- [Nebula Workflow Engine](https://github.com/vanyastaff/nebula)