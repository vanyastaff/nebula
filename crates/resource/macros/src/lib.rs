//! # nebula-resource-macros
//!
//! Proc-macros for the `nebula-resource` crate (slot model).
//!
//! ## Two-derive pattern
//!
//! Resource authors use two derives:
//!
//! 1. **`#[derive(Resource)]`** — emits slot plumbing only:
//!    - `impl DeclaresDependencies` enumerating `#[credential]` slot fields.
//!    - An inherent `pub fn <field>_slot(&self) -> Option<Arc<...>>` accessor per slot.
//!    - `impl HasCredentialSlots` with the order-sensitive positional epoch fold.
//!
//! 2. **Hand-written `impl Provider`** — the implementor supplies `key()`, the two
//!    associated types (`Config`, `Instance`), and the lifecycle methods (`create`,
//!    optionally `check`, `shutdown`, `destroy`, credential-rotation hooks).
//!
//! ```ignore
//! use nebula_credential::CredentialGuard;
//! use nebula_resource::{Provider, Resource, SlotCell};
//!
//! #[derive(Resource)]
//! struct Postgres {
//!     #[credential(key = "db_auth", purpose = "Main DB auth")]
//!     db_auth: SlotCell<CredentialGuard<DatabaseCredential>>,
//! }
//!
//! impl Provider for Postgres {
//!     type Config = PostgresConfig;
//!     type Instance = PgConnection;
//!
//!     fn key() -> nebula_core::ResourceKey { resource_key!("postgres") }
//!
//!     async fn create(&self, cfg: &PostgresConfig, ctx: &ResourceContext)
//!         -> Result<PgConnection, nebula_resource::Error>
//!     {
//!         let guard = self.db_auth_slot().ok_or_else(|| Error::transient("slot unbound"))?;
//!         // ... connect using guard ...
//!     }
//! }
//! ```
//!
//! The `#[resource(...)]` container attribute is not accepted by `Resource` — it
//! existed only on an older retired derive which emitted a `todo!()` body.
//!
//! ## `#[derive(ClassifyError)]`
//!
//! Generates `From<UserError> for nebula_resource::Error` based on
//! `#[classify(...)]` attributes on enum variants. Independent of the
//! `Resource` derive — used by resource implementors to bridge their domain
//! errors into the framework's `Error` type.
//!
//! Source-error chains are preserved: the original error is attached via
//! `Error::with_source(err)` so `std::error::Error::source()` returns it.
//!
//! ```ignore
//! #[derive(thiserror::Error, Debug, ClassifyError)]
//! enum DbError {
//!     #[error("connect timeout")]
//!     #[classify(transient)]
//!     ConnectTimeout,
//!
//!     #[error("rate-limited")]
//!     #[classify(exhausted, retry_after = "30s")]
//!     RateLimited,
//!
//!     // retry_after from a Duration field (tuple field `.0`):
//!     #[error("throttled")]
//!     #[classify(exhausted, retry_after = .0)]
//!     Throttled(std::time::Duration),
//!
//!     // retry_after from a named Duration field:
//!     #[error("quota exceeded")]
//!     #[classify(exhausted, retry_after = wait)]
//!     QuotaExceeded { wait: std::time::Duration },
//! }
//! ```
//!
//! ### Not classifiable here
//!
//! `NotFound`, `Ambiguous`, and `Revoked` `ErrorKind`
//! variants are framework-origin errors (produced by the engine lookup path, not by
//! resource implementations). `ClassifyError` cannot emit these — use the resource
//! `Error` constructors directly on the rare occasion your `create()` must signal one.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, parse_macro_input};

mod config;
mod field_slots;
mod slots;

