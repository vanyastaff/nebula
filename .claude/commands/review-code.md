---
description: Review code changes for quality and architecture
---

Review code changes in {{arg:file_or_scope}}:

## Review Checklist

### 1. **Architecture**
- [ ] Proper pattern applied (not quick patch)?
- [ ] Follows Nebula coding standards?
- [ ] No suppressed warnings (#[allow])?
- [ ] Root cause fixed (not symptoms)?

### 2. **Rust Best Practices**
- [ ] No unnecessary `.clone()`?
- [ ] Proper error handling (Result, not panic)?
- [ ] Lifetime annotations correct?
- [ ] Trait bounds complete?
- [ ] No unsafe code (or justified)?

### 3. **Rust 2024 Compatibility**
- [ ] Explicit type annotations where needed?
- [ ] Sized types in tests (not unsized)?
- [ ] Trait objects only for object-safe traits?
- [ ] No hidden lifetime issues?

### 4. **Testing**
- [ ] Code compiles: `cargo check`?
- [ ] Tests pass: `cargo test`?
- [ ] New tests for new functionality?
- [ ] Edge cases covered?

### 5. **Documentation**
- [ ] Public API documented?
- [ ] Complex logic explained?
- [ ] Examples in doc comments?
- [ ] Architecture decisions noted?

### 6. **Style**
- [ ] Formatted: `cargo fmt`?
- [ ] No clippy warnings: `cargo clippy`?
- [ ] Naming conventions followed?
- [ ] Comments clear and useful?

## Common Issues to Check

### Type Safety
```rust
// ❌ Avoid: mixing types
fn process(id: u64, count: u64)

// ✅ Better: newtype pattern
fn process(id: EntityId, count: Count)
```

### Error Handling
```rust
// ❌ Avoid: unwrap in library code
let value = map.get(key).unwrap();

// ✅ Better: return Result
let value = map.get(key).ok_or(Error::KeyNotFound)?;
```

### Lifetimes
```rust
// ❌ Avoid: unclear lifetimes
fn get<'a>(&'a self, key: &'a str)

// ✅ Better: explicit and clear
fn get<'a, 'b>(&'a self, key: &'b str) -> Option<&'a Value>
```

### Rust 2024 Tests
```rust
// ❌ Avoid: unsized types in Optional/Option
impl TypedValidator for TestValidator {
    type Input = str; // <- Problem!
}

// ✅ Better: sized types
impl TypedValidator for TestValidator {
    type Input = String; // <- Works!
}
```

## Automated Checks

Run these before approval:

```bash
# Format
cargo fmt --all --check

# Clippy
cargo clippy --all-features -- -D warnings

# Tests
cargo test --workspace

# Documentation
cargo doc --no-deps --all-features
```

## Example Usage

```
/review-code crates/nebula-validator/src/combinators/optional.rs
```

This will perform a comprehensive code review checking architecture, style, and correctness.
