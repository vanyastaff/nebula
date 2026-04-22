# 3. Idioms

[← Language Rules](./02-language-rules.md) | [README](./README.md) | [Next: Design Patterns →](./04-design-patterns.md)

---

## [I-001] Borrow the target type, not the wrapper

Parameters: `&str`, `&[T]`, `&T` — not `&String`, `&Vec<T>`, `&Box<T>`. The wrapper form loses `&'static str` literals and split slices.

```rust
// Bad
fn count_vowels(s: &String) -> usize { /* ... */ }

// Good
fn count_vowels(s: &str) -> usize { /* ... */ }
```

Exception: when the function genuinely requires the owned type's API (`String::capacity`, `Vec::reserve`), take `&mut String`/`&mut Vec<T>` and document why.

## [I-002] `format!` for mixed literal/non-literal strings

```rust
// Good — captured identifiers (Edition 2021+)
let msg = format!("user {name} logged in at {time}");

// Acceptable when constructing in a hot loop with known size
let mut s = String::with_capacity(expected);
s.push_str("user ");
s.push_str(name);
```

Use `format!` by default; hand-roll only when benchmarks show allocation pressure.

## [I-003] `new` + `Default` as a pair

If `new()` takes no arguments, also implement `Default`. If the struct is `#[derive(Default)]`-able, derive it, then define `new()` as `Self::default()`.

```rust
// Good
#[derive(Default)]
pub struct Settings { /* ... */ }
impl Settings {
    pub fn new() -> Self { Self::default() }
    pub fn with_limit(mut self, limit: usize) -> Self { self.limit = limit; self }
}
```

Return `Self`, not the concrete type name.

## [I-004] Smart-pointer-style owning collections

If your collection owns data and offers a borrowed view, expose the view via an explicit method (`.as_slice()`, `.as_view()`) rather than `Deref` coercion unless the type is *genuinely* a pointer wrapper. See `[A-003]`.

## [I-005] RAII guards for finalization

```rust
struct LogOnExit(&'static str);
impl Drop for LogOnExit {
    fn drop(&mut self) { tracing::info!("exit: {}", self.0); }
}

fn work() -> Result<()> {
    let _g = LogOnExit("work");   // named with _g, NOT `_` (which drops immediately)
    step1()?;
    step2()?;
    Ok(())
}
```

Rules: name guards `_g`, `_guard`; never use the bare `_` name (drops at statement end); never wrap a guard in `Rc`/`Arc` (lifetime would escape).

## [I-006] `mem::take` / `mem::replace` for enum transitions

```rust
enum State { A { name: String, x: u8 }, B { name: String } }

fn a_to_b(s: &mut State) {
    if let State::A { name, x: 0 } = s {
        *s = State::B { name: std::mem::take(name) };
    }
}
```

For `Option`: prefer `.take()` over `mem::take(opt)`. If the type is not `Default`, use `mem::replace(field, placeholder)`.

## [I-007] On-stack dynamic dispatch (Rust 1.79+)

Already covered in `[L-DROP-005]`. Prefer over `Box<dyn Trait>` when the trait object does not outlive the function:

```rust
// Good
let r: &mut dyn Read = if from_stdin { &mut io::stdin() } else { &mut File::open(p)? };
```

## [I-008] FFI — error idioms

```rust
// Flat enum as C return code
#[repr(i32)]
pub enum DbError { Ok = 0, Readonly = 1, IoError = 2 }

// Structured enum: return code + out-parameter string
#[no_mangle]
pub unsafe extern "C" fn db_error_message(e: DbError, out: *mut *mut c_char) -> c_int { /* … */ }
```

Never return owned strings by value from Rust to C; always transfer via explicit allocator (the caller's `malloc`/`free`).

## [I-009] FFI — accepting C strings

```rust
/// # Safety
/// `msg` must be non-null, NUL-terminated, valid for the call duration, and immutable.
#[no_mangle]
pub unsafe extern "C" fn log_msg(msg: *const c_char, level: c_int) {
    let Ok(s) = unsafe { CStr::from_ptr(msg) }.to_str() else { return; };
    crate::log(s, level.into());
}
```

Minimize `unsafe`; do not copy strings manually with `strlen` + `copy_nonoverlapping` — a classic UB source.

## [I-010] FFI — passing strings to C

```rust
fn notify(msg: &str) -> Result<(), ffi::NulError> {
    let cstr = CString::new(msg)?;
    // SAFETY: `seterr` docs say the pointer is read-only and not retained
    unsafe { seterr(cstr.as_ptr()); }
    Ok(())
    // cstr lives until here
}
```

The CString MUST be bound to a named local; `seterr(CString::new(msg)?.as_ptr())` is a dangling pointer immediately.

## [I-011] Iterate over `Option` via `IntoIterator`

```rust
// Good
targets.extend(optional_last);
for t in prefix.iter().chain(optional_suffix.iter()) { /* … */ }
items.iter().filter_map(|i| i.maybe_value())

// Do NOT write: for x in opt { ... } — use `if let Some(x) = opt`
```

## [I-012] Scoped rebinding for closure captures

```rust
let closure = {
    let a = a.clone();
    let b = b.as_ref();
    move || combine(a, b)
};
```

Group capture transforms next to the closure definition.

## [I-013] `#[non_exhaustive]` vs private marker field

- Cross-crate evolution of public enums: `#[non_exhaustive]`.
- Within-crate discipline: private marker field (`_priv: ()`).
- Structs with public fields: `#[non_exhaustive]` if the struct might grow; otherwise prefer a builder.

## [I-014] Easy doctest initialization

Wrap setup boilerplate in a hidden helper function; doctest compiles but doesn't run:

```rust
/// ```
/// # fn call_example(conn: Connection, req: Request) {
/// let resp = conn.send(req).unwrap();
/// # }
/// ```
```

## [I-015] Temporary mutability via shadowing

```rust
// Good
let data = {
    let mut data = load();
    data.sort_unstable();
    data
};
// Or:
let mut data = load();
data.sort_unstable();
let data = data; // rebinding as immutable
```

## [I-016] Return consumed argument on error

```rust
pub struct SendError<T>(pub T);
pub fn send<T>(value: T) -> Result<(), SendError<T>> {
    if try_send(&value) { Ok(()) } else { Err(SendError(value)) }
}
```

Mirrors `String::from_utf8` → `FromUtf8Error::into_bytes`. Caller can retry without cloning.

---

[← Language Rules](./02-language-rules.md) | [README](./README.md) | [Next: Design Patterns →](./04-design-patterns.md)
