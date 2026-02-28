# Combinators

The `combinators` module provides higher-level composition types beyond the primitive
`And/Or/Not/When` in `foundation::traits`.

---

## Logical Combinators

### `and` / `And` / `AndAll`

```rust
// Method style (from ValidateExt):
let v = min_length(5).and(max_length(20));

// Functional style:
let v = and(min_length(5), max_length(20));

// And-all from a vec:
let v = and_all(vec![min_length(5), max_length(20), alphanumeric()]);
```

`AndAll` short-circuits on the first failure.

### `or` / `Or` / `OrAny`

```rust
let v = exact_length(5).or(exact_length(10));
let v = or_any(vec![exact_length(5), exact_length(10)]);
```

`Or` attaches both errors to the result when both fail.

### `not` / `Not`

```rust
let v = contains("banned_word").not();
let v = not(contains("banned_word"));
```

### `all_of` / `any_of` (factory functions)

```rust
let v = all_of(vec![min_length(3), max_length(20), alphanumeric()]);
let v = any_of(vec![email(), url()]);
```

---

## Conditional Combinators

### `When` / `when`

Runs the validator only if `condition(&input)` is true.

```rust
// Only validate length if the string starts with "long_"
let v = min_length(10).when(|s: &str| s.starts_with("long_"));
assert!(v.validate("short").is_ok());       // skipped
assert!(v.validate("long_enough!").is_ok()); // validated
```

### `Unless` / `unless`

Runs the validator only if the condition is **false** (inverse of `When`).

```rust
// Skip validation if the input is the special "N/A" value
let v = unless(|s: &str| s == "N/A", min_length(3));
```

---

## Optional Combinator

### `Optional` / `optional`

Wraps any `Validate<T>` into a `Validate<Option<T>>`. `None` is always valid.

```rust
let v = Optional::new(min_length(5));
// or: let v = optional(min_length(5));

assert!(v.validate(&None::<String>).is_ok());
assert!(v.validate(&Some("hello".to_string())).is_ok());
assert!(v.validate(&Some("hi".to_string())).is_err());
```

---

## Collection Combinator

### `Each` / `each` / `each_fail_fast`

Applies a validator to every element of a collection.

```rust
// Collect all errors
let v = each(min_length(2));
let result = v.validate(&vec!["ok", "x", "fine", ""]); // errors for "x" and ""

// Stop on first error
let v = each_fail_fast(min_length(2));
```

---

## Field Combinator

### `Field` / `field` / `named_field`

Validates a field extracted from a parent struct via a getter closure.

```rust
struct User { name: String, age: u32 }

let name_validator = named_field("name", min_length(2), |u: &User| &u.name);
let age_validator  = named_field("age",  in_range(18, 120), |u: &User| &u.age);

let user_validator = name_validator.and(age_validator);
let user = User { name: "Al".into(), age: 25 };
user_validator.validate(&user)?;
```

`Field` errors automatically include the field name in `ValidationError::field`.

### `MultiField`

Combines multiple `Field` validators over the same struct.

---

## JSON Field Combinator

### `JsonField` / `json_field` / `json_field_optional`

Validates a specific key within a `serde_json::Value` object.

```rust
use nebula_validator::prelude::*;
use serde_json::json;

let v = json_field("email", email());
let data = json!({ "email": "user@example.com", "age": 30 });
v.validate(&data)?;

// Optional key (missing key passes)
let v = json_field_optional("nickname", min_length(2));
```

This is the primary way `nebula-parameter` connects `ValidationRule` descriptors to
actual validation of `serde_json::Value` payloads.

---

## Performance Combinator

### `Cached` / `cached`

Memoizes validation results. Useful for expensive validators (regex compilation,
external lookups) called repeatedly with the same input.

```rust
let v = cached(matches_regex(r"^\w+@\w+\.\w+$"));

v.validate("user@example.com")?;  // validates
v.validate("user@example.com")?;  // instant cache hit

// Cache statistics
let (hits, misses) = v.stats();
```

---

## Message Customization

### `WithMessage` / `with_message` / `WithCode` / `with_code`

Override the error message or code of any validator:

```rust
let v = with_message(min_length(8), "Password must be at least 8 characters");
let v = with_code(email(), "invalid_email");
```

---

## Lazy Combinator

### `Lazy` / `lazy`

Defers validator construction until first call. Useful for avoiding initialization
costs when the validator might not be used.

```rust
let v = lazy(|| matches_regex(r"complex_pattern"));
```

---

## Nested Validation

### `nested_validator` / `custom_nested` / `OptionalNested` / `CollectionNested`

Validates nested structs, attaching field paths to errors:

```rust
let address_v = named_field("city", min_length(2), |a: &Address| &a.city);
let user_v = nested_validator("address", address_v, |u: &User| &u.address);
```

Errors will have `field` set to `"address.city"`.
