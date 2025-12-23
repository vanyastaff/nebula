# nebula-value Code Conventions

This document describes the naming conventions and patterns used in `nebula-value`.

## Naming Patterns

### Value Access Methods

#### `as_*()` - Zero-Cost Reference Access
Returns `Option<T>` for safe access without allocation.

```rust
pub fn as_integer(&self) -> Option<Integer>
pub fn as_float(&self) -> Option<Float>
pub fn as_text(&self) -> Option<&Text>
pub fn as_str(&self) -> Option<&str>
```

**Use when:**
- No type conversion needed
- Cheap to return (Copy types or references)
- Returns `None` if type doesn't match

#### `as_*_ref()` - Explicit Reference Access
Returns `Option<&T>` when you explicitly want a reference.

```rust
pub fn as_float_ref(&self) -> Option<&Float>
pub fn as_bytes_ref(&self) -> Option<&Bytes>
```

**Use when:**
- You need a reference specifically
- Distinguishing from owned return in `as_*()`

#### `to_*()` - Converting Access
Returns `ValueResult<T>` with possible type coercion and allocation.

```rust
pub fn to_integer(&self) -> ValueResult<i64>
pub fn to_float(&self) -> ValueResult<f64>
pub fn to_boolean(&self) -> bool  // Infallible conversion
```

**Use when:**
- Type conversion may be needed
- May allocate or compute
- Can fail (returns `Result`)

#### `into_*()` - Consuming Conversion
Consumes `self` and returns the inner value.

```rust
pub fn into_arc(self) -> Arc<str>  // Text
pub fn into_inner(self) -> Array   // BoundedArray
```

**Use when:**
- Consuming conversion
- Extracting inner value
- No cloning needed

### Predicate Methods

#### `is_*()` - Boolean Checks
Returns `bool` for type or state queries.

```rust
pub fn is_null(&self) -> bool
pub fn is_integer(&self) -> bool
pub fn is_numeric(&self) -> bool
pub fn is_empty(&self) -> bool
```

#### `has_*()` - Existence Checks
Returns `bool` for checking presence.

```rust
pub fn has_key(&self, key: &str) -> bool  // Object
pub fn contains(&self, value: &Value) -> bool  // Array
```

### Builder Methods

#### `with_*()` - Builder Pattern
Returns `Self` or `ValueResult<Self>` for chaining.

```rust
pub fn with_limits(s: String, limits: &ValueLimits) -> ValueResult<Self>
```

#### `*_with_limit()` - Limit-Aware Operations
Operations that enforce limits.

```rust
pub fn push_with_limit(&self, value: Value, limits: &ValueLimits) -> ValueResult<Self>
pub fn merge_with_limit(&self, other: &Object, limits: &ValueLimits) -> ValueResult<Self>
```

### Construction Methods

#### `new()` - Default Constructor
```rust
pub fn new() -> Self
pub const fn new(value: i64) -> Self  // When possible, make it const
```

#### `from_*()` - Named Constructors
```rust
pub fn from_vec(vec: Vec<Value>) -> Self
pub fn from_str(s: &str) -> Self
pub fn from_iter<I: IntoIterator>(iter: I) -> Self
```

#### `*_empty()` - Empty Collection Constructors
```rust
pub fn array_empty() -> Self
pub fn object_empty() -> Self
```

## Type Conventions

### Error Types
- Each crate defines its own error type
- Use `thiserror` for error definitions
- Named `*Error`: `ValueError`, `ConversionError`, `SerdeError`
- Result alias: `*Result<T> = Result<T, *Error>`

### Newtype Wrappers
- Named after what they represent: `Integer`, `Float`, `Text`
- Provide `new()` and `value()` methods
- Implement standard traits where appropriate

### Extension Traits
- Named `*Ext`: `ValueExt`, `ArrayExt`, `ObjectExt`
- Provide additional helper methods
- Live in `helpers` module

## Module Organization

```
crates/nebula-value/
├── src/
│   ├── lib.rs           # Public API and docs
│   ├── core/            # Core Value type
│   │   ├── value.rs
│   │   ├── error.rs
│   │   ├── ops.rs
│   │   └── ...
│   ├── scalar/          # Scalar types
│   │   ├── number/
│   │   ├── text/
│   │   └── ...
│   ├── collections/     # Collections
│   │   ├── array/
│   │   └── object/
│   ├── temporal/        # Temporal types
│   ├── bounded/         # Bounded types (const generics)
│   └── helpers/         # Helper traits
```

## Documentation

### Doc Comments
- Use `///` for public items
- Use `//!` for module-level docs
- Include `# Examples` section
- Include `# Errors` section for fallible operations
- Include `# Panics` section if applicable

### Attributes
- `#[must_use]` for methods that return new values
- `#[inline]` for trivial getters
- `#[cfg(feature = "...")]` for feature-gated items
- `#[cfg_attr(docsrs, doc(cfg(feature = "...")))]` for docs.rs

## Rust 2024 Edition Patterns

### Const Generics
```rust
pub struct BoundedText<const MAX: usize> { ... }
pub struct BoundedArray<const MAX: usize> { ... }
```

### impl Trait
```rust
pub fn iter(&self) -> impl Iterator<Item = &Value>
pub fn keys(&self) -> impl Iterator<Item = &String>
```

### Const Functions
```rust
pub const fn new(value: i64) -> Self
pub const fn value(&self) -> i64
pub const fn max_len() -> usize
```

## Testing

### Test Organization
- Unit tests in same file: `#[cfg(test)] mod tests`
- Integration tests in `tests/` directory
- Property-based tests in separate files

### Test Naming
```rust
#[test]
fn test_*() { ... }          // Unit tests
#[test]
fn proptest_*() { ... }      // Property tests
#[test]
#[cfg(feature = "serde")]
fn test_*_serde() { ... }    // Feature-gated tests
```

## Performance Guidelines

### Prefer Immutability
- Use persistent data structures (`im::Vector`, `im::HashMap`)
- Return new instances instead of mutating
- Mark with `#[must_use]` to prevent accidental ignoring

### Avoid Allocations
- Use `&str` parameters, not `String`
- Use `impl Into<String>` when you need to own
- Provide both owned and borrowed access methods

### Use Checked Arithmetic
```rust
pub fn checked_add(self, other: Self) -> Option<Self>
```

Never panic in library code - return `Result` or `Option`.
