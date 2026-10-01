//! `#[derive(ResourceConfig)]` macro implementation.
//!
//! Emits:
//! - `impl nebula_resource::ResourceConfig for T` with:
//!   - `fn fingerprint(&self) -> u64` — a stable fingerprint, built by
//!     `nebula_resource::ConfigFingerprint`, of every field that does NOT carry
//!     `#[config(skip_fingerprint)]`: SHA-256 over the canonical JSON of those fields
//!     keyed by name. Each included field must implement `serde::Serialize`. The value
//!     is the same in every build — the effect journal records it — which
//!     `std::hash::Hash` cannot promise. Fieldless configs return `0`.
//!   - `fn validate(&self) -> Result<(), nebula_resource::Error>` — when any field is
//!     fingerprinted, first refuses a config whose fields have no stable fingerprint
//!     (`ConfigFingerprint::try_finish`); then, if `#[config(validate = path)]` is
//!     present, delegates to `path(self)`. A fieldless config without `validate`
//!     inherits the trait default `Ok(())`.
//! - Optionally `impl nebula_schema::HasSchema for T` returning a null schema for
//!   unit structs and an empty-record schema for empty-braced structs,
//!   UNLESS `#[config(schema = external)]` is specified, in which case no `HasSchema`
//!   impl is emitted (the caller is responsible for `#[derive(Schema)]` or a manual
//!   `impl HasSchema`). All other structs require that explicit schema source.
//!
//! ## Container attribute (`#[config(...)]`)
//!
//! Supported keys:
//! - `validate = path` — calls `path(self)` in the emitted `validate` method.
//! - `schema = external` — uses the caller's `HasSchema` implementation.
//!
//! Unknown keys are rejected with a `compile_error!` at the key span.
//!
//! ## Field attribute (`#[config(...)]`)
//!
//! Supported keys:
//! - `skip_fingerprint` — this field is excluded from the fingerprint.
//!
//! Unknown field-level keys are rejected with a `compile_error!` at the key span.
//!
//! ## Rejected forms
//!
//! - Enums and unions: compile error at the type ident.
//! - Tuple struct fields with `#[config]`: compile error.
//! - Field included in fingerprint that does not impl `serde::Serialize` — compile
//!   error from the emitted `ConfigFingerprint::field(name, &self.field)` call.
//! - Field included in fingerprint whose type names a hash-ordered set (`HashSet`,
//!   `hashbrown::HashSet`, `FxHashSet`, … anywhere in the type) — compile error at
//!   the segment: its array order follows the per-process hash seed, so the
//!   fingerprint would not be stable. `HashMap` is fine (object keys are sorted).
//!
//! `validate` also refuses NaN/infinite floats (JSON would alias them to `null`).

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, Path, Token, ext::IdentExt as _, parse_macro_input};

pub(crate) fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

// ── Container-level options ────────────────────────────────────────────────

struct ContainerOptions {
    /// Optional `validate = path` — the path of a free function `fn(&T) -> Result<(), Error>`.
    validate_fn: Option<Path>,
    /// If `true`, the `HasSchema` impl is suppressed — the caller provides their own.
    schema_external: bool,
}

impl ContainerOptions {
    fn parse(attrs: &[syn::Attribute]) -> syn::Result<Self> {
        let mut validate_fn: Option<Path> = None;
        let mut schema_external = false;

        for attr in attrs {
            if !attr.path().is_ident("config") {
                continue;
            }

            // Parse `#[config(key = value, ...)]` or `#[config(flag)]`.
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("validate") {
                    // `validate = path`
                    let _eq: Token![=] = meta.input.parse()?;
                    let path: Path = meta.input.parse()?;
                    if validate_fn.is_some() {
                        return Err(syn::Error::new_spanned(
                            &meta.path,
                            "duplicate `validate` key in #[config(...)]",
                        ));
                    }
                    validate_fn = Some(path);
                    Ok(())
                } else if meta.path.is_ident("schema") {
                    // `schema = external`
                    let _eq: Token![=] = meta.input.parse()?;
                    let val_ident: Ident = meta.input.parse()?;
                    if val_ident != "external" {
                        return Err(syn::Error::new_spanned(
                            &val_ident,
                            "only `schema = external` is supported; \
                             use `#[config(schema = external)]` to suppress the default HasSchema impl",
                        ));
                    }
                    schema_external = true;
                    Ok(())
                } else {
                    Err(syn::Error::new_spanned(
                        &meta.path,
                        format!(
                            "unknown #[config(...)] key `{}`; supported keys: \
                             `validate = path`, `schema = external`",
                            meta.path
                                .get_ident()
                                .map(ToString::to_string)
                                .unwrap_or_else(|| "<path>".to_string()),
                        ),
                    ))
                }
            })?;
        }

        Ok(Self {
            validate_fn,
            schema_external,
        })
    }
}

