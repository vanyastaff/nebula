# Macros

Three macros cover the most common custom-validator and composition patterns.

---

## `validator!`

Generates a complete validator: struct definition, `Validate<T>` implementation,
constructor, and optional factory function.

`#[derive(Debug, Clone)]` is always applied. Add extra derives via `#[derive(...)]`.

### Variants

**Unit validator** (zero-sized, `Copy`):

```rust
validator! {
    pub NotEmpty for str;
    rule(input) { !input.is_empty() }
    error(input) { ValidationError::new("not_empty", "must not be empty") }
    fn not_empty();
}

let v = not_empty(); // const fn
not_empty().validate("hello"); // Ok
```

**Struct with fields** (auto-generated `new` from fields):

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

let v = min_length(5);
let v = MinLength::new(5);
```

**Custom constructor** (overrides auto `new`):

```rust
validator! {
    pub LengthRange { min: usize, max: usize } for str;
    rule(self, input) { let l = input.len(); l >= self.min && l <= self.max }
    error(self, input) { ValidationError::new("range", "out of range") }
    new(min: usize, max: usize) { Self { min, max } }
    fn length_range(min: usize, max: usize);
}
```

**Fallible constructor** (returns `Result`):

```rust
validator! {
    pub Range { lo: usize, hi: usize } for usize;
    rule(self, input) { *input >= self.lo && *input <= self.hi }
    error(self, input) { ValidationError::new("range", "out of range") }
    new(lo: usize, hi: usize) -> ValidationError {
        if lo > hi {
            return Err(ValidationError::new("invalid", "lo must be <= hi"));
        }
        Ok(Self { lo, hi })
    }
    fn range(lo: usize, hi: usize) -> ValidationError;
}

let v = range(1, 10)?;           // returns Result
let v = Range::new(1, 10)?;
```

**Generic validator**:

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

let v = min(18_u32);
let v = min(3.14_f64);
```

**Phantom generic** (generic type parameter, no field):

```rust
validator! {
    pub Required<T> for Option<T>;
    rule(input) { input.is_some() }
    error(input) { ValidationError::new("required", "required") }
    fn required();
}

let v = required::<String>();
```

### Macro Architecture

The macro has three layers internally:
1. **Entry points** (5 arms) — parse user syntax variants
2. **Tail parser** (5 arms) — detect `new` / `fn factory` presence
3. **Code generators** (@helpers) — each handles exactly one concern with no duplication

---

## `compose!`

AND-chains multiple validators. Equivalent to repeated `.and()`.

```rust
let v = compose![min_length(5), max_length(20), alphanumeric()];
// same as: min_length(5).and(max_length(20)).and(alphanumeric())
```

Single-element `compose![v]` returns `v` unchanged.

---

## `any_of!`

OR-chains multiple validators. Equivalent to repeated `.or()`.

```rust
let v = any_of![exact_length(5), exact_length(10)];
// same as: exact_length(5).or(exact_length(10))
```

Single-element `any_of![v]` returns `v` unchanged.
