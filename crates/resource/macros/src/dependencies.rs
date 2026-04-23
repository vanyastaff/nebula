//! Parsing and codegen for `DeclaresDependencies` via `#[uses_credential]` /
//! `#[uses_resource]` attributes on `#[derive(Resource)]` structs.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Attribute, Ident, Result, Token, Type,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Parse all dependency attributes from a struct and generate the
/// `DeclaresDependencies` impl block plus compile-time assertions.
pub(crate) fn expand(
    struct_name: &Ident,
    generics: &syn::Generics,
    attrs: &[Attribute],
) -> Result<TokenStream2> {
    let mut cred_deps: Vec<DepEntry> = Vec::new();
    let mut res_deps: Vec<DepEntry> = Vec::new();

    for attr in attrs {
        if attr.path().is_ident("uses_credential") {
            cred_deps.push(parse_single_dep(attr)?);
        } else if attr.path().is_ident("uses_resource") {
            res_deps.push(parse_single_dep(attr)?);
        } else if attr.path().is_ident("uses_credentials") {
            cred_deps.extend(parse_bulk_deps(attr)?);
        } else if attr.path().is_ident("uses_resources") {
            res_deps.extend(parse_bulk_deps(attr)?);
        }
    }

    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // --- build credential chain calls ---
    let cred_calls: Vec<TokenStream2> = cred_deps
        .iter()
        .map(|d| {
            let ty = &d.ty;
            let mut chain = quote! {
                .credential(nebula_core::CredentialRequirement::new(
                    <#ty as nebula_core::CredentialLike>::KEY_STR,
                    ::std::any::TypeId::of::<#ty>(),
                    ::std::any::type_name::<#ty>(),
                ))
            };
            // Wrap optional / purpose as builder methods on the *requirement*,
            // but the builder pattern returns `Dependencies`, not
            // `CredentialRequirement`. So we need to apply optional/purpose
            // *before* passing to `.credential()`.
            // Re-structure: build the requirement first, then chain.
            chain = {
                let optional = if d.optional {
                    quote! { .optional() }
                } else {
                    quote! {}
                };
                let purpose = d
                    .purpose
                    .as_ref()
                    .map(|p| {
                        quote! { .purpose(#p) }
                    })
                    .unwrap_or_default();
                quote! {
                    .credential(
                        nebula_core::CredentialRequirement::new(
                            <#ty as nebula_core::CredentialLike>::KEY_STR,
                            ::std::any::TypeId::of::<#ty>(),
                            ::std::any::type_name::<#ty>(),
                        )
                        #optional
                        #purpose
                    )
                }
            };
            chain
        })
        .collect();

    // --- build resource chain calls ---
    let res_calls: Vec<TokenStream2> = res_deps
        .iter()
        .map(|d| {
            let ty = &d.ty;
            let optional = if d.optional {
                quote! { .optional() }
            } else {
                quote! {}
            };
            let purpose = d
                .purpose
                .as_ref()
                .map(|p| {
                    quote! { .purpose(#p) }
                })
                .unwrap_or_default();
            quote! {
                .resource(
                    nebula_core::ResourceRequirement::new(
                        <#ty as nebula_core::ResourceLike>::KEY_STR,
                        ::std::any::TypeId::of::<#ty>(),
                        ::std::any::type_name::<#ty>(),
                    )
                    #optional
                    #purpose
                )
            }
        })
        .collect();

    // --- compile-time trait assertions ---
    let cred_asserts: Vec<TokenStream2> = cred_deps
        .iter()
        .map(|d| {
            let ty = &d.ty;
            quote! { _assert_credential_like::<#ty>(); }
        })
        .collect();

    let res_asserts: Vec<TokenStream2> = res_deps
        .iter()
        .map(|d| {
            let ty = &d.ty;
            quote! { _assert_resource_like::<#ty>(); }
        })
        .collect();

    let assertions = if cred_asserts.is_empty() && res_asserts.is_empty() {
        quote! {}
    } else {
        quote! {
            const _: () = {
                fn _assert_credential_like<T: nebula_core::CredentialLike>() {}
                fn _assert_resource_like<T: nebula_core::ResourceLike>() {}

                fn _check() {
                    #(#cred_asserts)*
                    #(#res_asserts)*
                }
            };
        }
    };

    Ok(quote! {
        impl #impl_generics nebula_core::DeclaresDependencies for #struct_name #ty_generics #where_clause {
            fn dependencies() -> nebula_core::Dependencies {
                nebula_core::Dependencies::new()
                    #(#cred_calls)*
                    #(#res_calls)*
            }
        }

        #assertions
    })
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// A single parsed dependency entry (either credential or resource).
struct DepEntry {
    ty: Type,
    optional: bool,
    purpose: Option<String>,
}

// ---------------------------------------------------------------------------
// Single-form parsing: `#[uses_credential(GithubToken, optional, purpose = "...")]`
// ---------------------------------------------------------------------------

fn parse_single_dep(attr: &Attribute) -> Result<DepEntry> {
    let parsed: SingleDep = attr.parse_args()?;
    Ok(parsed.into_entry())
}

/// Parser for the contents of `#[uses_credential(Type, optional, purpose = "...")]`.
struct SingleDep {
    ty: Type,
    optional: bool,
    purpose: Option<String>,
}

impl SingleDep {
    fn into_entry(self) -> DepEntry {
        DepEntry {
            ty: self.ty,
            optional: self.optional,
            purpose: self.purpose,
        }
    }
}

impl Parse for SingleDep {
    fn parse(input: ParseStream) -> Result<Self> {
        let ty: Type = input.parse()?;
        let mut optional = false;
        let mut purpose = None;

        while input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }

            let ident: Ident = input.parse()?;
            match ident.to_string().as_str() {
                "optional" => optional = true,
                "purpose" => {
                    input.parse::<Token![=]>()?;
                    let lit: syn::LitStr = input.parse()?;
                    purpose = Some(lit.value());
                },
                other => {
                    return Err(syn::Error::new_spanned(
                        &ident,
                        format!(
                            "unknown dependency modifier `{other}`, expected `optional` or `purpose = \"...\"`"
                        ),
                    ));
                },
            }
        }

        Ok(Self {
            ty,
            optional,
            purpose,
        })
    }
}

// ---------------------------------------------------------------------------
// Bulk-form parsing: `#[uses_credentials([GithubToken, SlackToken(optional, purpose = "...")])]`
// ---------------------------------------------------------------------------

fn parse_bulk_deps(attr: &Attribute) -> Result<Vec<DepEntry>> {
    let parsed: BulkDeps = attr.parse_args()?;
    Ok(parsed.entries)
}

struct BulkDeps {
    entries: Vec<DepEntry>,
}

impl Parse for BulkDeps {
    fn parse(input: ParseStream) -> Result<Self> {
        let content;
        syn::bracketed!(content in input);

        let items = Punctuated::<BulkItem, Token![,]>::parse_terminated(&content)?;
        let entries = items.into_iter().map(BulkItem::into_entry).collect();
        Ok(Self { entries })
    }
}

struct BulkItem {
    ty: Type,
    optional: bool,
    purpose: Option<String>,
}

impl BulkItem {
    fn into_entry(self) -> DepEntry {
        DepEntry {
            ty: self.ty,
            optional: self.optional,
            purpose: self.purpose,
        }
    }
}

impl Parse for BulkItem {
    fn parse(input: ParseStream) -> Result<Self> {
        let ty: Type = input.parse()?;
        let mut optional = false;
        let mut purpose = None;

        // Optional modifiers in parens: `SlackToken(optional, purpose = "notifications")`
        if input.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in input);

            let mods = Punctuated::<BulkModifier, Token![,]>::parse_terminated(&content)?;
            for m in mods {
                match m {
                    BulkModifier::Optional => optional = true,
                    BulkModifier::Purpose(p) => purpose = Some(p),
                }
            }
        }

        Ok(Self {
            ty,
            optional,
            purpose,
        })
    }
}

enum BulkModifier {
    Optional,
    Purpose(String),
}

impl Parse for BulkModifier {
    fn parse(input: ParseStream) -> Result<Self> {
        let ident: Ident = input.parse()?;
        match ident.to_string().as_str() {
            "optional" => Ok(Self::Optional),
            "purpose" => {
                input.parse::<Token![=]>()?;
                let lit: syn::LitStr = input.parse()?;
                Ok(Self::Purpose(lit.value()))
            },
            other => Err(syn::Error::new_spanned(
                &ident,
                format!("unknown modifier `{other}`, expected `optional` or `purpose = \"...\"`"),
            )),
        }
    }
}
