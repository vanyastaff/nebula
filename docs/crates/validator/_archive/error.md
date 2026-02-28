# ValidationError

Structured error type for validation failures. Optimized for the common case (code +
message only) with lazy heap allocation for extended metadata.

---

## Memory Layout (≤ 80 bytes)

```
code    : Cow<'static, str>         — 24 bytes (borrowed for static strings = 0 alloc)
message : Cow<'static, str>         — 24 bytes
field   : Option<Cow<'static, str>> — 24 bytes
extras  : Option<Box<ErrorExtras>>  —  8 bytes (None unless params/nested/severity/help used)
```

Static string codes/messages cost **zero allocations**. Dynamic strings allocate only
the `String` itself. `ErrorExtras` (params, nested errors, severity, help) is lazily
allocated on first use — the typical single-error case pays no overhead.

---

## Construction

```rust
// Zero-allocation (static strings):
let e = ValidationError::new("min_length", "String is too short");

// Dynamic message (allocates only the String):
let e = ValidationError::new("min_length", format!("Must be at least {min} chars"));
```

### Builder Methods

```rust
e.with_field("user.email")            // sets the field path
 .with_param("min", "5")              // adds a template parameter
 .with_param("actual", "3")
 .with_severity(ErrorSeverity::Warning)
 .with_help("Use at least 5 characters")
 .with_nested(vec![nested_err])       // attach child errors
 .with_nested_error(single_nested)    // attach one child error
```

All builder methods are `#[must_use]` and return `Self` for chaining.

---

## Accessors

```rust
err.code          // &Cow<'static, str>
err.message       // &Cow<'static, str>
err.field         // Option<&Cow<'static, str>>
err.param("min")  // Option<&str>
err.params()      // &[(Cow, Cow)] — all parameters
err.nested()      // &[ValidationError] — nested errors
err.has_nested()  // bool
err.severity()    // ErrorSeverity (default: Error)
err.help()        // Option<&str>

err.total_error_count()  // 1 + all nested counts (recursive)
err.flatten()            // Vec<&ValidationError> — depth-first flat list
err.to_json_value()      // serde_json::Value — for serialization
```

**Display format:**
```
[user.email] min_length: Must be at least 5 characters (params: [min=5, actual=3])
  Help: Use at least 5 characters
  Nested errors:
    1. required: This field is required
```

---

## ErrorSeverity

```rust
pub enum ErrorSeverity {
    Error,    // default — must be fixed
    Warning,  // should be addressed but doesn't block
    Info,     // informational
}
```

---

## Convenience Constructors

```rust
ValidationError::required("email")
ValidationError::min_length("password", 8, 3)   // field, min, actual
ValidationError::max_length("name", 255, 300)
ValidationError::invalid_format("date", "ISO 8601")
ValidationError::type_mismatch("age", "number", "string")
ValidationError::out_of_range("score", 0, 100, 150)
ValidationError::exact_length("pin", 4, 6)
ValidationError::length_range("bio", 10, 500, 3)
ValidationError::custom("Business rule violated")
```

---

## ValidationErrors

Collection of errors for multi-error scenarios:

```rust
pub struct ValidationErrors { errors: Vec<ValidationError> }

let mut errs = ValidationErrors::new();
errs.add(ValidationError::new("a", "First"));
errs.add(ValidationError::new("b", "Second"));

errs.has_errors()   // bool
errs.len()          // usize
errs.errors()       // &[ValidationError]

// Convert to a single wrapped error
let combined = errs.into_single_error("Validation failed");

// Convert to Result
let result: Result<T, ValidationErrors> = errs.into_result(ok_value);
```

`ValidationErrors` implements `IntoIterator`, `FromIterator<ValidationError>`,
`Display`, and `std::error::Error`.