/// Derive macro that emits slot plumbing for a resource struct (slot model).
///
/// ## What is emitted
///
/// - `impl DeclaresDependencies` enumerating each `#[credential]` slot field.
/// - An inherent `pub fn <field>_slot(&self) -> Option<Arc<CredentialGuard<C>>>` accessor per slot.
/// - `impl HasCredentialSlots` with the order-sensitive positional epoch fold.
///
/// ## What is NOT emitted
///
/// This derive does **not** emit any `Provider` impl, `todo!()`, `key()`,
/// `metadata()`, or topology constant. The implementor writes `impl Provider`
/// by hand (2 associated types + `create`).
///
/// ## Field attributes
///
/// `#[credential]` / `#[credential(key = "...", purpose = "...")]`
///
/// - `key = "..."` — slot key override (defaults to field name). Validated
///   at expansion time against `CredentialKey` rules.
/// - `purpose = "..."` — human-readable description (catalog / UI).
///
/// Field types accepted (path-tail matching, bare or fully qualified):
/// - `SlotCell<CredentialGuard<C>>`
/// - `CredentialSlot<C>` (type alias for the above)
///
/// ## Slot-less structs
///
/// Deriving on a struct with no `#[credential]` fields is legal and emits
/// empty `DeclaresDependencies` + `HasCredentialSlots { epoch → 0 }`.
///
/// ## Rejected forms
///
/// - Enums and unions: compile error at the type ident.
/// - Tuple structs with a `#[credential]` field: compile error.
/// - Wrong field type: compile error naming both accepted shapes.
/// - `#[resource(...)]` container attribute: compile error (removed).
#[proc_macro_derive(Resource, attributes(credential))]
pub fn derive_resource(input: TokenStream) -> TokenStream {
    slots::derive(input)
}

/// Derive macro that generates `impl ResourceConfig` with a deterministic structural
/// fingerprint and an optional default empty `impl HasSchema`.
///
/// ## What is emitted
///
/// - `impl nebula_resource::ResourceConfig` with:
///   - `fn fingerprint(&self) -> u64` — structural hash over all fields that implement
///     [`std::hash::Hash`]. Fields tagged `#[config(skip_fingerprint)]` are excluded.
///   - `fn validate(&self) -> Result<(), Error>` — only emitted if
///     `#[config(validate = path)]` is specified; otherwise the trait default (`Ok(())`)
///     applies.
/// - `impl nebula_schema::HasSchema` returning an empty schema — suppressed when
///   `#[config(schema = external)]` is present (use alongside `#[derive(Schema)]`).
///
/// ## Container attribute (`#[config(...)]`)
///
/// | Key | Effect |
/// |-----|--------|
/// | `validate = path` | Delegates `validate(&self)` to `path(self)`. |
/// | `schema = external` | Suppresses the default `HasSchema` emission. |
///
/// ## Field attribute (`#[config(skip_fingerprint)]`)
///
/// Excludes the annotated field from the fingerprint hash fold. The field type is not
/// required to implement [`std::hash::Hash`] when skipped.
///
/// ## Example
///
/// ```ignore
/// #[derive(ResourceConfig, serde::Deserialize, Clone)]
/// struct PgConfig {
///     url: String,
///     max_conns: u32,
///     /// Excluded from change-detection — label changes never trigger hot-reload.
///     #[config(skip_fingerprint)]
///     label: String,
/// }
/// ```
#[proc_macro_derive(ResourceConfig, attributes(config))]
pub fn derive_resource_config(input: TokenStream) -> TokenStream {
    config::derive(input)
}

