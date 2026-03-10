# Macro Authoring & Debugging Guide

How to create custom validators using `validator!`, `compose!`, and `any_of!`.
Covers the macro architecture, all syntax variants, common errors, and
debugging techniques.

---

## Table of Contents

1. [Quick Reference — Syntax Variants](#1-quick-reference)
2. [The `validator!` Macro In Depth](#2-validator-macro-in-depth)
3. [Compose and AnyOf Macros](#3-compose-and-anyof-macros)
4. [Common Errors and Fixes](#4-common-errors-and-fixes)
5. [Debugging Techniques](#5-debugging-techniques)
6. [Advanced Patterns](#6-advanced-patterns)
7. [Checklist for New Validators](#7-checklist)

---

## 1. Quick Reference

### Syntax cheat sheet

```
validator! {
    [#[derive(...)]]           // optional extra derives (Debug, Clone always added)
    [pub] Name                 // visibility + type name
    [<T: Bounds>]              // optional generic with bounds
    [{ field: Type, ... }]     // optional fields
    for InputType;             // the type this validator checks

    rule([self,] input) { ... }    // → bool (true = valid)
    error([self,] input) { ... }   // → ValidationError (on failure)

    [new(args) [-> ErrorType] { ... }]   // optional custom/fallible constructor
    [fn factory(args) [-> ErrorType];]   // optional factory function
}
```

### All 5 variants at a glance

| Variant | Header | `rule`/`error` self | Constructor |
|---------|--------|---------------------|-------------|
| Unit | `Name for T;` | no `self` | auto (unit) |
| Struct | `Name { fields } for T;` | `self, input` | auto from fields |
| Generic bounded | `Name<T: Bounds> { fields } for T;` | `self, input` | auto from fields |
| Phantom unit | `Name<T> for SomeType<T>;` | no `self` | auto (PhantomData) |
| Phantom struct | `Name<T> { fields } for SomeType<T>;` | `self, input` | auto from fields |

---

## 2. `validator!` Macro In Depth

### Architecture (3 layers)

The macro internally works in three stages:

```
User syntax  →  [Layer 1: Entry points]  →  [Layer 2: Tail parser]  →  [Layer 3: @helpers]
                 5 header arms              5 constructor variants      7 code generators
```

**Layer 1** recognizes which variant you're using (unit, struct, generic, phantom)
and normalizes it into a canonical internal form.

**Layer 2** parses the optional constructor/factory section after `rule` + `error`.

**Layer 3** generates the actual Rust code: struct definition, `new()`, `Validate`
impl, and factory function.

### Unit validator (zero-sized)

No fields, no `self` reference:

```rust
validator! {
    /// Rejects empty strings.
    pub NotEmpty for str;
    rule(input) { !input.is_empty() }
    error(_input) { ValidationError::new("not_empty", "must not be empty") }
    fn not_empty();
}

// Generated:
// - #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] pub struct NotEmpty;
// - impl Validate<str> for NotEmpty { ... }
// - pub const fn not_empty() -> NotEmpty { NotEmpty }
```

**Key**: Unit validators get `Copy`, `PartialEq`, `Eq`, `Hash` automatically.
The factory function is `const fn` (zero-cost construction).

### Struct validator (with fields)

Has state, needs `self` in rule/error:

```rust
validator! {
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub MinLength { min: usize } for str;
    rule(self, input) { input.len() >= self.min }
    error(self, input) {
        ValidationError::min_length("", self.min, input.len())
    }
    fn min_length(min: usize);
}

// Generated:
// - pub struct MinLength { pub min: usize }
// - MinLength::new(min: usize) -> Self
// - impl Validate<str> for MinLength { ... }
// - pub fn min_length(min: usize) -> MinLength { MinLength::new(min) }
```

**Key**: Fields are automatically `pub`. Constructor is auto-generated from
field list unless overridden.

### Generic bounded validator

Type parameter with trait bounds:

```rust
validator! {
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub Min<T: PartialOrd + Display + Copy> { min: T } for T;
    rule(self, input) { *input >= self.min }
    error(self, input) {
        ValidationError::new("min", format!("must be >= {}", self.min))
    }
    fn min(value: T);
}
```

**Key**: The generic `T` appears in both the struct and the `for T` target type.
The factory function is generic too: `pub fn min<T: PartialOrd + Display + Copy>(value: T) -> Min<T>`.

### Custom constructor

Override the auto-generated `new()`:

```rust
validator! {
    pub LengthRange { min: usize, max: usize } for str;
    rule(self, input) { let l = input.len(); l >= self.min && l <= self.max }
    error(self, input) { ValidationError::new("length_range", "out of range") }
    new(min: usize, max: usize) {
        Self { min, max }
    }
    fn length_range(min: usize, max: usize);
}
```

### Fallible constructor

Constructor returns `Result`:

```rust
validator! {
    pub StrictRange { lo: usize, hi: usize } for usize;
    rule(self, input) { *input >= self.lo && *input <= self.hi }
    error(self, input) { ValidationError::out_of_range("", self.lo, self.hi, *input) }
    new(lo: usize, hi: usize) -> ValidationError {
        if lo > hi {
            return Err(ValidationError::new("invalid_range", "lo must be <= hi"));
        }
        Ok(Self { lo, hi })
    }
    fn strict_range(lo: usize, hi: usize) -> ValidationError;
}
```

**Key**: The `-> ValidationError` after `new(args)` signals a fallible constructor.
The factory function also returns `Result<StrictRange, ValidationError>`.

---

## 3. Compose and AnyOf Macros

### `compose!` — AND-chain

```rust
use nebula_validator::{compose, prelude::*};

let username = compose!(min_length(3), max_length(20), alphanumeric());
username.validate("alice123")?;
```

Equivalent to `min_length(3).and(max_length(20)).and(alphanumeric())`.

### `any_of!` — OR-chain

```rust
use nebula_validator::{any_of, prelude::*};

let flexible_id = any_of!(exact_length(5), exact_length(10));
flexible_id.validate("ABCDE")?;
```

Equivalent to `exact_length(5).or(exact_length(10))`.

---

## 4. Common Errors and Fixes

### Error: "no rules expected the token `self`"

**Cause**: Unit validator (no fields) but `rule(self, input)` used.

```rust
// ❌ Wrong — unit validators don't have self
validator! {
    pub NotEmpty for str;
    rule(self, input) { !input.is_empty() }  // ERROR
    ...
}

// ✅ Fix — remove self
validator! {
    pub NotEmpty for str;
    rule(input) { !input.is_empty() }
    ...
}
```

**Rule**: `self` is only for struct variants (validators with `{ fields }`).

### Error: "expected `;`" after `for Type`

**Cause**: Missing semicolon after the `for Type` declaration.

```rust
// ❌ Wrong
validator! { pub NotEmpty for str  ... }

// ✅ Fix
validator! { pub NotEmpty for str; ... }
```

### Error: "`self` identifier changed in Rust 2024"

**Cause**: The macro uses `self` as a pattern variable internally.
If you see hygiene-related errors with `self`, ensure the `self` identifier
in `rule(self, input)` and `error(self, input)` matches exactly.

```rust
// ❌ Wrong — mismatched self names
rule(self, input) { ... }
error(s, input) { ... }     // must use `self` too

// ✅ Fix — both use self
rule(self, input) { ... }
error(self, input) { ... }
```

### Error: "trait bound `X: Validate<Y>` is not satisfied"

**Cause**: The `for Type` target doesn't match what you're validating against.

```rust
// ❌ Wrong — validates &str but declared for String
validator! { pub Foo for String; ... }
foo.validate("hello");  // ERROR: "hello" is &str, not String

// ✅ Fix — use str (unsized) for string slice validation
validator! { pub Foo for str; ... }
foo.validate("hello");  // OK
```

### Error: factory function type mismatch

**Cause**: Factory function arguments don't match struct fields or constructor args.

```rust
// ❌ Wrong — factory takes `min` but struct has `minimum`
validator! {
    pub MinLen { minimum: usize } for str;
    ...
    fn min_len(min: usize);  // ERROR: arg doesn't match field name
}

// ✅ Fix — match argument to field name, or use custom constructor
validator! {
    pub MinLen { minimum: usize } for str;
    ...
    new(min: usize) { Self { minimum: min } }
    fn min_len(min: usize);
}
```

### Error: "unused variable" in error arm

**Cause**: Error block doesn't use the `input` parameter.

```rust
// Warning: unused variable `input`
error(self, input) { ValidationError::new("code", "msg") }

// ✅ Fix — prefix with underscore
error(self, _input) { ValidationError::new("code", "msg") }
```

---

## 5. Debugging Techniques

### Expand the macro

Use `cargo expand` to see what the macro generates:

```bash
# Install cargo-expand
cargo install cargo-expand

# Expand a specific module
cargo expand -p nebula-validator validators::length
```

This shows the generated struct, constructor, Validate impl, and factory function.

### cargo check with error highlighting

When working with macro errors, `cargo check` output often points to the
**expanded code** not your source. Look for the "in this macro invocation"
note to find which arm is failing.

### Minimal reproduction

If a macro invocation fails, strip it down to the smallest failing case:

```rust
// Start with the simplest variant that works
validator! {
    pub Test for str;
    rule(input) { true }
    error(_input) { ValidationError::new("test", "test") }
}

// Then add features incrementally:
// 1. Add fields
// 2. Add factory function
// 3. Add generics
// 4. Add custom constructor
```

### Check the `@` helpers

The macro internally uses `@`-prefixed helper arms. If `cargo expand` shows an
error in an `@struct_def` or `@validate_impl` arm, the issue is in the struct
definition or Validate implementation respectively:

| Helper | Generates |
|--------|-----------|
| `@struct_def` | `struct Name { fields }` with derives |
| `@auto_new_impl` | `fn new(fields) -> Self` |
| `@custom_new_impl` | user-provided constructor body |
| `@fallible_new_impl` | user-provided fallible constructor |
| `@validate_impl` | `impl Validate<T> for Name` |
| `@factory_fn` | `pub fn factory(args) -> Name` |
| `@fallible_factory_fn` | `pub fn factory(args) -> Result<Name, E>` |

---

## 6. Advanced Patterns

### Validator with multiple trait bounds

```rust
validator! {
    pub InRange<T: PartialOrd + Display + Copy> { lo: T, hi: T } for T;
    rule(self, input) { *input >= self.lo && *input <= self.hi }
    error(self, input) {
        ValidationError::out_of_range("", self.lo, self.hi, *input)
    }
    fn in_range(lo: T, hi: T);
}
```

### Phantom generic (validate container without owning element)

```rust
validator! {
    pub Required<T> for Option<T>;
    rule(input) { input.is_some() }
    error(_input) { ValidationError::required("") }
    fn required();
}
```

The struct gets a `PhantomData<T>` field automatically.

### Composing macro-generated validators

```rust
let username_validator = compose!(
    min_length(3),
    max_length(20),
    alphanumeric()
);

// Equivalent to:
let username_validator = min_length(3)
    .and(max_length(20))
    .and(alphanumeric());
```

### Extending with combinators post-macro

```rust
// Create a base validator via macro
validator! {
    pub CreditCard for str;
    rule(input) { luhn_valid(input) }
    error(_input) { ValidationError::new("credit_card", "invalid card number") }
    fn credit_card();
}

// Extend with combinators
let full_validator = credit_card()
    .and(min_length(13))
    .and(max_length(19))
    .cached();  // memoize for repeated checks
```

---

## 7. Checklist for New Validators

Before merging a new `validator!` invocation:

- [ ] **Error code** is unique and registered in `error_registry_v1.json`
- [ ] **Error message** is descriptive and uses `format!` with actual values
- [ ] **Factory function** is provided for ergonomic construction
- [ ] **`for` type** is correct (`str` not `String`, unsigned types for size, etc.)
- [ ] **Doc comment** on the validator struct (`///` above the macro invocation)
- [ ] **Unit tests** cover: valid input, invalid input, boundary conditions
- [ ] **Benchmark budget** declared if in a hot path (see [PERFORMANCE.md](PERFORMANCE.md))
- [ ] **`self`** is used only in struct variants (not unit validators)
- [ ] **Derives** include `Copy` for value-type validators (small structs, `usize` fields)

### Error code conventions

| Category | Code Pattern | Examples |
|----------|-------------|----------|
| Length | `min_length`, `max_length`, `exact_length` | Length constraints |
| Content | `invalid_format`, `contains`, `matches_regex` | Content checks |
| Range | `min`, `max`, `out_of_range` | Numeric bounds |
| Presence | `required`, `not_empty` | Null/empty checks |
| Network | `invalid_ipv4`, `invalid_hostname` | Network formats |
| Boolean | `must_be_true`, `must_be_false` | Boolean state |
| Temporal | `invalid_date`, `invalid_time` | Date/time formats |

See the full registry: `tests/fixtures/compat/error_registry_v1.json`
