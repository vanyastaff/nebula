# nebula-validator — Combinators

Combinators are the composition layer of `nebula-validator`. They take one or more validators
and produce a new validator with different or extended semantics. All combinators are zero-cost
generic types — the compiler monomorphizes and inlines them, producing the same machine code
as hand-written if/else chains.

---

## Logical Combinators

### `And<L, R>` — `.and(v)`

Both validators must pass. Evaluation short-circuits on the first failure.

```rust
use nebula_validator::prelude::*;

let v = min_length(3).and(max_length(20)).and(alphanumeric());
assert!(v.validate("alice").is_ok());
assert!(v.validate("ab").is_err());       // min_length fails; max_length is never called
assert!(v.validate("alice_bob").is_err()); // alphanumeric fails
```

Use `And` when all rules must hold simultaneously and failing fast on the first error is
acceptable. For cases where you need to collect all failures at once, use
[`MultiField`](#multifield---multiple-struct-fields) or [`AllOf`](#allof--anyof---homogeneous-collections).

### `Or<L, R>` — `.or(v)`

At least one validator must pass. Both branches are always evaluated.

```rust
let v = exact_length(8).or(exact_length(16)); // accept 8 or 16-char keys
assert!(v.validate("12345678").is_ok());
assert!(v.validate("1234567890123456").is_ok());
assert!(v.validate("abc").is_err()); // code: or_failed, nested: [exact_length, exact_length]
```

On failure, error code is `or_failed` with both branch errors as nested children.

### `Not<V>` — `.not()`

Inverts the result of the inner validator.

```rust
let v = contains("admin").not(); // reject values that contain "admin"
assert!(v.validate("alice").is_ok());
assert!(v.validate("admin_user").is_err()); // code: not_failed
```

### `When<V, C>` — `.when(predicate)`

Conditionally skips the inner validator. Passes unconditionally when the predicate returns
`false`.

```rust
let v = min_length(10).when(|s: &str| !s.is_empty());
// Empty string: predicate is false → skipped → Ok
// Non-empty string shorter than 10: inner validator runs → Err
assert!(v.validate("").is_ok());
assert!(v.validate("short").is_err());
```

### `Unless<V, C>` — `unless(validator, condition)`

Equivalent to `when(!condition)`. The validator runs when the condition is false.

```rust
use nebula_validator::combinators::unless;

let v = unless(min_length(10), |s: &str| s.starts_with("http://"));
// URLs skip the length check; non-URLs must be ≥10 chars.
```

---

## Conditional Combinators

### `Optional<V>` — `optional(v)`

`None` always passes. `Some(x)` delegates to the inner validator.

```rust
use nebula_validator::combinators::optional;

let v = optional(min_length(3));
assert!(v.validate(&None::<String>).is_ok());
assert!(v.validate(&Some("alice".to_string())).is_ok());
assert!(v.validate(&Some("ab".to_string())).is_err());
```

---

## Collection Combinators

### `Each<V>` — `each(v)` / `each_fail_fast(v)`

Validates every element of a slice. In the default mode, all elements are checked and all
failures are collected. In fail-fast mode, evaluation stops at the first failing element.

```rust
use nebula_validator::combinators::{each, each_fail_fast};

let v = each(min_length(3));
let result = v.validate(&["alice", "ab", "bob", "x"][..]);
// Err: each_failed; params: failed_count=2, failed_indices="1,3"

let v_fast = each_fail_fast(min_length(3));
let result = v_fast.validate(&["alice", "ab", "bob"][..]);
// Err: each_failed; params: index=1 (stops after first failure)
```

**Error code:** `each_failed`
**Params (default mode):** `failed_count`, `total_count`, `failed_indices` (comma-separated)
**Params (fail-fast mode):** `index`

---

## Struct Field Combinators

### `Field<T, U, V, F>` — `field(v, accessor)` / `named_field(name, v, accessor)`

Applies a validator to a single field extracted by a closure. Use `named_field` to attach a
name to the field path in errors.

```rust
use nebula_validator::combinators::named_field;
use nebula_validator::validators::min_length;

struct User { email: String }

let v = named_field("email", min_length(5), |u: &User| u.email.as_str());
v.validate(&User { email: "a@b".into() })?;
// Err: min_length, field: "email"
```

`Field` error path composition: if the inner validator itself produces a field `"sub"`, the
final path becomes `"parent.sub"`.

Extension methods on `Validate<U>` provide a more ergonomic form:

```rust
min_length(5).for_field("email", |u: &User| u.email.as_str())
```

### `MultiField<T>` — multiple struct fields

Validates multiple fields in a single pass, collecting all field errors.

```rust
use nebula_validator::combinators::MultiField;
use nebula_validator::validators::{min_length, in_range};

struct User { name: String, age: u32 }

let v = MultiField::<User>::new()
    .add_field("name", min_length(2), |u: &User| u.name.as_str())
    .add_field("age",  in_range(18u32, 130), |u: &User| &u.age);

// Returns Ok if all pass.
// 1 failure → that ValidationError is returned directly.
// 2+ failures → ValidationError { code: "multiple_field_errors", nested: [...] }
```

---

## JSON Document Combinators

### `json_field(pointer, v)` and `json_field_optional(pointer, v)`

Validates a value at a RFC 6901 JSON Pointer path inside a `serde_json::Value`.

```rust
use nebula_validator::combinators::{json_field, json_field_optional};
use nebula_validator::validators::{min_length, min, is_true};
use serde_json::json;

let v = json_field("/server/host", min_length(1))
    .and(json_field("/server/port", min::<i64>(1)))
    .and(json_field_optional("/server/tls", is_true()));

// Required path missing → code: path_not_found
// Type mismatch (number where string expected) → code: type_mismatch
// Validation failure → inner validator's error code

v.validate(&json!({
    "server": { "host": "localhost", "port": 8080 }
}))?;
```

**Path format:**

| Path | Targets |
|------|---------|
| `/server/host` | `value["server"]["host"]` |
| `/tags/0` | `value["tags"][0]` |
| `""` | Root value |
| `/a~1b` | Key `a/b` (RFC 6901 escaping) |

**Required variant** (`json_field`): missing path → error code `path_not_found`.
**Optional variant** (`json_field_optional`): missing path or `null` value → `Ok(())`.

---

## Factory Combinators

### `AllOf<V>` / `AnyOf<V>` — homogeneous collections

Accept an iterator of validators of the same type. Use `AnyValidator<T>` for heterogeneous
collections.

```rust
use nebula_validator::combinators::{all_of, any_of};
use nebula_validator::prelude::*;

// All must pass — aggregates all failures as nested errors
let v = all_of([min_length(3), max_length(20), alphanumeric()]);

// At least one must pass — empty AnyOf always passes
let v = any_of([exact_length(8), exact_length(16)]);

// Heterogeneous:
let v = all_of([
    AnyValidator::new(min_length(3)),
    AnyValidator::new(email()),
]);
```

---

## Message Override Combinators

Replace the error code or message produced by any inner validator.

```rust
use nebula_validator::combinators::{with_message, with_code};

let v = with_message(min_length(8), "Password must be at least 8 characters.");
let v = with_code(email(), "invalid_email");
```

These are useful when you want to present user-facing messages without exposing internal
error codes, or when you need to map to a specific code expected by a downstream consumer.

---

## `Cached<V>` — memoized validation

Wraps any validator and memoizes results by input hash. Thread-safe via `RwLock`.

```rust
use nebula_validator::combinators::cached;

let v = cached(matches_regex(r"^[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}$")?);
// First call: validates and caches. Subsequent calls with the same input: cache hit.
```

Use `Cached` for:
- Regex validators called repeatedly with a small set of distinct values.
- Expensive custom validators in hot paths.

Do not use `Cached` for validators whose correctness depends on mutable external state.

---

## `Lazy<V>` — deferred construction

Defers validator construction until the first call. Useful when the validator is expensive to
build (e.g., compiling a regex) and may not always be needed.

```rust
use nebula_validator::combinators::lazy;

let v = lazy(|| matches_regex(r"^\d{4}-\d{2}-\d{2}$").expect("valid regex"));
// Regex is compiled only on the first call to validate().
```

---

## Composition Patterns

### Building reusable rule sets

Combine validators into named bindings for reuse across multiple fields or types:

```rust
use nebula_validator::prelude::*;

fn username_rules() -> impl Validate<str> {
    min_length(3).and(max_length(30)).and(alphanumeric())
}

fn password_rules() -> impl Validate<str> {
    min_length(12)
        .and(contains_any_of(["!", "@", "#"]))
        .and(not_all_lowercase())
}
```

### The `compose!` and `any_of!` macros

Shorthand for `And` and `Or` chains:

```rust
// AND-chain: equivalent to a.and(b).and(c)
let v = compose![min_length(3), max_length(20), alphanumeric()];

// OR-chain: equivalent to a.or(b).or(c)
let v = any_of![exact_length(8), exact_length(16), exact_length(32)];
```

### Type erasure for plugin/SDK consumers

When validator types cross crate boundaries (e.g., a plugin registers validators against a
shared registry), use `AnyValidator<T>`:

```rust
fn register_validators(registry: &mut Vec<AnyValidator<str>>) {
    registry.push(AnyValidator::new(min_length(1)));
    registry.push(AnyValidator::new(email()));
}
```

The registry can then iterate and call `validate()` without knowing the concrete types.

---

## Performance Notes

**Static dispatch is always preferred.** A chain `A.and(B).and(C)` compiles to the same code
as three sequential `if` checks. No allocation, no vtable.

**`AnyValidator` costs ~2–5 ns per call** from the vtable indirection. This is negligible for
user-input validation but avoid it in tight inner loops over millions of items.

**`Cached` is effective when:**
- The validator is expensive (>1 µs per call).
- The set of distinct input values is small (high cache hit rate).
- Inputs are `Hash + Eq` — the cache key is a hash of the input.

**`Lazy` saves startup time** but pays a branch on every call to check whether the inner
validator has been initialized. After the first call, the branch is predicted and essentially
free. Use it for module-level statics or conditionally constructed validators.