/// Derive macro that generates `From<T> for nebula_resource::Error`.
///
/// Place `#[classify(kind)]` on each enum variant to specify how the
/// framework should handle errors of that variant. Source-error chains are
/// preserved: the original error is attached via `Error::with_source(err)`.
///
/// # Supported kinds
///
/// - `transient` — retry with backoff
/// - `permanent` — never retry
/// - `exhausted` — retry after cooldown (optionally with `retry_after`)
/// - `backpressure` — caller decides
/// - `cancelled` — operation was cancelled
///
/// # `retry_after` forms (only valid with `exhausted`)
///
/// - Literal string: `retry_after = "30s"` (parsed at expansion time)
/// - Tuple-field index: `retry_after = .0` (reads `Duration` at runtime)
/// - Named field: `retry_after = field_name` (reads `Duration` at runtime)
///
/// # Not classifiable here
///
/// `NotFound`, `Ambiguous`, and `Revoked` are framework-origin
/// `ErrorKind` variants. `ClassifyError` cannot
/// emit them — use the `Error` constructors directly on the rare occasion
/// your `create()` must signal one.
///
/// # Errors
///
/// Compile-time errors are emitted when:
/// - The macro is applied to a non-enum type
/// - A variant is missing the `#[classify(...)]` attribute
/// - An unknown classification kind is used
/// - The `retry_after` duration string cannot be parsed
#[proc_macro_derive(ClassifyError, attributes(classify))]
pub fn derive_classify_error(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match classify_error_impl(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Parsed classification for a single variant.
struct Classification {
    kind: ClassifyKind,
}

enum ClassifyKind {
    Transient,
    Permanent,
    Exhausted { retry_after: ExhaustedRetryAfter },
    Backpressure,
    Cancelled,
}

/// How the `retry_after` duration is supplied for an `exhausted` variant.
enum ExhaustedRetryAfter {
    /// No `retry_after` specified — `None` is passed to `Error::exhausted`.
    None,
    /// Static literal parsed at expansion time.
    Static(std::time::Duration),
    /// Runtime field: tuple index (e.g. `.0`).
    TupleIndex(syn::Index),
    /// Runtime field: named field ident.
    NamedField(Ident),
}

fn classify_error_impl(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let enum_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let data = match &input.data {
        Data::Enum(data) => data,
        _ => {
            return Err(syn::Error::new_spanned(
                enum_name,
                "ClassifyError can only be derived for enums",
            ));
        },
    };

    let mut match_arms = Vec::new();

    for variant in &data.variants {
        let variant_name = &variant.ident;
        let classification = parse_classify_attr(variant)?;
        let arm = build_arm(enum_name, variant_name, &variant.fields, &classification);
        match_arms.push(arm);
    }

    // Classify by **reference** to `err` (so a `retry_after` field can be read
    // as a `&Duration` without moving the variant), build the `Error`, end the
    // borrow, and only then move `err` into `with_source` to preserve the full
    // source chain. Binding the whole value with `@` while a sub-field is
    // borrowed by `ref` is E0505 (borrow of a moved value); reading first and
    // moving last avoids it. `EnumType: std::error::Error + Send + Sync +
    // 'static` always holds for `thiserror`-derived enums, the target use case.
    Ok(quote! {
        impl #impl_generics ::core::convert::From<#enum_name #ty_generics> for nebula_resource::Error
        #where_clause
        {
            fn from(err: #enum_name #ty_generics) -> Self {
                let __msg = ::std::string::ToString::to_string(&err);
                let __built: nebula_resource::Error = match &err {
                    #(#match_arms)*
                };
                __built.with_source(err)
            }
        }
    })
}

/// Build a complete match arm: `&`-pattern ⇒ Error constructor.
///
/// Arms match `&err`, so any bound field (e.g. a runtime `retry_after`
/// `Duration`) is a shared reference and is copied out, never moved. The
/// `with_source(err)` move happens once after the match, not per arm.
fn build_arm(
    enum_name: &Ident,
    variant_name: &Ident,
    fields: &Fields,
    classification: &Classification,
) -> TokenStream2 {
    let pattern = build_arm_pattern(enum_name, variant_name, fields, classification);
    let constructor = build_arm_constructor(classification);

    quote! {
        #pattern => #constructor,
    }
}

/// Build the pattern for one match arm over `&err`.
///
/// For `exhausted` variants with a runtime `retry_after`, the relevant field is
/// bound as `__retry_after` (a `&Duration` under match-ergonomics); all other
/// fields are ignored. No arm moves out of `err`.
fn build_arm_pattern(
    enum_name: &Ident,
    variant_name: &Ident,
    fields: &Fields,
    classification: &Classification,
) -> TokenStream2 {
    match &classification.kind {
        ClassifyKind::Exhausted {
            retry_after: ExhaustedRetryAfter::TupleIndex(idx),
        } => {
            let pos = idx.index as usize;
            let count = match fields {
                Fields::Unnamed(f) => f.unnamed.len(),
                _ => 0,
            };
            let pats: Vec<TokenStream2> = (0..count)
                .map(|i| {
                    if i == pos {
                        quote! { __retry_after }
                    } else {
                        quote! { _ }
                    }
                })
                .collect();
            quote! { #enum_name::#variant_name(#(#pats),*) }
        },
        ClassifyKind::Exhausted {
            retry_after: ExhaustedRetryAfter::NamedField(field_ident),
        } => {
            quote! {
                #enum_name::#variant_name { #field_ident: __retry_after, .. }
            }
        },
        _ => match fields {
            Fields::Unit => quote! { #enum_name::#variant_name },
            Fields::Unnamed(_) => quote! { #enum_name::#variant_name(..) },
            Fields::Named(_) => quote! { #enum_name::#variant_name { .. } },
        },
    }
}

/// Build the `nebula_resource::Error::*` constructor (without `with_source`).
///
/// `__msg` and `__retry_after` are expected to be in scope when the emitted
/// code runs (bound by `build_arm`).
fn build_arm_constructor(classification: &Classification) -> TokenStream2 {
    match &classification.kind {
        ClassifyKind::Transient => quote! { nebula_resource::Error::transient(__msg) },
        ClassifyKind::Permanent => quote! { nebula_resource::Error::permanent(__msg) },
        ClassifyKind::Exhausted { retry_after } => build_exhausted_constructor(retry_after),
        ClassifyKind::Backpressure => quote! { nebula_resource::Error::backpressure(__msg) },
        ClassifyKind::Cancelled => quote! { nebula_resource::Error::cancelled() },
    }
}

fn build_exhausted_constructor(retry_after: &ExhaustedRetryAfter) -> TokenStream2 {
    match retry_after {
        ExhaustedRetryAfter::None => {
            quote! {
                nebula_resource::Error::exhausted(__msg, ::core::option::Option::None)
            }
        },
        ExhaustedRetryAfter::Static(dur) => {
            let secs = dur.as_secs();
            let nanos = dur.subsec_nanos();
            quote! {
                nebula_resource::Error::exhausted(
                    __msg,
                    ::core::option::Option::Some(
                        ::core::time::Duration::new(#secs, #nanos)
                    ),
                )
            }
        },
        ExhaustedRetryAfter::TupleIndex(_) | ExhaustedRetryAfter::NamedField(_) => {
            // `__retry_after: &Duration` was bound by ref in the pattern arm.
            quote! {
                nebula_resource::Error::exhausted(
                    __msg,
                    ::core::option::Option::Some(*__retry_after),
                )
            }
        },
    }
}

/// Parse the `#[classify(...)]` attribute from a variant.
fn parse_classify_attr(variant: &syn::Variant) -> syn::Result<Classification> {
    let mut found = None;

    for attr in &variant.attrs {
        if !attr.path().is_ident("classify") {
            continue;
        }

        if found.is_some() {
            return Err(syn::Error::new_spanned(
                attr,
                "duplicate #[classify(...)] attribute",
            ));
        }

        found = Some(parse_classify_meta(attr)?);
    }

    found.ok_or_else(|| {
        syn::Error::new_spanned(
            &variant.ident,
            format!(
                "variant `{}` is missing a #[classify(...)] attribute",
                variant.ident
            ),
        )
    })
}

/// Parse the inner contents of `#[classify(...)]`.
fn parse_classify_meta(attr: &syn::Attribute) -> syn::Result<Classification> {
    let mut kind_ident: Option<Ident> = None;
    let mut retry_after: Option<ExhaustedRetryAfter> = None;

    attr.parse_nested_meta(|meta| {
        let ident = meta
            .path
            .get_ident()
            .ok_or_else(|| syn::Error::new_spanned(&meta.path, "expected an identifier"))?;

        let name = ident.to_string();
        match name.as_str() {
            "transient" | "permanent" | "exhausted" | "backpressure" | "cancelled" => {
                if kind_ident.is_some() {
                    return Err(syn::Error::new_spanned(
                        ident,
                        "multiple classification kinds specified",
                    ));
                }
                kind_ident = Some(ident.clone());
                Ok(())
            },
            "retry_after" => {
                // Three forms:
                //   retry_after = "30s"    — static literal string
                //   retry_after = .0       — tuple field index (Duration)
                //   retry_after = name     — named field ident (Duration)
                let value = meta.value()?;

                // Try static string literal first.
                if value.peek(syn::LitStr) {
                    let lit: syn::LitStr = value.parse()?;
                    let dur = parse_duration(&lit.value())
                        .map_err(|msg| syn::Error::new_spanned(&lit, msg))?;
                    retry_after = Some(ExhaustedRetryAfter::Static(dur));
                    return Ok(());
                }

                // Try `.N` (tuple field index): the stream starts with `.`.
                use syn::Token;
                if value.peek(Token![.]) {
                    value.parse::<Token![.]>()?;
                    let idx: syn::Index = value.parse()?;
                    retry_after = Some(ExhaustedRetryAfter::TupleIndex(idx));
                    return Ok(());
                }

                // Remaining: plain ident for a named field.
                let field_ident: Ident = value.parse()?;
                retry_after = Some(ExhaustedRetryAfter::NamedField(field_ident));
                Ok(())
            },
            _ => Err(syn::Error::new_spanned(
                ident,
                format!("unknown classify attribute `{name}`"),
            )),
        }
    })?;

    let kind_ident = kind_ident.ok_or_else(|| {
        syn::Error::new_spanned(
            attr,
            "missing classification kind (transient, permanent, exhausted, backpressure, cancelled)",
        )
    })?;

    if retry_after.is_some() && kind_ident != "exhausted" {
        return Err(syn::Error::new_spanned(
            &kind_ident,
            "retry_after is only valid with `exhausted`",
        ));
    }

    let kind = match kind_ident.to_string().as_str() {
        "transient" => ClassifyKind::Transient,
        "permanent" => ClassifyKind::Permanent,
        "exhausted" => ClassifyKind::Exhausted {
            retry_after: retry_after.unwrap_or(ExhaustedRetryAfter::None),
        },
        "backpressure" => ClassifyKind::Backpressure,
        "cancelled" => ClassifyKind::Cancelled,
        _ => unreachable!("kind_ident was validated against the match above"),
    };

    Ok(Classification { kind })
}

/// Parse a human-readable duration string like `"30s"`, `"5m"`, `"1h"`.
///
/// Supported suffixes: `s` (seconds), `m` (minutes), `h` (hours), `ms` (milliseconds).
/// Plain numbers without a suffix are treated as seconds.
fn parse_duration(s: &str) -> Result<std::time::Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration string".to_string());
    }

    if let Some(val) = s.strip_suffix("ms") {
        let n: u64 = val
            .trim()
            .parse()
            .map_err(|_| format!("invalid millisecond value: `{val}`"))?;
        return Ok(std::time::Duration::from_millis(n));
    }

    if let Some(val) = s.strip_suffix('s') {
        let n: u64 = val
            .trim()
            .parse()
            .map_err(|_| format!("invalid second value: `{val}`"))?;
        return Ok(std::time::Duration::from_secs(n));
    }

    if let Some(val) = s.strip_suffix('m') {
        let n: u64 = val
            .trim()
            .parse()
            .map_err(|_| format!("invalid minute value: `{val}`"))?;
        return Ok(std::time::Duration::from_secs(n * 60));
    }

    if let Some(val) = s.strip_suffix('h') {
        let n: u64 = val
            .trim()
            .parse()
            .map_err(|_| format!("invalid hour value: `{val}`"))?;
        return Ok(std::time::Duration::from_secs(n * 3600));
    }

    // Bare number = seconds.
    let n: u64 = s
        .parse()
        .map_err(|_| format!("invalid duration: `{s}` (use a suffix: s, m, h, ms)"))?;
    Ok(std::time::Duration::from_secs(n))
}