// ── Field-level options ────────────────────────────────────────────────────

struct FieldOptions {
    skip_fingerprint: bool,
}

impl FieldOptions {
    fn parse(attrs: &[syn::Attribute]) -> syn::Result<Self> {
        let mut skip_fingerprint = false;

        for attr in attrs {
            if !attr.path().is_ident("config") {
                continue;
            }

            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("skip_fingerprint") {
                    skip_fingerprint = true;
                    Ok(())
                } else {
                    Err(syn::Error::new_spanned(
                        &meta.path,
                        format!(
                            "unknown field-level #[config(...)] key `{}`; \
                             supported field key: `skip_fingerprint`",
                            meta.path
                                .get_ident()
                                .map(ToString::to_string)
                                .unwrap_or_else(|| "<path>".to_string()),
                        ),
                    ))
                }
            })?;
        }

        Ok(Self { skip_fingerprint })
    }
}

// ── Main expansion ─────────────────────────────────────────────────────────

fn expand(input: DeriveInput) -> syn::Result<TokenStream2> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Parse container-level options.
    let opts = ContainerOptions::parse(&input.attrs)?;

    // Configs can be unit, named, or positional structs, but never enums/unions.
    let fields = match &input.data {
        Data::Struct(s) => &s.fields,
        Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                struct_name,
                "#[derive(ResourceConfig)] can only be used on structs, not enums",
            ));
        },
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                struct_name,
                "#[derive(ResourceConfig)] can only be used on structs, not unions",
            ));
        },
    };

    // The stable fingerprint of every included named field (or every positional
    // field of a tuple struct, named by index); `None` when no field is included.
    let fingerprinted = fingerprinted_fields(fields)?;

    // No field: all instances are structurally identical, so 0 is correct.
    let fingerprint_body = fingerprinted
        .as_ref()
        .map_or_else(|| quote! { 0 }, |builder| quote! { #builder.finish() });

    // A config whose fields have no stable fingerprint is refused before the
    // caller's own validation runs.
    let fingerprint_check = fingerprinted.as_ref().map(|builder| {
        quote! { #builder.try_finish()?; }
    });
    let validate_impl = match (&opts.validate_fn, fingerprint_check) {
        (None, None) => {
            // No validate override — inherit the trait default (`Ok(())`).
            quote! {}
        },
        (path, check) => {
            let delegate = path.as_ref().map_or_else(
                || quote! { ::core::result::Result::Ok(()) },
                |path| quote! { #path(self) },
            );
            quote! {
                fn validate(&self) -> ::core::result::Result<(), ::nebula_resource::Error> {
                    #check
                    #delegate
                }
            }
        },
    };

    let resource_config_impl = quote! {
        impl #impl_generics ::nebula_resource::ResourceConfig for #struct_name #ty_generics #where_clause {
            fn fingerprint(&self) -> u64 {
                #fingerprint_body
            }
            #validate_impl
        }
    };

    // Unit structs use serde's null wire; empty-braced structs remain records.
    let has_schema_impl = if opts.schema_external {
        quote! {}
    } else {
        let schema = match fields {
            Fields::Unit => quote! { <() as ::nebula_schema::HasSchema>::schema() },
            Fields::Named(named) if named.named.is_empty() => {
                quote! { ::core::result::Result::Ok(::nebula_schema::ValidSchema::empty()) }
            },
            _ => {
                return Err(syn::Error::new_spanned(
                    struct_name,
                    "only unit and empty-braced configs have automatic schemas; add \
                     #[config(schema = external)] and derive Schema or implement HasSchema",
                ));
            },
        };
        quote! {
            impl #impl_generics ::nebula_schema::HasSchema for #struct_name #ty_generics #where_clause {
                fn schema() -> ::core::result::Result<::nebula_schema::ValidSchema, ::nebula_schema::ValidationReport> {
                    #schema
                }
            }
        }
    };

    Ok(quote! {
        #resource_config_impl
        #has_schema_impl
    })
}

// ── Fingerprint body builder ───────────────────────────────────────────────

/// The `ConfigFingerprint` builder expression over every field not marked
/// `#[config(skip_fingerprint)]`, or `None` when no field is included.
///
/// The fingerprint must be stable across builds — the effect journal records
/// it — so fields are encoded through `serde::Serialize` into canonical JSON
/// and digested with SHA-256 by `nebula_resource::ConfigFingerprint`, never
/// through `std::hash::Hash`, whose data is not stable between compiler
/// versions or platforms. A named field is keyed by its identifier, a
/// positional one by its index; the builder sorts them, so declaration
/// order does not matter.
fn fingerprinted_fields(fields: &Fields) -> syn::Result<Option<TokenStream2>> {
    let calls: Vec<TokenStream2> = match fields {
        Fields::Unit => Vec::new(),
        Fields::Named(named) => named
            .named
            .iter()
            .map(|field| {
                let field_opts = FieldOptions::parse(&field.attrs)?;
                if field_opts.skip_fingerprint {
                    return Ok(None);
                }
                // `Fields::Named` guarantees every field has an ident;
                // the `let-else` turns the structural invariant into a
                // compiler error rather than a panic.
                let Some(ident) = field.ident.as_ref() else {
                    return Err(syn::Error::new_spanned(
                        field,
                        "internal: named field missing ident — \
                         report this as a #[derive(ResourceConfig)] bug",
                    ));
                };
                reject_unordered(&field.ty)?;
                let name = ident.unraw().to_string();
                Ok(Some(quote! { .field(#name, &self.#ident) }))
            })
            .filter_map(Result::transpose)
            .collect::<syn::Result<Vec<_>>>()?,
        Fields::Unnamed(unnamed) => unnamed
            .unnamed
            .iter()
            .enumerate()
            .map(|(i, field)| {
                let field_opts = FieldOptions::parse(&field.attrs)?;
                if field_opts.skip_fingerprint {
                    return Ok(None);
                }
                reject_unordered(&field.ty)?;
                let idx = syn::Index::from(i);
                let name = i.to_string();
                Ok(Some(quote! { .field(#name, &self.#idx) }))
            })
            .filter_map(Result::transpose)
            .collect::<syn::Result<Vec<_>>>()?,
    };
    if calls.is_empty() {
        return Ok(None);
    }
    Ok(Some(quote! {
        ::nebula_resource::ConfigFingerprint::new() #(#calls)*
    }))
}

/// Refuses a fingerprinted field whose type names a hash-ordered set
/// anywhere in it (`HashSet`, `hashbrown::HashSet`, `FxHashSet`, …, also
/// inside `Option`/`Vec`/references/tuples/arrays).
///
/// The canonical encoding sorts object keys — so a `HashMap` is fine — but
/// keeps array order, which is content for a `Vec`. A hash set serializes
/// as an array in its iteration order, which follows the per-process hash
/// seed: equal configurations would get different fingerprints, and a
/// journaled effect would be refused after a restart. The check is
/// syntactic: a set hidden behind a type alias or inside another type is
/// the field type's own determinism contract (documented on
/// `ConfigFingerprint`).
fn reject_unordered(ty: &syn::Type) -> syn::Result<()> {
    match ty {
        syn::Type::Path(path) => {
            if let Some(qself) = &path.qself {
                reject_unordered(&qself.ty)?;
            }
            for segment in &path.path.segments {
                if segment.ident.to_string().ends_with("HashSet") {
                    return Err(syn::Error::new_spanned(
                        segment,
                        format!(
                            "`{}` iterates in hash order, which differs between processes, so \
                             the configuration fingerprint (recorded by the effect journal) \
                             would not be stable; use `BTreeSet`, a sorted `Vec`, or mark the \
                             field `#[config(skip_fingerprint)]`",
                            segment.ident
                        ),
                    ));
                }
                if let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments {
                    for argument in &arguments.args {
                        if let syn::GenericArgument::Type(inner) = argument {
                            reject_unordered(inner)?;
                        }
                    }
                }
            }
            Ok(())
        },
        syn::Type::Reference(reference) => reject_unordered(&reference.elem),
        syn::Type::Array(array) => reject_unordered(&array.elem),
        syn::Type::Slice(slice) => reject_unordered(&slice.elem),
        syn::Type::Paren(paren) => reject_unordered(&paren.elem),
        syn::Type::Group(group) => reject_unordered(&group.elem),
        syn::Type::Ptr(pointer) => reject_unordered(&pointer.elem),
        syn::Type::Tuple(tuple) => tuple.elems.iter().try_for_each(reject_unordered),
        _ => Ok(()),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    // Tests for the expansion helpers live in nebula-resource's integration test
    // suite (`tests/resource_config_derive.rs`) where the emitted code can
    // actually be compiled and run.
    //
    // Parsing unit tests for `ContainerOptions` and `FieldOptions` cannot be
    // placed here easily without depending on `syn::parse_str`, which would
    // require proc-macro2 features not available inside a `proc-macro` crate's
    // own `#[cfg(test)]`. They are covered by the integration tests instead.
}
