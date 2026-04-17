//! Proc-macro crate for [`nebula-validator`](https://crates.io/crates/nebula-validator).
//!
//! This crate is a private implementation detail — users depend on
//! `nebula-validator` with the `derive` feature (enabled by default) and
//! import the [`Validator`] derive through the top-level re-export:
//!
//! ```rust,ignore
//! use nebula_validator::Validator;
//!
//! #[derive(Validator)]
//! struct User {
//!     #[validate(min_length = 3, max_length = 32)]
//!     name: String,
//!     #[validate(email)]
//!     email: String,
//!     #[validate(required, range(min = 18, max = 120))]
//!     age: Option<u8>,
//! }
//! ```
//!
//! # Container attributes (`#[validator(...)]`)
//!
//! | Key | Purpose | Example |
//! |---|---|---|
//! | `message = "..."` | Root error message when collapsing multiple field errors | `#[validator(message = "user validation failed")]` |
//!
//! # Field attributes (`#[validate(...)]`)
//!
//! Common rules (full catalogue in the generated diagnostics):
//!
//! | Attribute | Applies to | Description |
//! |---|---|---|
//! | `required` | `Option<T>` | Field must be `Some` |
//! | `min_length = N` / `max_length = N` / `exact_length = N` | `String` | Character-count bounds |
//! | `length_range(min = N, max = M)` | `String` | Inclusive length range |
//! | `min = E` / `max = E` / `range(min = A, max = B)` | numeric | Numeric bounds |
//! | `min_size = N` / `max_size = N` / `exact_size = N` / `size_range(...)` / `not_empty_collection` | `Vec<T>` | Collection-size bounds |
//! | `email`, `url`, `uuid`, `ipv4`, `ipv6`, `hostname`, `date`, `date_time`, `time` | `String` | Built-in format checks |
//! | `alphanumeric`, `alphabetic`, `numeric`, `lowercase`, `uppercase`, `not_empty` | `String` | Character-class checks |
//! | `contains = "..."` / `starts_with = "..."` / `ends_with = "..."` | `String` | Substring checks |
//! | `regex = "..."` | `String` | Pattern match — compiled once via `LazyLock` and **validated at macro-time** |
//! | `is_true` / `is_false` | `bool` | Boolean checks |
//! | `nested` | any `SelfValidating` type | Delegates to inner validator |
//! | `custom = path_or_closure` | any | `Fn(&T) -> Result<(), ValidationError>` |
//! | `using = expr` | any | Any `impl Validate<T>` |
//! | `all(expr, ...)` / `any(expr, ...)` | any | Compose existing validators |
//! | `each(...)` / `inner(...)` | `Vec<T>` | Apply any of the above to every element |
//! | `message = "..."` | any | Override error message for this field's failures |
//!
//! # Architecture
//!
//! Three phases, each in its own module:
//!
//! 1. `parse` — `syn::DeriveInput` → `model::ValidatorInput` IR. Validates attribute combinations
//!    and regex patterns at macro-time.
//! 2. `model` — pure intermediate representation, zero `syn` types.
//! 3. `emit` — `model::ValidatorInput` → `proc_macro2::TokenStream`. Pre-compiles regex patterns
//!    via `LazyLock` for runtime performance.
//!
//! Shared `types` introspection helpers (`Option<T>`, `Vec<T>`, `String`,
//! `bool` recognition) are used by both parse and emit.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod emit;
mod model;
mod parse;
mod types;

/// Derive macro generating field-level validation for a named-field struct.
///
/// Emits:
/// - `impl Validate<Self> for Self` — forwards to `validate_fields()` and collapses accumulated
///   errors into a single `ValidationError`.
/// - `impl SelfValidating for Self` — enables `#[validate(nested)]` in parents.
/// - An inherent `validate_fields(&self) -> Result<(), ValidationErrors>` that returns the full
///   collection of field-level failures.
///
/// Container attributes go on `#[validator(...)]`; field attributes go on
/// `#[validate(...)]`. See the [crate-level docs](crate) for the complete
/// attribute catalogue.
///
/// Invalid attribute combinations, type mismatches, and bad regex patterns
/// are reported as compile errors with spans pointing at the offending
/// field. See the `tests/ui/` directory in `nebula-validator` for examples.
#[proc_macro_derive(Validator, attributes(validator, validate))]
pub fn derive_validator(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => nebula_macro_support::diag::to_compile_error(e).into(),
    }
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let ir = parse::parse(&input)?;
    Ok(emit::emit(&ir))
}
