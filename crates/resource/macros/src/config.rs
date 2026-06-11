//! `#[derive(ResourceConfig)]` macro implementation.
//!
//! Emits:
//! - `impl nebula_resource::ResourceConfig for T` with:
//!   - `fn fingerprint(&self) -> u64` — deterministic structural hash folded over every
//!     field that does NOT carry `#[config(skip_fingerprint)]`. Each included field
//!     must implement `std::hash::Hash`. Uses `DefaultHasher::new()` (SipHash with
//!     fixed seed) — deterministic within a process, which is the only requirement for
//!     hot-reload change-detection.
//!   - `fn validate(&self) -> Result<(), nebula_resource::Error>` — if
//!     `#[config(validate = path)]` is present, delegates to `path(self)`; otherwise
//!     the default `Ok(())` from the trait is inherited (method not emitted, so the
//!     trait default applies).
//! - Optionally `impl nebula_schema::HasSchema for T` returning an empty schema,
//!   UNLESS `#[config(schema = external)]` is specified, in which case no `HasSchema`
//!   impl is emitted (the caller is responsible for `#[derive(Schema)]` or a manual
//!   `impl HasSchema`).
//!
//! ## Container attribute (`#[config(...)]`)
//!
//! Supported keys:
//! - `validate = path` — calls `path(self)` in the emitted `validate` method.
//! - `schema = external` — suppresses the empty-`HasSchema` emission.
//!
//! Unknown keys are rejected with a `compile_error!` at the key span.
//!
//! ## Field attribute (`#[config(...)]`)
//!
//! Supported keys:
//! - `skip_fingerprint` — this field is excluded from the fingerprint hash fold.
//!
//! Unknown field-level keys are rejected with a `compile_error!` at the key span.
//!
//! ## Rejected forms
//!
//! - Enums and unions: compile error at the type ident.
//! - Tuple struct fields with `#[config]`: compile error.
//! - Field included in fingerprint that does not impl `Hash` — compile error from
//!   the emitted `<field as std::hash::Hash>::hash(&self.field, &mut h)` call.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, Path, Token, parse_macro_input};

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

    // Extract named fields (reject enums/unions/unit structs with no fields for fingerprint).
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

    // Build fingerprint hash fold: hash every included named field (or all positional
    // fields for tuple structs). For unit structs: hash nothing → constant 0 by the
    // fold base.
    let fingerprint_body = build_fingerprint_body(fields)?;

    // Build optional `validate` override.
    let validate_impl = if let Some(path) = &opts.validate_fn {
        quote! {
            fn validate(&self) -> ::core::result::Result<(), ::nebula_resource::Error> {
                #path(self)
            }
        }
    } else {
        // No validate override — inherit the trait default (`Ok(())`).
        quote! {}
    };

    let resource_config_impl = quote! {
        impl #impl_generics ::nebula_resource::ResourceConfig for #struct_name #ty_generics #where_clause {
            fn fingerprint(&self) -> u64 {
                #fingerprint_body
            }
            #validate_impl
        }
    };

    // Optionally emit a default empty-schema HasSchema impl.
    let has_schema_impl = if opts.schema_external {
        quote! {}
    } else {
        quote! {
            impl #impl_generics ::nebula_schema::HasSchema for #struct_name #ty_generics #where_clause {
                fn schema() -> ::nebula_schema::ValidSchema {
                    ::nebula_schema::ValidSchema::empty()
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

fn build_fingerprint_body(fields: &Fields) -> syn::Result<TokenStream2> {
    // DefaultHasher::new() uses SipHash with fixed, non-random seed — it IS
    // deterministic within a process and within the same std version. Hot-reload
    // change detection only compares fingerprints within the same running process,
    // so cross-run determinism is not required here.
    match fields {
        Fields::Unit => {
            // Unit struct: no fields → hash nothing → return 0.
            // All instances are structurally identical, so 0 is correct.
            Ok(quote! { 0 })
        },
        Fields::Named(named) => {
            let hash_calls: Vec<TokenStream2> = named
                .named
                .iter()
                .map(|field| {
                    let field_opts = FieldOptions::parse(&field.attrs)?;
                    if field_opts.skip_fingerprint {
                        return Ok(quote! {});
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
                    Ok(quote! {
                        ::std::hash::Hash::hash(&self.#ident, &mut __hasher);
                    })
                })
                .collect::<syn::Result<Vec<_>>>()?;

            let all_skipped = hash_calls.iter().all(TokenStream2::is_empty);
            if all_skipped {
                // All fields skipped — same as unit struct.
                Ok(quote! { 0 })
            } else {
                Ok(quote! {
                    {
                        use ::std::hash::Hasher as _;
                        let mut __hasher = ::std::collections::hash_map::DefaultHasher::new();
                        #(#hash_calls)*
                        __hasher.finish()
                    }
                })
            }
        },
        Fields::Unnamed(unnamed) => {
            // Tuple struct: check that none of the fields carry `#[config(...)]` that
            // would be confusing. We hash them positionally.
            let hash_calls: Vec<TokenStream2> = unnamed
                .unnamed
                .iter()
                .enumerate()
                .map(|(i, field)| {
                    let field_opts = FieldOptions::parse(&field.attrs)?;
                    if field_opts.skip_fingerprint {
                        return Ok(quote! {});
                    }
                    let idx = syn::Index::from(i);
                    Ok(quote! {
                        ::std::hash::Hash::hash(&self.#idx, &mut __hasher);
                    })
                })
                .collect::<syn::Result<Vec<_>>>()?;

            let all_skipped = hash_calls.iter().all(TokenStream2::is_empty);
            if all_skipped {
                Ok(quote! { 0 })
            } else {
                Ok(quote! {
                    {
                        use ::std::hash::Hasher as _;
                        let mut __hasher = ::std::collections::hash_map::DefaultHasher::new();
                        #(#hash_calls)*
                        __hasher.finish()
                    }
                })
            }
        },
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
