# Built-in Validators

All factory functions are in `nebula_validator::prelude::*`. Each factory returns
a typed struct that implements `Validate<T>` for the appropriate input types.

---

## String / Length (`validators::length`)

Accept `T: AsRef<str> + ?Sized` — works with `&str`, `String`, `Cow<str>`.

| Factory | Struct | Check |
|---|---|---|
| `min_length(n)` | `MinLength` | `len >= n` (chars) |
| `max_length(n)` | `MaxLength` | `len <= n` (chars) |
| `not_empty()` | `NotEmpty` | `!input.is_empty()` |
| `exact_length(n)` | `ExactLength` | `len == n` |
| `length_range(min, max)` | `LengthRange` | `min <= len <= max` |
| `min_length_bytes(n)` | — | `byte_len >= n` |
| `max_length_bytes(n)` | — | `byte_len <= n` |
| `exact_length_bytes(n)` | — | `byte_len == n` |
| `length_range_bytes(min, max)` | — | `min <= byte_len <= max` |

---

## String / Pattern (`validators::pattern`)

| Factory | Check |
|---|---|
| `alphanumeric()` | all chars are `[a-zA-Z0-9]` |
| `alphabetic()` | all chars are `[a-zA-Z]` |
| `numeric()` | all chars are `[0-9]` |
| `uppercase()` | all chars are uppercase |
| `lowercase()` | all chars are lowercase |
| `contains(sub)` | input contains the substring |
| `starts_with(prefix)` | input starts with prefix |
| `ends_with(suffix)` | input ends with suffix |

---

## String / Content (`validators::content`)

| Factory | Check |
|---|---|
| `email()` | valid email address format |
| `url()` | valid URL (http/https) |
| `matches_regex(pattern)` | matches the given regex string |

---

## Numeric / Range (`validators::range`)

Accept any `T: PartialOrd + Display + Copy`.

| Factory | Check |
|---|---|
| `min(value)` | `input >= value` |
| `max(value)` | `input <= value` |
| `in_range(lo, hi)` | `lo <= input <= hi` |
| `greater_than(value)` | `input > value` |
| `less_than(value)` | `input < value` |
| `exclusive_range(lo, hi)` | `lo < input < hi` |

---

## Collection / Size (`validators::size`)

Accept `T: AsRef<[E]>` — works with `Vec<E>`, `&[E]`, arrays.

| Factory | Check |
|---|---|
| `min_size(n)` | `len >= n` |
| `max_size(n)` | `len <= n` |
| `exact_size(n)` | `len == n` |
| `size_range(min, max)` | `min <= len <= max` |
| `not_empty_collection()` | `!is_empty()` |

---

## Boolean (`validators::boolean`)

Accept `bool`.

| Factory | Check |
|---|---|
| `is_true()` | `*input == true` |
| `is_false()` | `*input == false` |

---

## Nullable (`validators::nullable`)

| Factory / Struct | Input | Check |
|---|---|---|
| `required::<T>()` | `Option<T>` | `input.is_some()` |
| `not_null::<T>()` | `Option<T>` | `input.is_some()` (alias) |

---

## Network (`validators::network`)

| Factory | Check |
|---|---|
| `ip_addr()` | valid IPv4 or IPv6 address |
| `ipv4()` | valid IPv4 address |
| `ipv6()` | valid IPv6 address |
| `hostname()` | valid hostname |

---

## Temporal (`validators::temporal`)

| Factory | Check |
|---|---|
| `date()` | valid ISO 8601 date (YYYY-MM-DD) |
| `time()` | valid ISO 8601 time |
| `date_time()` | valid ISO 8601 datetime |
| `uuid()` | valid UUID string |

---

## Composition Example

```rust
use nebula_validator::prelude::*;

// Username: 3-20 chars, alphanumeric only
let username = min_length(3).and(max_length(20)).and(alphanumeric());

// Age: 18-120
let age = in_range(18_u32, 120);

// Optional email
let email_opt = required::<String>().not().or(email());

// Tags list: 1-10 items, each 1-50 chars
let tags = min_size(1).and(max_size(10));
```
