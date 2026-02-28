# Core Traits

## Validate\<T\>

The central trait. Generic over the input type `T`.

```rust
pub trait Validate<T: ?Sized> {
    fn validate(&self, input: &T) -> Result<(), ValidationError>;

    // Validates a serde_json::Value by converting it to T via AsValidatable
    fn validate_any<U: AsValidatable<T>>(&self, input: &U) -> Result<(), ValidationError>
    where Self: Sized;
}
```

**Type safety through trait bounds:**

```rust
// String validators accept AsRef<str> — works with &str, String, Cow<str>
impl<T: AsRef<str> + ?Sized> Validate<T> for MinLength { … }

// Numeric validators accept PartialOrd — works with i32, f64, u8, …
impl<T: PartialOrd + Display + Copy> Validate<T> for Min<T> { … }

// Collection validators accept AsRef<[E]> — works with Vec<E>, &[E], arrays
impl<T: AsRef<[E]>, E> Validate<T> for MinSize { … }

// Compile-time error for invalid combinations:
"hello".validate(&min_length(3));  // ✓ AsRef<str>
42_i32.validate(&min_length(3));   // ✗ i32 doesn't implement AsRef<str>
```

---

## Validatable

Extension trait blanket-implemented for **all** types. Enables the left-to-right style.

```rust
pub trait Validatable {
    fn validate_with<V: Validate<Self>>(&self, validator: &V)
        -> Result<&Self, ValidationError>;
}

// Blanket impl: every T gets this for free
impl<T: ?Sized> Validatable for T { … }
```

Usage:

```rust
"hello".validate_with(&min_length(3))?;

// Chaining
"hello"
    .validate_with(&min_length(3))?
    .validate_with(&max_length(20))?;
```

---

## ValidateExt\<T\>

Combinator methods. Blanket-implemented for every `Validate<T>`.

```rust
pub trait ValidateExt<T: ?Sized>: Validate<T> + Sized {
    fn and<V: Validate<T>>(self, other: V) -> And<Self, V>;
    fn or<V: Validate<T>>(self, other: V)  -> Or<Self, V>;
    fn not(self)                           -> Not<Self>;
    fn when<C: Fn(&T) -> bool>(self, c: C) -> When<Self, C>;
}
```

Examples:

```rust
// AND — short-circuits on first failure
let username = min_length(3).and(max_length(20)).and(alphanumeric());

// OR — short-circuits on first success
let id = uuid().or(email());

// NOT — inverts result
let no_spaces = contains(" ").not();

// WHEN — runs validator only if condition is true
let maybe = min_length(10).when(|s: &str| s.starts_with("long_"));
assert!(maybe.validate("short").is_ok()); // skipped — condition false
```

---

## Primitive Combinators (in `foundation`)

These are the types produced by `ValidateExt` methods:

| Type | Logic |
|---|---|
| `And<L, R>` | Both must pass; short-circuits on first failure |
| `Or<L, R>` | At least one must pass; if both fail, nested errors included |
| `Not<V>` | Passes if inner fails, fails if inner passes |
| `When<V, C>` | Runs validator only when `condition(&input)` is true |

All implement `Validate<T>` and are `Debug + Clone + Copy` (where inner types allow).

---

## Type Aliases

```rust
pub type ValidationResult<T>      = Result<T, ValidationError>;
pub type ValidationResultMulti<T> = Result<T, ValidationErrors>;
```
