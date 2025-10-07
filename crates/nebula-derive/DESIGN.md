# nebula-derive Design Document

## Problem: Adding New Validators

**Original Issue**: When adding a new validator to `nebula-validator`, you would need to:
1. Implement the validator in `nebula-validator`
2. Update `nebula-derive/src/validator/parse.rs` to parse the attribute
3. Update `nebula-derive/src/validator/generate.rs` to generate code

This is tedious and creates tight coupling between the two crates.

## Solution: Universal `expr` Attribute

We implemented a **universal expression syntax** that allows using ANY validator without modifying `nebula-derive`.

### How It Works

#### 1. Two Syntax Options

**Option A: Built-in syntax (convenient)**
```rust
#[validate(min_length = 5, max_length = 20)]
username: String
```
- Pros: Clean, intuitive
- Cons: Requires `nebula-derive` update for new validators

**Option B: Universal expr (flexible)**
```rust
#[validate(expr = "min_length(5).and(max_length(20))")]
username: String
```
- Pros: Works with ANY validator, no derive updates needed
- Cons: Slightly more verbose

#### 2. Code Generation

When `expr` is present, it takes **priority** and generates:

```rust
// Generated from: #[validate(expr = "min_length(5).and(max_length(20))")]
if let Err(e) = (min_length(5).and(max_length(20))).validate(&self.username) {
    errors.add(e.with_field("username"));
}
```

The expression is **parsed as Rust code** and injected directly into the validator implementation.

### Architecture

```
User writes:
  #[validate(expr = "my_new_validator()")]
           ↓
nebula-derive parses expr string
           ↓
Generates code: (my_new_validator()).validate(&self.field)
           ↓
At compile time, Rust resolves my_new_validator()
           ↓
Works with ANY validator in scope!
```

### Examples

#### Example 1: New Validator

```rust
// You just added this to nebula-validator
pub fn uuid_v4() -> impl TypedValidator<Input = str, Output = (), Error = ValidationError> {
    // implementation
}

// Use it immediately WITHOUT updating nebula-derive!
#[derive(Validator)]
struct Form {
    #[validate(expr = "nebula_validator::validators::string::uuid_v4()")]
    id: String,
}
```

#### Example 2: Complex Composition

```rust
#[derive(Validator)]
struct User {
    // Complex validator chain
    #[validate(expr = r#"
        min_length(3)
            .and(max_length(20))
            .and(alphanumeric())
            .cached()
    "#)]
    username: String,
}
```

#### Example 3: External Validators

```rust
// Using a validator from a third-party crate
#[derive(Validator)]
struct Data {
    #[validate(expr = "external_crate::special_validator()")]
    special_field: String,
}
```

## Design Decisions

### Why `expr` instead of macro-based registry?

**Considered alternatives:**

1. **Macro Registry** (like `validator_registry!`)
   - Pros: Centralized definition
   - Cons: Still requires updates to registry, complex implementation

2. **Reflection-based** (like `#[validate(call = "min_length", args(5))]`)
   - Pros: Structured
   - Cons: Loses type safety, complex parsing

3. **Universal Expression** (chosen)
   - Pros: Simple, flexible, type-safe at compile time
   - Cons: Slightly more verbose

### Priority System

When generating code, we prioritize:

1. **`expr`** - If present, use ONLY this (full control to user)
2. **Built-in attributes** - If no `expr`, use convenient syntax
3. **Skip** - If neither, and not skipped, no validation

This allows mixing styles:

```rust
#[derive(Validator)]
struct Mixed {
    // Built-in (most common)
    #[validate(min_length = 5)]
    username: String,

    // Universal (for new/complex validators)
    #[validate(expr = "my_custom_validator()")]
    custom: String,
}
```

## Future Extensions

### Possible Enhancements

1. **Import helpers**:
   ```rust
   #[derive(Validator)]
   #[validator(use = "nebula_validator::validators::string::*")]
   struct Form {
       #[validate(expr = "min_length(5)")]  // Shorter!
       field: String,
   }
   ```

2. **Macro expansion** for common patterns:
   ```rust
   // Could expand to complex expr
   #[validate(length = "5..20")]
   ```

3. **Validator builder syntax**:
   ```rust
   #[validate(build = "MinLength::new(5).with_message('Too short')")]
   ```

## Testing Strategy

1. **Unit tests**: Parse different `expr` formats
2. **Integration tests**: Verify generated code compiles and works
3. **Compile-fail tests**: Ensure invalid expressions fail gracefully
4. **Example programs**: Real-world usage patterns

## Performance

- **Zero runtime overhead**: All expressions parsed at compile time
- **No dynamic dispatch**: Direct function calls in generated code
- **Inlining**: Validator chains can be inlined by LLVM
- **Same performance** as hand-written validation code

## Conclusion

The `expr` attribute solves the coupling problem elegantly:

✅ **No need to update `nebula-derive`** when adding validators
✅ **Type-safe** - compiler checks validator expressions
✅ **Zero overhead** - compile-time code generation
✅ **Flexible** - works with ANY validator
✅ **Backwards compatible** - doesn't break existing code

Users can choose:
- **Built-in syntax** for common validators (convenient)
- **`expr` syntax** for new/custom validators (flexible)
