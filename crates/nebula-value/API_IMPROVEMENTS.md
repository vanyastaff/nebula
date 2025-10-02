# API Improvements

## Summary of Recent API Enhancements

### 1. Simplified `text()` Method

**Before:**
```rust
// Two separate methods
Value::text(my_string)       // Takes String
Value::text_str("hello")     // Takes &str
```

**After:**
```rust
// Single unified method
Value::text("hello")         // Takes &str
Value::text(my_string)       // Takes String
Value::text(my_str.clone())  // Takes any Into<String>
```

**Benefits:**
- ✅ Cleaner API - one method instead of two
- ✅ More idiomatic Rust - uses `impl Into<String>`
- ✅ Easier to remember and use
- ✅ Backward compatible for String arguments

### 2. Re-exported `json!` Macro

**Before:**
```rust
use nebula_value::prelude::*;
use serde_json::json;  // Had to import separately

let array = ArrayBuilder::new()
    .push(json!(1))
    .build()?;
```

**After:**
```rust
use nebula_value::prelude::*;  // json! included in prelude

let array = ArrayBuilder::new()
    .push(json!(1))
    .build()?;
```

**Alternative:**
```rust
use nebula_value::json;  // Direct import

let array = ArrayBuilder::new()
    .push(json!(1))
    .build()?;
```

**Benefits:**
- ✅ One less import needed
- ✅ More convenient - everything from one crate
- ✅ Cleaner code in examples and user code
- ✅ Available in both root and prelude
- ✅ Conditional on `serde` feature (optional)

## Migration Guide

### Replacing `text_str` → `text`

If you have existing code using `text_str`, simply rename to `text`:

```rust
// Old
let val = Value::text_str("hello");

// New
let val = Value::text("hello");
```

All functionality remains the same - just the name changed.

### Using Re-exported `json!`

**Option 1: Use prelude (recommended)**
```rust
use nebula_value::prelude::*;

// json! is now available
let data = json!({ "key": "value" });
```

**Option 2: Direct import**
```rust
use nebula_value::{Value, json};

let data = json!(42);
```

**Option 3: Keep using serde_json (still works)**
```rust
use nebula_value::Value;
use serde_json::json;

let data = json!(42);
```

## Examples

### Complete Example with New API

```rust
use nebula_value::prelude::*;
use nebula_value::collections::array::ArrayBuilder;
use nebula_value::collections::object::ObjectBuilder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Text values - simplified
    let greeting = Value::text("Hello");
    let name = Value::text("World");

    // Arrays with re-exported json! macro
    let numbers = ArrayBuilder::new()
        .push(json!(1))
        .push(json!(2))
        .push(json!(3))
        .build()?;

    // Objects with re-exported json! macro
    let user = ObjectBuilder::new()
        .insert("name", json!("Alice"))
        .insert("age", json!(30))
        .insert("active", json!(true))
        .build()?;

    // Complex nested structures
    let config = json!({
        "version": "1.0.0",
        "settings": {
            "timeout": 30,
            "retries": 3
        },
        "users": [
            {"id": 1, "name": "Alice"},
            {"id": 2, "name": "Bob"}
        ]
    });

    Ok(())
}
```

## Implementation Details

### `text()` Implementation

```rust
// In src/core/value.rs
impl Value {
    /// Create a text value from String or &str
    pub fn text(v: impl Into<String>) -> Self {
        Self::Text(Text::new(v.into()))
    }
}
```

### `json!` Re-export

```rust
// In src/lib.rs
#[cfg(feature = "serde")]
pub use serde_json::json;

// Also in prelude
pub mod prelude {
    // ... other exports

    #[cfg(feature = "serde")]
    pub use serde_json::json;
}
```

## Testing

All improvements are fully tested:
- ✅ 332 tests pass (190 unit + 117 property + 21 integration + 4 doc)
- ✅ All examples compile and run
- ✅ Backward compatibility maintained
- ✅ No breaking changes to existing functionality

## See Also

- [README.md](README.md) - Main documentation with updated examples
- [examples/json_reexport.rs](examples/json_reexport.rs) - Demonstration of re-exported json! macro
- [examples/basic_usage.rs](examples/basic_usage.rs) - Basic usage with new API
